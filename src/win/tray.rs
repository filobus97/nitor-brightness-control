//! `Shell_NotifyIconW` wrappers. Icons are identified by `uID` only, never GUID (spec §10).

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
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
    TrayEvent {
        msg: (lparam.0 as usize & 0xFFFF) as u32,
        uid: ((lparam.0 as usize >> 16) & 0xFFFF) as u32,
        x: (wparam.0 & 0xFFFF) as u16 as i16 as i32,
        y: ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
    }
}

fn data(hwnd: HWND, uid: u32) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: uid,
        ..Default::default()
    }
}

/// Copy `s` into `buf`, truncated to leave room for the NUL.
fn copy_str(buf: &mut [u16], s: &str) {
    let max = buf.len() - 1;
    for (dst, src) in buf.iter_mut().zip(s.encode_utf16().take(max)) {
        *dst = src;
    }
}

fn icon_data(hwnd: HWND, uid: u32, icon: HICON, tip: &str) -> NOTIFYICONDATAW {
    let mut nid = data(hwnd, uid);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_APP_TRAY;
    nid.hIcon = icon;
    copy_str(&mut nid.szTip, tip);
    nid
}

/// Add an icon and set `NOTIFYICON_VERSION_4`. Tooltip is truncated to 127 chars.
pub fn add(hwnd: HWND, uid: u32, icon: HICON, tip: &str) -> bool {
    let mut nid = icon_data(hwnd, uid, icon, tip);
    // SAFETY: `nid` is a fully initialised NOTIFYICONDATAW that outlives the calls.
    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            return false;
        }
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &nid).as_bool()
    }
}

pub fn modify(hwnd: HWND, uid: u32, icon: HICON, tip: &str) -> bool {
    let nid = icon_data(hwnd, uid, icon, tip);
    // SAFETY: `nid` is valid for the call.
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() }
}

pub fn remove(hwnd: HWND, uid: u32) {
    let nid = data(hwnd, uid);
    // SAFETY: `nid` is valid for the call.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// Screen rectangle of the icon (`Shell_NotifyIconGetRect`); `None` if hidden in overflow or absent.
pub fn rect(hwnd: HWND, uid: u32) -> Option<RECT> {
    let id = NOTIFYICONIDENTIFIER {
        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: uid,
        ..Default::default()
    };
    // SAFETY: `id` is valid for the call.
    let r = unsafe { Shell_NotifyIconGetRect(&id) }.ok()?;
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

/// Show a balloon/toast notification attached to the icon.
/// `quiet_time`: let Windows hold it back during quiet time / Focus; pass `false` for the
/// answer to a user's action.
pub fn balloon(hwnd: HWND, uid: u32, title: &str, text: &str, quiet_time: bool) {
    let mut nid = data(hwnd, uid);
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = if quiet_time {
        NIIF_INFO | NIIF_RESPECT_QUIET_TIME
    } else {
        NIIF_INFO
    };
    copy_str(&mut nid.szInfoTitle, title);
    copy_str(&mut nid.szInfo, text);
    // SAFETY: `nid` is valid for the call.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}
