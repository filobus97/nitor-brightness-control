//! Opt-in weather: cloud cover from Open-Meteo, fetched on a background thread.
//!
//! Network only while automation and weather are on and a location is set, and only the
//! coordinates rounded to 0.1 degree are sent. No polling: a fetch is due when `plan` runs
//! and `next_try` has passed; `next_wake` hands that time to the engine's single timer.

use crate::auto::weather::{self, Forecast, Status};
use crate::config::Auto;
use crate::win::http;
use crate::win::util::log;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Answers are about 2 KB; refuse anything absurd.
const MAX_BODY: usize = 1 << 16;

/// Coordinates in tenths of a degree: what a request is for.
type Place = (i32, i32);

/// The finished request: its place, and the forecast or the error.
static RESULT: Mutex<Option<(Place, Result<Forecast, String>)>> = Mutex::new(None);

pub struct Weather {
    hwnd: isize,
    /// Last good forecast (memory only); kept across failed fetches until it goes stale.
    forecast: Option<Forecast>,
    /// Place being served while weather is wanted.
    place: Option<Place>,
    /// Earliest unix time of the next fetch.
    next_try: i64,
    busy: bool,
    status: Status,
}

fn tenths(auto: &Auto) -> Option<Place> {
    let t = |v: f64| (v * 10.0).round() as i32;
    Some((t(auto.latitude?), t(auto.longitude?)))
}

impl Weather {
    pub fn new(hwnd: isize) -> Weather {
        Weather {
            hwnd,
            forecast: None,
            place: None,
            next_try: 0,
            busy: false,
            status: Status::None,
        }
    }

    pub fn status(&self) -> Status {
        self.status.clone()
    }

    /// Bucketed cloud cover to plan with at `now`, if a fresh forecast covers it.
    pub fn cover(&self, now: i64) -> Option<u8> {
        self.forecast.as_ref()?.planning_cover(now)
    }

    /// When the next fetch is due, if weather is wanted and none is running.
    pub fn next_fetch(&self) -> Option<i64> {
        (self.place.is_some() && !self.busy).then_some(self.next_try)
    }

    /// When the bucketed cloud cover (or its validity) next changes.
    pub fn next_cover_change(&self, now: i64) -> Option<i64> {
        self.forecast.as_ref()?.next_change(now)
    }

    /// Bring the fetching in line with the configuration; starts a fetch when due.
    pub fn sync(&mut self, auto: &Auto, now: i64) {
        let wanted = auto.enabled && auto.weather;
        let place = if wanted { tenths(auto) } else { None };
        if place != self.place {
            // Switched off, or somewhere else: the old forecast does not apply.
            self.place = place;
            self.forecast = None;
            self.status = Status::None;
            self.next_try = 0;
        }
        let Some(place) = self.place else { return };
        if self.busy || now < self.next_try {
            return;
        }
        self.busy = true;
        let hwnd = self.hwnd;
        let url = weather::forecast_url(f64::from(place.0) / 10.0, f64::from(place.1) / 10.0);
        let spawned = std::thread::Builder::new()
            .name("nitor-weather".into())
            .spawn(move || {
                let fetched = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                let r = http::get(&url, Some("application/json"), MAX_BODY).and_then(|body| {
                    weather::parse(&String::from_utf8_lossy(&body), fetched)
                        .ok_or_else(|| "unexpected answer".to_string())
                });
                *RESULT.lock().unwrap_or_else(PoisonError::into_inner) = Some((place, r));
                // SAFETY: posting to a window handle is safe even if the window is gone.
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        super::WM_APP_AUTO_WEATHER,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            });
        if let Err(e) = spawned {
            self.busy = false;
            self.next_try = weather::next_fetch(now, false);
            self.status = Status::Failed(e.to_string());
            log(&format!("weather: thread failed: {e}"));
        }
    }

    /// `WM_APP_AUTO_WEATHER`: take the result of the request.
    pub fn on_result(&mut self, now: i64) {
        self.busy = false;
        let Some((place, result)) = RESULT.lock().unwrap_or_else(PoisonError::into_inner).take()
        else {
            return;
        };
        if Some(place) != self.place {
            // Answer for a place no longer configured: ask again for the current one.
            self.next_try = 0;
            return;
        }
        match result {
            Ok(f) => {
                let cover = f.cover_at(now);
                let shown = cover.map_or_else(|| "n/a".to_string(), |c| format!("{c}%"));
                log(&format!("weather: ok, cover now {shown}"));
                self.status = Status::Ok {
                    at: local_hm(),
                    cover: cover.unwrap_or(0),
                };
                self.forecast = Some(f);
                self.next_try = weather::next_fetch(now, true);
            }
            Err(e) => {
                log(&format!("weather: err: {e}"));
                self.status = Status::Failed(e);
                self.next_try = weather::next_fetch(now, false);
            }
        }
    }
}

/// Local `(hour, minute)` now.
fn local_hm() -> (u8, u8) {
    // SAFETY: plain call without arguments.
    let t = unsafe { GetLocalTime() };
    (t.wHour as u8, t.wMinute as u8)
}
