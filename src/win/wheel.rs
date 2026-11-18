//! Mouse wheel over tray icons via a temporary `WH_MOUSE_LL` hook (spec §9).
//! The hook is installed on, and only called on, the UI thread.

use std::cell::RefCell;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::PtInRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_APP, WM_MOUSEWHEEL,
};

/// Posted to the app window for a wheel event over a Nitor icon. Decode with [`decode`].
pub const WM_APP_WHEEL: u32 = WM_APP + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelEvent {
    pub uid: u32,
    /// Raw wheel delta (positive = away from user), usually ±120 per notch.
    pub delta: i32,
    pub shift: bool,
}

fn pack(delta: i32, shift: bool, uid: u32) -> (WPARAM, LPARAM) {
    let w = (delta as i16 as u16 as usize) | (usize::from(shift) << 16);
    (WPARAM(w), LPARAM(uid as isize))
}

pub fn decode(wparam: WPARAM, lparam: LPARAM) -> WheelEvent {
    WheelEvent {
        uid: lparam.0 as u32,
        delta: (wparam.0 & 0xFFFF) as u16 as i16 as i32,
        shift: (wparam.0 >> 16) & 1 != 0,
    }
}

#[derive(Default)]
struct State {
    hook: Option<HHOOK>,
    hwnd: HWND,
    rects: Vec<(u32, RECT)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn hit(pt: POINT) -> Option<(HWND, u32)> {
    STATE.with(|s| {
        let s = s.borrow();
        s.rects
            .iter()
            // SAFETY: both pointers refer to live values.
            .find(|(_, r)| unsafe { PtInRect(r, pt) }.as_bool())
            .map(|&(uid, _)| (s.hwnd, uid))
    })
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEWHEEL {
        // SAFETY: for WH_MOUSE_LL with HC_ACTION, lparam points to an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if let Some((hwnd, uid)) = hit(info.pt) {
            let delta = (info.mouseData >> 16) as i16 as i32;
            // SAFETY: plain state query.
            let shift = unsafe { GetAsyncKeyState(i32::from(VK_SHIFT.0)) } < 0;
            let (w, l) = pack(delta, shift, uid);
            // SAFETY: posting to a window handle is safe even if it is gone.
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_WHEEL, w, l);
            }
            return LRESULT(1);
        }
    }
    // SAFETY: forwarding the unmodified hook arguments.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Install the hook (if not installed) and replace the cached icon rectangles.
/// Wheel events inside a rectangle are consumed and posted to `hwnd`; all others pass through.
pub fn arm(hwnd: HWND, rects: &[(u32, RECT)]) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.hwnd = hwnd;
        s.rects = rects.to_vec();
        if s.hook.is_none() {
            // SAFETY: hook_proc has the HOOKPROC signature; this module handle outlives the hook.
            let hmod = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
            s.hook = unsafe {
                SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), Some(HINSTANCE(hmod.0)), 0)
            }
            .ok();
        }
    });
}

/// Remove the hook.
pub fn disarm() {
    let hook = STATE.with(|s| s.borrow_mut().hook.take());
    if let Some(h) = hook {
        // SAFETY: `h` was returned by SetWindowsHookExW on this thread.
        unsafe {
            let _ = UnhookWindowsHookEx(h);
        }
    }
}

pub fn is_armed() -> bool {
    STATE.with(|s| s.borrow().hook.is_some())
}

/// Whether `pt` lies in any cached icon rectangle.
pub fn contains(pt: POINT) -> bool {
    hit(pt).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trip() {
        for (d, sh, uid) in [
            (120, false, 3),
            (-120, true, 0),
            (-1, false, 7),
            (32767, true, 9),
        ] {
            let (w, l) = pack(d, sh, uid);
            assert_eq!(
                decode(w, l),
                WheelEvent {
                    uid,
                    delta: d,
                    shift: sh
                }
            );
        }
    }
}
