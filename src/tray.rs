//! System tray module for Screen Time Manager
//! Handles the system tray icon and context menu

use std::mem::zeroed;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
                NOTIFYICONDATAW,
            },
            WindowsAndMessaging::*,
        },
    },
};

use crate::blocking::{extend_time, hide_blocking_overlay, show_blocking_overlay, BLOCKING_HWND};
use crate::constants::*;
use crate::database::{get_blocking_message, get_warning_config, is_pause_enabled};
use crate::dialogs::{show_settings_dialog, show_stats_dialog, verify_passcode_for_quit};
use crate::i18n;
use crate::logging;
use crate::mini_overlay::{is_paused, is_idle_paused, can_pause, toggle_pause, PauseBlockedReason, get_remaining_pause_budget};
use crate::overlay::{show_overlay, OVERLAY_HWND};
use crate::telegram;
use std::sync::atomic::{AtomicU32, Ordering};

/// Global state for the notification icon data
pub static mut NOTIFY_ICON_DATA: Option<NOTIFYICONDATAW> = None;

/// Timer used to retry adding the tray icon until Explorer's notification
/// area is ready (this is usually only a problem during logon).
pub const TRAY_RETRY_TIMER_ID: usize = 20;
const TRAY_RETRY_INTERVAL_MS: u32 = 1500;

/// `TaskbarCreated` is broadcast by Explorer when it (re)starts, meaning all
/// tray icons have to be registered again.
pub static TASKBAR_CREATED_MESSAGE: AtomicU32 = AtomicU32::new(0);

/// Look up the `TaskbarCreated` broadcast message. Must be called before the
/// message loop starts.
pub unsafe fn init_taskbar_created_message() {
    let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
    TASKBAR_CREATED_MESSAGE.store(msg, Ordering::SeqCst);
}

/// Add the system tray icon.
///
/// Returns `false` and arms a retry timer when the shell is not ready yet
/// (typical during logon). Never panics: a missing tray icon must not take
/// the whole application down.
pub unsafe fn add_tray_icon(hwnd: HWND) -> bool {
    // Drop a stale registration first. NIM_ADD fails if the same icon ID is
    // already registered (e.g. after an Explorer restart notification), so a
    // clean delete-before-add makes the retry loop reliable.
    if let Some(old) = NOTIFY_ICON_DATA.as_ref() {
        let _ = Shell_NotifyIconW(NIM_DELETE, old);
        NOTIFY_ICON_DATA = None;
    }

    let hinstance = match GetModuleHandleW(None) {
        Ok(h) => h,
        Err(e) => {
            logging::log(&format!("Tray: GetModuleHandleW failed: {}", e));
            return false;
        }
    };

    let hicon = match LoadIconW(hinstance, PCWSTR(1 as *const u16))
        .or_else(|_| LoadIconW(None, IDI_APPLICATION))
    {
        Ok(icon) => icon,
        Err(e) => {
            logging::log(&format!("Tray: failed to load icon: {}", e));
            return false;
        }
    };

    let tooltip = i18n::t("tray.tooltip");
    let mut tip_buffer: [u16; 128] = [0; 128];
    for (i, c) in tooltip.encode_utf16().enumerate() {
        if i >= 127 { break; }
        tip_buffer[i] = c;
    }

    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.hIcon = hicon;
    nid.szTip = tip_buffer;

    if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
        logging::log("Tray: Shell_NotifyIconW(NIM_ADD) failed (shell not ready); retrying in 1.5s");
        let _ = SetTimer(hwnd, TRAY_RETRY_TIMER_ID, TRAY_RETRY_INTERVAL_MS, None);
        return false;
    }

    NOTIFY_ICON_DATA = Some(nid);
    logging::log("Tray: icon added");
    true
}

/// Remove the system tray icon
pub unsafe fn remove_tray_icon() {
    if let Some(ref nid) = NOTIFY_ICON_DATA {
        let _ = Shell_NotifyIconW(NIM_DELETE, nid);
        NOTIFY_ICON_DATA = None;
    }
}

/// Show the context menu when right-clicking the tray icon
pub unsafe fn show_context_menu(hwnd: HWND) {
    let hmenu = match CreatePopupMenu() {
        Ok(menu) => menu,
        Err(e) => {
            logging::log(&format!("Tray: failed to create popup menu: {}", e));
            return;
        }
    };

    // Determine pause menu item text and state
    let paused = is_paused();
    let pause_enabled = is_pause_enabled();

    let (pause_text, pause_flags) = if paused {
        // Currently manually paused - show resume option (always available)
        (i18n::t("tray.resume"), MF_BYPOSITION | MF_STRING)
    } else if is_idle_paused() {
        // Currently idle-paused - grey out manual pause (already paused via idle)
        (i18n::t("tray.pause_idle"), MF_BYPOSITION | MF_STRING | MF_GRAYED)
    } else if !pause_enabled {
        // Pause feature disabled
        (i18n::t("tray.pause_disabled"), MF_BYPOSITION | MF_STRING | MF_GRAYED)
    } else {
        // Check if pause is available
        match can_pause() {
            Ok(()) => {
                let budget_mins = get_remaining_pause_budget() / 60;
                let text = format!("Pause Timer ({}m left)", budget_mins);
                // Need to leak the string for the menu (will be cleaned up with menu)
                let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let ptr = wide.as_ptr();
                std::mem::forget(wide);
                return show_context_menu_with_pause(hwnd, hmenu, PCWSTR(ptr), MF_BYPOSITION | MF_STRING);
            }
            Err(PauseBlockedReason::BudgetExhausted) => {
                (i18n::t("tray.pause_budget_used"), MF_BYPOSITION | MF_STRING | MF_GRAYED)
            }
            Err(PauseBlockedReason::CooldownActive { seconds_remaining }) => {
                let mins = (seconds_remaining + 59) / 60; // Round up
                let text = format!("Pause ({}m cooldown)", mins);
                let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let ptr = wide.as_ptr();
                std::mem::forget(wide);
                return show_context_menu_with_pause(hwnd, hmenu, PCWSTR(ptr), MF_BYPOSITION | MF_STRING | MF_GRAYED);
            }
            Err(PauseBlockedReason::MinActiveTimeNotMet { seconds_remaining }) => {
                let mins = (seconds_remaining + 59) / 60;
                let text = format!("Pause (wait {}m)", mins);
                let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let ptr = wide.as_ptr();
                std::mem::forget(wide);
                return show_context_menu_with_pause(hwnd, hmenu, PCWSTR(ptr), MF_BYPOSITION | MF_STRING | MF_GRAYED);
            }
            Err(PauseBlockedReason::TimeTooLow) => {
                (i18n::t("tray.pause_time_low"), MF_BYPOSITION | MF_STRING | MF_GRAYED)
            }
            Err(PauseBlockedReason::Disabled) => {
                (i18n::t("tray.pause_disabled"), MF_BYPOSITION | MF_STRING | MF_GRAYED)
            }
        }
    };

    let pause_wide: Vec<u16> = pause_text.encode_utf16().chain(std::iter::once(0)).collect();
    show_context_menu_with_pause(hwnd, hmenu, PCWSTR(pause_wide.as_ptr()), pause_flags);
}

/// Insert a menu item, logging (but ignoring) failures.
unsafe fn insert_menu(hmenu: HMENU, position: u32, flags: MENU_ITEM_FLAGS, id: usize, text: PCWSTR) {
    if let Err(e) = InsertMenuW(hmenu, position, flags, id, text) {
        logging::log(&format!("Tray: InsertMenuW failed at position {}: {}", position, e));
    }
}

/// Helper to show context menu with pause item
unsafe fn show_context_menu_with_pause(hwnd: HWND, hmenu: HMENU, pause_text: PCWSTR, pause_flags: MENU_ITEM_FLAGS) {
    let stats_text = i18n::wide("tray.stats");
    insert_menu(hmenu, 0, MF_BYPOSITION | MF_STRING, IDM_TODAYS_STATS as usize, PCWSTR(stats_text.as_ptr()));
    let settings_text = i18n::wide("tray.settings");
    insert_menu(hmenu, 1, MF_BYPOSITION | MF_STRING, IDM_SETTINGS as usize, PCWSTR(settings_text.as_ptr()));
    insert_menu(hmenu, 2, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
    let extend15_text = i18n::wide("tray.extend_15");
    insert_menu(hmenu, 3, MF_BYPOSITION | MF_STRING, IDM_EXTEND_15 as usize, PCWSTR(extend15_text.as_ptr()));
    let extend45_text = i18n::wide("tray.extend_45");
    insert_menu(hmenu, 4, MF_BYPOSITION | MF_STRING, IDM_EXTEND_45 as usize, PCWSTR(extend45_text.as_ptr()));
    insert_menu(hmenu, 5, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());

    // Pause menu item with dynamic text
    insert_menu(hmenu, 6, pause_flags, IDM_PAUSE_TOGGLE as usize, pause_text);

    let mut idx = 7;

    // Show idle status if idle-paused
    if is_idle_paused() {
        let idle_text = i18n::wide("tray.idle_paused");
        insert_menu(hmenu, idx, MF_BYPOSITION | MF_STRING | MF_GRAYED, 0, PCWSTR(idle_text.as_ptr()));
        idx += 1;
    }

    insert_menu(hmenu, idx, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
    idx += 1;
    let warning_text = i18n::wide("tray.show_warning");
    insert_menu(hmenu, idx, MF_BYPOSITION | MF_STRING, IDM_SHOW_OVERLAY as usize, PCWSTR(warning_text.as_ptr()));
    idx += 1;
    let blocking_text = i18n::wide("tray.show_blocking");
    insert_menu(hmenu, idx, MF_BYPOSITION | MF_STRING, IDM_SHOW_BLOCKING as usize, PCWSTR(blocking_text.as_ptr()));
    idx += 1;
    insert_menu(hmenu, idx, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
    idx += 1;
    let about_text = i18n::wide("tray.about");
    insert_menu(hmenu, idx, MF_BYPOSITION | MF_STRING, IDM_ABOUT as usize, PCWSTR(about_text.as_ptr()));
    idx += 1;
    let quit_text = i18n::wide("tray.quit");
    insert_menu(hmenu, idx, MF_BYPOSITION | MF_STRING, IDM_QUIT as usize, PCWSTR(quit_text.as_ptr()));

    let mut point = zeroed();
    if GetCursorPos(&mut point).is_err() {
        // Fall back to the position of the message that opened the menu.
        let pos = GetMessagePos();
        point.x = (pos & 0xFFFF) as i16 as i32;
        point.y = ((pos >> 16) & 0xFFFF) as i16 as i32;
    }

    let _ = SetForegroundWindow(hwnd);

    let _ = TrackPopupMenu(
        hmenu,
        TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
        point.x,
        point.y,
        0,
        hwnd,
        None,
    );

    DestroyMenu(hmenu).ok();
}

/// Main window procedure for handling tray events
pub unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Explorer (re)started: every tray icon has to be registered again.
    let taskbar_created = TASKBAR_CREATED_MESSAGE.load(Ordering::SeqCst);
    if taskbar_created != 0 && msg == taskbar_created {
        logging::log("Tray: Explorer restarted (TaskbarCreated); re-adding icon");
        remove_tray_icon();
        add_tray_icon(hwnd);
        return LRESULT(0);
    }

    match msg {
        WM_TIMER => {
            if wparam.0 == TRAY_RETRY_TIMER_ID {
                let _ = KillTimer(hwnd, TRAY_RETRY_TIMER_ID);
                add_tray_icon(hwnd);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_TRAYICON => {
            let event = lparam.0 as u32;
            match event {
                WM_RBUTTONUP | WM_LBUTTONUP => {
                    show_context_menu(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let menu_id = (wparam.0 & 0xFFFF) as u16;
            match menu_id {
                IDM_PAUSE_TOGGLE => {
                    // Toggle pause state (no passcode required - it's a child feature)
                    match toggle_pause() {
                        Ok(_is_now_paused) => {
                            // Success - UI will update automatically
                        }
                        Err(_reason) => {
                            // Should not happen since menu item should be grayed out
                            // But just in case, do nothing
                        }
                    }
                }
                IDM_SHOW_OVERLAY => {
                    let (minutes, message) = get_warning_config(1);
                    show_overlay(&message, minutes);
                }
                IDM_SHOW_BLOCKING => {
                    let message = get_blocking_message();
                    show_blocking_overlay(&message);
                }
                IDM_TODAYS_STATS => {
                    if verify_passcode_for_quit(hwnd) {
                        show_stats_dialog(hwnd);
                    }
                }
                IDM_SETTINGS => {
                    if verify_passcode_for_quit(hwnd) {
                        show_settings_dialog(hwnd);
                    }
                }
                IDM_EXTEND_15 => {
                    if verify_passcode_for_quit(hwnd) {
                        extend_time(15);
                    }
                }
                IDM_EXTEND_45 => {
                    if verify_passcode_for_quit(hwnd) {
                        extend_time(45);
                    }
                }
                IDM_ABOUT => {
                    let about_msg = i18n::wide("about.text");
                    let about_title = i18n::wide("window.about");
                    MessageBoxW(
                        hwnd,
                        PCWSTR(about_msg.as_ptr()),
                        PCWSTR(about_title.as_ptr()),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
                IDM_QUIT => {
                    if verify_passcode_for_quit(hwnd) {
                        // Tell the watchdog not to resurrect us after an
                        // intentional (passcode-protected) quit.
                        logging::mark_stopped();
                        DestroyWindow(hwnd).ok();
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // Signal Telegram bot to shut down (sends shutdown notification)
            telegram::signal_shutdown();

            let overlay_hwnd = HWND(OVERLAY_HWND.load(Ordering::SeqCst));
            if !overlay_hwnd.0.is_null() {
                DestroyWindow(overlay_hwnd).ok();
            }
            let blocking_hwnd = HWND(BLOCKING_HWND.load(Ordering::SeqCst));
            if !blocking_hwnd.0.is_null() {
                hide_blocking_overlay();
                DestroyWindow(blocking_hwnd).ok();
            }
            remove_tray_icon();
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
