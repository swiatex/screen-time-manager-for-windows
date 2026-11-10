//! Screen Time Manager - A system tray application for Windows
//!
//! This application runs in the background with only a system tray icon visible.
//! Right-clicking the icon shows a context menu with options including quit.

#![windows_subsystem = "windows"]

mod blocking;
mod constants;
mod database;
mod dialogs;
mod dpi;
mod i18n;
mod logging;
mod mini_overlay;
mod overlay;
mod telegram;
mod tray;

use std::mem::zeroed;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{BOOL, CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_CLASS_ALREADY_EXISTS},
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::CreateMutexW,
        },
        UI::HiDpi::{SetProcessDpiAwareness, PROCESS_PER_MONITOR_DPI_AWARE},
        UI::WindowsAndMessaging::*,
    },
};

use blocking::{create_blocking_overlay, create_secondary_overlays, register_blocking_class, REMAINING_SECONDS};
use constants::MUTEX_NAME;
use database::{init_database, load_remaining_time, get_current_weekday, get_daily_limit};
use mini_overlay::{create_mini_overlay, register_mini_overlay_class, show_mini_overlay};
use overlay::{create_overlay_window, register_overlay_class};
use tray::{add_tray_icon, init_taskbar_created_message, remove_tray_icon, window_proc};
use std::sync::atomic::Ordering;

fn main() {
    unsafe {
        // Record unexpected panics before the process aborts. This turns
        // "the app silently disappears" into a diagnosable event.
        logging::install_panic_hook();
        logging::log(&format!(
            "=== Screen Time Manager v{} starting ===",
            env!("CARGO_PKG_VERSION")
        ));

        // Set DPI awareness before creating any windows
        let _ = SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE);
        dpi::init_dpi();

        // Check for single instance
        if !ensure_single_instance() {
            // Silent exit: at logon both the scheduled task and a Startup
            // shortcut may launch us at the same time. A modal message box
            // in that situation is pointless and confusing.
            logging::log("Another instance is already running; exiting.");
            return;
        }

        // We are the primary instance: a previous intentional stop no longer applies.
        logging::clear_stopped_marker();

        // Initialize database
        if let Err(e) = init_database() {
            logging::log(&format!("Failed to initialize database: {}", e));
            let msg: Vec<u16> = format!("Failed to initialize database: {}\0", e)
                .encode_utf16()
                .collect();
            MessageBoxW(
                None,
                PCWSTR(msg.as_ptr()),
                w!("Database Error"),
                MB_OK | MB_ICONERROR,
            );
            return;
        }

        // Get the module handle
        let hinstance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => {
                logging::log(&format!("Failed to get module handle: {}", e));
                MessageBoxW(
                    None,
                    w!("Failed to initialize Screen Time Manager."),
                    w!("Startup Error"),
                    MB_OK | MB_ICONERROR,
                );
                return;
            }
        };

        // Register main window class
        let class_name = w!("ScreenTimeManagerClass");
        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..zeroed()
        };

        if RegisterClassW(&wnd_class) == 0 {
            let err = GetLastError();
            if err != ERROR_CLASS_ALREADY_EXISTS {
                logging::log(&format!("Failed to register window class: {:?}", err));
                MessageBoxW(
                    None,
                    w!("Failed to initialize Screen Time Manager."),
                    w!("Startup Error"),
                    MB_OK | MB_ICONERROR,
                );
                return;
            }
        }

        // Register overlay and blocking window classes
        register_overlay_class(hinstance);
        register_blocking_class(hinstance);
        register_mini_overlay_class(hinstance);

        // Create a hidden window for message handling
        let hwnd = match CreateWindowExW(
            Default::default(),
            class_name,
            w!("Screen Time Manager"),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            hinstance,
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                logging::log(&format!("Failed to create main window: {}", e));
                MessageBoxW(
                    None,
                    w!("Failed to create the main window."),
                    w!("Startup Error"),
                    MB_OK | MB_ICONERROR,
                );
                return;
            }
        };

        // Create the overlay windows (initially hidden)
        create_overlay_window(hinstance);
        create_blocking_overlay(hinstance);
        create_secondary_overlays(hinstance);  // Create overlays for secondary monitors
        create_mini_overlay(hinstance);

        // Initialize remaining time from database or daily limit
        let remaining = load_remaining_time().unwrap_or_else(|| {
            // No saved time for today, use daily limit
            let weekday = get_current_weekday();
            (get_daily_limit(weekday) * 60) as i32  // Convert minutes to seconds
        });
        REMAINING_SECONDS.store(remaining, Ordering::SeqCst);

        // Initialize session active time from database
        let session_active = database::get_session_active_time();
        mini_overlay::SESSION_ACTIVE_SECONDS.store(session_active, Ordering::SeqCst);

        // Show the mini overlay with remaining time
        show_mini_overlay();

        // If time is already exhausted, show blocking overlay immediately
        if remaining <= 0 {
            let msg = database::get_blocking_message();
            blocking::show_blocking_overlay(&msg);
        }

        // Add the system tray icon. This can legitimately fail very early
        // during logon, before Explorer has created the notification area.
        // add_tray_icon() arms a retry timer instead of aborting the process.
        init_taskbar_created_message();
        add_tray_icon(hwnd);

        // Start Telegram bot in background thread (if configured)
        telegram::start_bot_thread();

        // Message loop
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        logging::log("Message loop ended; shutting down.");

        // Cleanup: remove the tray icon
        remove_tray_icon();
    }
}

/// Ensures only one instance of the application is running
unsafe fn ensure_single_instance() -> bool {
    let mutex_name: Vec<u16> = MUTEX_NAME.encode_utf16().chain(std::iter::once(0)).collect();

    let handle = CreateMutexW(
        None,
        BOOL::from(true),
        PCWSTR(mutex_name.as_ptr()),
    );

    match handle {
        Ok(h) => {
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(h);
                false
            } else {
                true
            }
        }
        Err(_) => false,
    }
}
