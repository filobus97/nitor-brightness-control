//! `Shell_NotifyIconW` wrappers. Icons are identified by `uID` only, never GUID (spec §10).

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{HICON, WM_APP};

/// Callback message for all Nitor tray icons.
pub const WM_APP_TRAY: u32 = WM_APP + 1;

/// A decoded `NOTIFYICON_VERSION_4` callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayEvent {
    /// `WM_MOUSEMOVE`, `NIN_SELECT`, `NIN_KEYSELECT`, `WM_CONTEXTMENU`, `WM_MBUTTONUP`, …
    pub msg: u32,
    pub uid: u32,
    /// Anchor point in screen coordinates.
    pub x: i32,
    pub y: i32,
}

pub fn decode(wparam: WPARAM, lparam: LPARAM) -> TrayEvent {
    let _ = (wparam, lparam);
    todo!()
}

/// Add an icon and set `NOTIFYICON_VERSION_4`. Tooltip is truncated to 127 chars.
pub fn add(hwnd: HWND, uid: u32, icon: HICON, tip: &str) -> bool {
    let _ = (hwnd, uid, icon, tip);
    todo!()
}

pub fn modify(hwnd: HWND, uid: u32, icon: HICON, tip: &str) -> bool {
    let _ = (hwnd, uid, icon, tip);
    todo!()
}

pub fn remove(hwnd: HWND, uid: u32) {
    let _ = (hwnd, uid);
    todo!()
}

/// Screen rectangle of the icon (`Shell_NotifyIconGetRect`); `None` if hidden in overflow or absent.
pub fn rect(hwnd: HWND, uid: u32) -> Option<RECT> {
    let _ = (hwnd, uid);
    todo!()
}

/// Show a balloon/toast notification attached to the icon.
pub fn balloon(hwnd: HWND, uid: u32, title: &str, text: &str) {
    let _ = (hwnd, uid, title, text);
    todo!()
}
