//! Nitor Plus automation, Windows side. The decisions are made by the pure `crate::auto`
//! core; this module owns the clock, timers and sensors and applies the results.
//!
//! Idle-first: no polling. The only recurring wake-up is one `SetTimer` (`TIMER_AUTO`) set
//! to the planner's next wake time and re-armed after each wake (killed when nothing is
//! scheduled). Everything else is event driven: the ambient-light sensor (WinRT event),
//! resume/display-on/rescan (existing system notifications), settings changes,
//! `WM_TIMECHANGE`, one background thread per location request and one per (rare) weather fetch.
//! Colour temperature (`color`) follows the same planner logic on its own one-shot `TIMER_COLOR`.

mod adaptive;
mod color;
mod engine;
mod location;
mod sensor;
mod weather;

pub use adaptive::adaptive_brightness_on;
pub use color::{
    ColorEngine, ColorView, gamma_range_extended, night_light_active, request_warm_range,
};
pub use engine::{Engine, MonitorView};
pub use sensor::sensor_available;

use windows::Win32::UI::WindowsAndMessaging::WM_APP;

/// Timer: run the planner (the earliest next wake across monitors).
pub const TIMER_AUTO: usize = 5;
/// Timer: one-shot, lets a held-back sensor reading be re-judged after its hold time.
pub const TIMER_SENSOR: usize = 6;
/// Posted by the sensor thread when a new reading waits.
pub const WM_APP_AUTO_SENSOR: u32 = WM_APP + 8;
/// Posted by a location thread when its result waits.
pub const WM_APP_AUTO_LOCATION: u32 = WM_APP + 9;
/// Posted by the weather thread when its result waits.
pub const WM_APP_AUTO_WEATHER: u32 = WM_APP + 10;
/// Timer: the colour planner's next wake.
pub const TIMER_COLOR: usize = 10;
/// Posted when the administrator prompt for the extended gamma range is over.
pub const WM_APP_AUTO_COLOR: u32 = WM_APP + 14;
