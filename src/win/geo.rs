//! Place search for the Settings window: one request per explicit click, on a worker
//! thread. The result is queued here and the window is told with a posted message.

use super::http::get;
use crate::auto::geocode::{self, Place};
use crate::i18n::Lang;
use std::ffi::c_void;
use std::sync::{Mutex, PoisonError};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Answers are a few KB; refuse anything absurd.
const MAX_BODY: usize = 1 << 18;

/// Finished searches as `(ticket, places)`; `None` places: the request or the answer failed.
static DONE: Mutex<Vec<(u64, Option<Vec<Place>>)>> = Mutex::new(Vec::new());

/// Search for `query` in the background; when done, post `msg` to `hwnd` and make the
/// result available through [`take`] under `ticket`.
pub fn spawn(query: String, lang: Lang, hwnd: HWND, msg: u32, ticket: u64) {
    // HWND is not Send; carry it as an integer.
    let hwnd = hwnd.0 as isize;
    std::thread::spawn(move || {
        let places = get(
            &geocode::search_url(&query, lang),
            Some("application/json"),
            MAX_BODY,
        )
        .ok()
        .and_then(|body| geocode::parse(&String::from_utf8_lossy(&body)));
        DONE.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((ticket, places));
        // SAFETY: posting to a window handle is safe even if the window is gone.
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), msg, WPARAM(0), LPARAM(0));
        }
    });
}

/// The result of search `ticket`, if it has finished (older results are dropped).
pub fn take(ticket: u64) -> Option<Option<Vec<Place>>> {
    let mut done = DONE.lock().unwrap_or_else(PoisonError::into_inner);
    let found = done.iter().position(|(t, _)| *t == ticket)?;
    let result = done.swap_remove(found).1;
    done.retain(|(t, _)| *t > ticket);
    Some(result)
}
