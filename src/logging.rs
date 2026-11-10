//! Diagnostics helpers: persistent logging, panic reporting and the
//! "stopped by user" marker used by the watchdog.
//!
//! Logs live next to the database in `%LOCALAPPDATA%\.screen-time-manager\`.
//! They are intentionally tiny and appended only on lifecycle events or
//! when something goes wrong.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

const MAX_LOG_BYTES: u64 = 512 * 1024;

/// Directory holding the app database and diagnostics files.
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join(".screen-time-manager"))
}

pub fn log_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("app.log"))
}

fn timestamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;

    let st = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}

/// Append a single timestamped line to the log file. Never panics.
pub fn log(message: &str) {
    let Some(path) = log_path() else { return };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Keep the log from growing without bound.
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = std::fs::write(&path, "");
        }
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "[{}] {}", timestamp(), message);
    }
}

/// Install a panic hook that records the panic message and source location
/// before the process aborts (the release profile uses `panic = "abort"`).
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };

        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>");

        log(&format!(
            "PANIC in thread '{}' at {}: {}",
            thread_name, location, payload
        ));

        // Preserve the normal stderr output as well.
        default_hook(info);
    }));
}

/// Path of the marker written when the parent quits the app with the passcode.
pub fn stopped_marker_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("stopped"))
}

/// Tell the watchdog that the app was stopped intentionally.
pub fn mark_stopped() {
    if let Some(path) = stopped_marker_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, timestamp());
        log("Stopped by user (passcode); watchdog will not restart until next launch.");
    }
}

/// Remove the intentional-stop marker. Called only by the primary instance
/// once it owns the single-instance mutex, so duplicate launches cannot
/// re-enable the watchdog after the parent quit the app.
pub fn clear_stopped_marker() {
    if let Some(path) = stopped_marker_path() {
        let _ = std::fs::remove_file(path);
    }
}
