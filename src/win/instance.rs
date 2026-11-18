//! Single instance (named mutex) and argument forwarding via `WM_COPYDATA` (spec §10, §11).
//! The primary's window class is [`WINDOW_CLASS`]; it is a hidden top-level window
//! (not message-only, so it receives the `TaskbarCreated` broadcast and `FindWindowW` finds it).

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};

pub const WINDOW_CLASS: &str = "NitorTrayWindow";

/// Held by the primary instance; releases the mutex on drop.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        todo!()
    }
}

pub enum Instance {
    Primary(InstanceGuard),
    /// Another instance runs; its window (may be null if it is still starting up).
    Secondary(HWND),
}

pub fn acquire() -> Instance {
    todo!()
}

/// A command line forwarded by a secondary instance.
pub struct Forwarded {
    pub args: Vec<String>,
    /// The secondary's reply window (may be null).
    pub reply_to: HWND,
}

/// From the secondary: create a hidden reply window, send `args` with `WM_COPYDATA`
/// (synchronous `SendMessageW`), and collect the reply text the primary sends back
/// during that call. Returns `(exit_code, reply_text)`, or `None` if sending failed.
pub fn forward(primary: HWND, args: &[String]) -> Option<(i32, String)> {
    let _ = (primary, args);
    todo!()
}

/// In the primary's window procedure for `WM_COPYDATA`: decode a forwarded command line.
pub fn decode(lparam: LPARAM) -> Option<Forwarded> {
    let _ = lparam;
    todo!()
}

/// In the primary, while handling the forwarded message: send reply text back.
pub fn reply(primary: HWND, reply_to: HWND, text: &str) {
    let _ = (primary, reply_to, text);
    todo!()
}
