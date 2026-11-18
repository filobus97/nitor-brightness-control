//! Windows location (opt-in): one WinRT `Geolocator` position per request, on a thread.

use crate::win::util::log;
use std::sync::Mutex;
use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator};
use windows::Foundation::TimeSpan;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// 100 ns ticks per second (`TimeSpan`).
const TICKS: i64 = 10_000_000;
/// A cached position up to this old is fine.
const MAX_AGE_SECS: i64 = 3600;
/// Give up waiting for a fix after this long.
const TIMEOUT_SECS: i64 = 60;

static RESULT: Mutex<Option<Result<(f64, f64), String>>> = Mutex::new(None);

/// Start a request; the result is stored and `WM_APP_AUTO_LOCATION` posted to `hwnd`.
/// Never blocks the caller.
pub fn request(hwnd: isize) {
    let spawned = std::thread::Builder::new()
        .name("nitor-location".into())
        .spawn(move || {
            let r = fetch();
            *RESULT.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            // SAFETY: posting to a window handle is safe even if the window is gone.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut _)),
                    super::WM_APP_AUTO_LOCATION,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
    if let Err(e) = spawned {
        log(&format!("auto location: thread failed: {e}"));
    }
}

/// The finished request's result, once.
pub fn take() -> Option<Result<(f64, f64), String>> {
    RESULT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn fetch() -> Result<(f64, f64), String> {
    let e = |e: windows::core::Error| e.message();
    let access = Geolocator::RequestAccessAsync()
        .and_then(|op| op.join())
        .map_err(e)?;
    if access == GeolocationAccessStatus::Denied {
        return Err("access denied in Windows privacy settings".into());
    }
    let position = Geolocator::new()
        .and_then(|g| {
            g.GetGeopositionAsyncWithAgeAndTimeout(
                TimeSpan {
                    Duration: MAX_AGE_SECS * TICKS,
                },
                TimeSpan {
                    Duration: TIMEOUT_SECS * TICKS,
                },
            )
        })
        .and_then(|op| op.join())
        .map_err(e)?;
    let c = position.Coordinate().map_err(e)?;
    Ok((c.Latitude().map_err(e)?, c.Longitude().map_err(e)?))
}
