//! Mouse wheel over tray icons via a temporary `WH_MOUSE_LL` hook (spec §9).
//! The hook is installed on, and only called on, the UI thread.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// Posted to the app window for a wheel event over a Nitor icon. Decode with [`decode`].
pub const WM_APP_WHEEL: u32 = WM_APP + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelEvent {
    pub uid: u32,
    /// Raw wheel delta (positive = away from user), usually ±120 per notch.
    pub delta: i32,
    pub shift: bool,
}

pub fn decode(wparam: WPARAM, lparam: LPARAM) -> WheelEvent {
    let _ = (wparam, lparam);
    todo!()
}

/// Install the hook (if not installed) and replace the cached icon rectangles.
/// Wheel events inside a rectangle are consumed and posted to `hwnd`; all others pass through.
pub fn arm(hwnd: HWND, rects: &[(u32, RECT)]) {
    let _ = (hwnd, rects);
    todo!()
}

/// Remove the hook.
pub fn disarm() {
    todo!()
}

pub fn is_armed() -> bool {
    todo!()
}

/// Whether `pt` lies in any cached icon rectangle.
pub fn contains(pt: POINT) -> bool {
    let _ = pt;
    todo!()
}
