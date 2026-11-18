//! Single instance (named mutex) and argument forwarding via `WM_COPYDATA` (spec §10, §11).
//! The primary's window class is [`WINDOW_CLASS`]; it is a hidden top-level window
//! (not message-only, so it receives the `TaskbarCreated` broadcast and `FindWindowW` finds it).

use std::cell::RefCell;
use std::ffi::c_void;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT,
    WPARAM,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowW, HWND_MESSAGE, RegisterClassW,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COPYDATA, WNDCLASSW,
};
use windows::core::PCWSTR;

use super::util::wide;

pub const WINDOW_CLASS: &str = "NitorTrayWindow";

const MAGIC_ARGS: usize = 0x4E49_5401;
const MAGIC_REPLY: usize = 0x4E49_5402;
const REPLY_CLASS: &str = "NitorReplyWindow";

/// Held by the primary instance; releases the mutex on drop.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: the handle is the mutex created in `acquire`, owned by this thread.
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

pub enum Instance {
    Primary(InstanceGuard),
    /// Another instance runs; its window (may be null if it is still starting up).
    Secondary(HWND),
}

pub fn acquire() -> Instance {
    let name = wide("Local\\Nitor.SingleInstance");
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe {
        match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            Ok(h) if GetLastError() != ERROR_ALREADY_EXISTS => Instance::Primary(InstanceGuard(h)),
            Ok(h) => {
                let _ = CloseHandle(h);
                Instance::Secondary(find_primary())
            }
            // Could not create the mutex at all: behave as primary-less secondary.
            Err(_) => Instance::Secondary(find_primary()),
        }
    }
}

fn find_primary() -> HWND {
    let class = wide(WINDOW_CLASS);
    // SAFETY: `class` is NUL-terminated and outlives the call.
    unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) }.unwrap_or_default()
}

/// A command line forwarded by a secondary instance.
pub struct Forwarded {
    pub args: Vec<String>,
    /// The secondary's reply window (may be null).
    pub reply_to: HWND,
}

thread_local! {
    static REPLY_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

unsafe extern "system" fn reply_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_COPYDATA {
        // SAFETY: for WM_COPYDATA, lparam points to a COPYDATASTRUCT valid during the call.
        if let Some(bytes) = unsafe { payload(lparam, MAGIC_REPLY) } {
            let text = String::from_utf8_lossy(bytes).into_owned();
            REPLY_TEXT.with(|t| t.borrow_mut().push_str(&text));
            return LRESULT(1);
        }
    }
    // SAFETY: default handling of an unhandled message.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The payload bytes of a `COPYDATASTRUCT` with the given magic.
///
/// # Safety
/// `lparam` must be zero or point to a valid `COPYDATASTRUCT` for the lifetime `'a`.
unsafe fn payload<'a>(lparam: LPARAM, magic: usize) -> Option<&'a [u8]> {
    if lparam.0 == 0 {
        return None;
    }
    // SAFETY: guaranteed by the caller.
    let cds = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
    if cds.dwData != magic {
        return None;
    }
    if cds.lpData.is_null() || cds.cbData == 0 {
        return Some(&[]);
    }
    // SAFETY: lpData/cbData describe a readable buffer per the COPYDATASTRUCT contract.
    Some(unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) })
}

fn send_copydata(
    to: HWND,
    from: HWND,
    magic: usize,
    bytes: &[u8],
    timeout_ms: u32,
) -> Option<usize> {
    let cds = COPYDATASTRUCT {
        dwData: magic,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr() as *mut c_void,
    };
    let mut result = 0usize;
    // SAFETY: `cds` and `bytes` outlive this synchronous call.
    let r = unsafe {
        SendMessageTimeoutW(
            to,
            WM_COPYDATA,
            WPARAM(from.0 as usize),
            LPARAM(&cds as *const _ as isize),
            SMTO_ABORTIFHUNG,
            timeout_ms,
            Some(&mut result),
        )
    };
    (r.0 != 0).then_some(result)
}

/// From the secondary: create a hidden reply window, send `args` with `WM_COPYDATA`
/// (synchronous `SendMessageW`), and collect the reply text the primary sends back
/// during that call. Returns `(exit_code, reply_text)`, or `None` if sending failed.
pub fn forward(primary: HWND, args: &[String]) -> Option<(i32, String)> {
    let class = wide(REPLY_CLASS);
    REPLY_TEXT.with(|t| t.borrow_mut().clear());
    // SAFETY: standard class registration and message-only window creation; the class
    // name buffer outlives both calls.
    let hwnd = unsafe {
        let hinst = HINSTANCE(GetModuleHandleW(None).unwrap_or_default().0);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(reply_proc),
            hInstance: hinst,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&wc); // may already exist; creation below reports real failures
        CreateWindowExW(
            Default::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR::null(),
            Default::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst),
            None,
        )
    }
    .ok()?;
    let joined = args.join("\0");
    let result = send_copydata(primary, hwnd, MAGIC_ARGS, joined.as_bytes(), 10_000);
    // SAFETY: `hwnd` was created above on this thread.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    let text = REPLY_TEXT.with(|t| std::mem::take(&mut *t.borrow_mut()));
    result.map(|r| (r as i32, text))
}

/// In the primary's window procedure for `WM_COPYDATA`: decode a forwarded command line.
/// `wparam` carries the sender's reply window.
pub fn decode(wparam: WPARAM, lparam: LPARAM) -> Option<Forwarded> {
    // SAFETY: called from the window procedure with the WM_COPYDATA lparam.
    let bytes = unsafe { payload(lparam, MAGIC_ARGS) }?;
    let text = String::from_utf8_lossy(bytes);
    let args = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\0').map(str::to_owned).collect()
    };
    Some(Forwarded {
        args,
        reply_to: HWND(wparam.0 as _),
    })
}

/// In the primary, while handling the forwarded message: send reply text back.
pub fn reply(primary: HWND, reply_to: HWND, text: &str) {
    if reply_to.0.is_null() {
        return;
    }
    let _ = send_copydata(reply_to, primary, MAGIC_REPLY, text.as_bytes(), 2000);
}
