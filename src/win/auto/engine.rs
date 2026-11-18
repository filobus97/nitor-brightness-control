//! The automation engine: per-monitor state, planning rounds, the single wake-up timer.

use super::sensor::{self, Subscription, sensor_available};
use super::weather::Weather;
use super::{TIMER_AUTO, TIMER_SENSOR, adaptive, location};
use crate::auto::ambient::{AmbientFilter, HOLD_SECS};
use crate::auto::drive::{
    Context, MonitorState, earliest, level_bounds, location as configured_location, location_due,
    round_coord, step, timer_delay_ms, user_override,
};
use crate::auto::plan::MIN_STEP_PCT;
use crate::auto::weather::Status as WeatherStatus;
use crate::config::{Auto, Config, LocationSource};
use crate::win::util::log;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

/// What the engine needs to know about one controllable monitor.
pub struct MonitorView {
    /// Position in the app's monitor list, echoed in the returned writes.
    pub index: usize,
    pub id: String,
    /// Current brightness (percent).
    pub value: u8,
    /// Calibrated `(link_min, link_max)`.
    pub link: (u8, u8),
}

pub(super) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub struct Engine {
    hwnd: isize,
    states: HashMap<String, MonitorState>,
    /// Global "pause automation" end (memory only).
    pause_until: Option<i64>,
    sensor: Option<Subscription>,
    filter: Option<AmbientFilter>,
    filter_bounds: (f32, f32),
    last_lux: Option<f32>,
    /// Level the sensor mode currently asks for.
    sensor_level: Option<f32>,
    location_try: i64,
    location_busy: bool,
    adaptive_logged: bool,
    weather: Weather,
}

impl Engine {
    pub fn new(hwnd: HWND) -> Engine {
        Engine {
            hwnd: hwnd.0 as isize,
            states: HashMap::new(),
            pause_until: None,
            sensor: None,
            filter: None,
            filter_bounds: (0.0, 0.0),
            last_lux: None,
            sensor_level: None,
            location_try: 0,
            location_busy: false,
            adaptive_logged: false,
            weather: Weather::new(hwnd.0 as isize),
        }
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    /// Whether the global pause is running.
    pub fn paused(&self) -> bool {
        self.pause_until.is_some_and(|t| t > now_unix())
    }

    /// The unix time the global pause ends (`None`: not paused).
    pub fn pause_until(&self) -> Option<i64> {
        self.pause_until.filter(|&t| t > now_unix())
    }

    /// "Pause automation for 1 hour" / "Resume automation".
    pub fn toggle_pause(&mut self) {
        self.pause_until = if self.paused() {
            None
        } else {
            Some(now_unix() + 3600)
        };
        log(&format!("auto pause until {:?}", self.pause_until));
    }

    /// The user changed monitor `id` by hand: hold automation off for it.
    pub fn user_change(&mut self, id: &str, auto: &Auto) {
        if !auto.enabled {
            return;
        }
        let st = self.states.entry(id.to_string()).or_default();
        user_override(st, auto, now_unix());
        log(&format!(
            "auto override id={id} until {:?}",
            st.override_until
        ));
    }

    /// Back from sleep: the panels may have been reset, so forget what was applied.
    /// (The rescan that follows re-reads them and plans again.)
    pub fn resume(&mut self) {
        for st in self.states.values_mut() {
            st.applied = None;
        }
    }

    /// One planning round: bring sensor and location in line with `cfg`, decide for every
    /// monitor, re-arm the timer. Returns `(view.index, value)` writes to apply as automatic.
    pub fn plan(&mut self, cfg: &Config, views: &[MonitorView]) -> Vec<(usize, u8)> {
        let now = now_unix();
        let auto = &cfg.auto;
        self.sync_sensor(auto, now);
        self.sync_location(auto, now);
        self.weather.sync(auto, now);
        if !auto.enabled {
            self.arm(None, now);
            return Vec::new();
        }
        if !self.adaptive_logged {
            self.adaptive_logged = true;
            log(&format!(
                "auto adaptive brightness (Windows): {}",
                adaptive::adaptive_brightness_on()
            ));
        }
        if self.pause_until.is_some_and(|t| t <= now) {
            self.pause_until = None;
        }
        let ctx = Context {
            auto,
            now,
            pause_until: self.pause_until,
            sensor_level: self.sensor_level,
            cloud_cover: self.weather.cover(now),
        };
        let mut writes = Vec::new();
        let mut wake = None;
        for v in views {
            let st = self.states.entry(v.id.clone()).or_default();
            let s = step(st, &ctx, v.link, v.value);
            wake = earliest(wake, s.next_wake);
            if let Some((value, reason)) = s.write {
                log(&format!(
                    "auto write id={} target={value}% reason={}",
                    v.id,
                    reason.as_str()
                ));
                writes.push((v.index, value));
            }
        }
        // A fetch is always due on time; a change of the forecast matters for planning
        // only without the sensor, which measures the real light and takes precedence.
        wake = earliest(wake, self.weather.next_fetch());
        if self.sensor_level.is_none() {
            wake = earliest(wake, self.weather.next_cover_change(now));
        }
        self.arm(wake, now);
        writes
    }

    /// Set (or clear) the single planner timer.
    fn arm(&self, wake: Option<i64>, now: i64) {
        match wake {
            Some(t) => {
                log(&format!("auto next wake in {} s", (t - now).max(1)));
                // SAFETY: plain timer call on the app's own window; replaces the old timer.
                unsafe {
                    SetTimer(
                        Some(self.hwnd()),
                        TIMER_AUTO,
                        timer_delay_ms(t + 1, now),
                        None,
                    );
                }
            }
            None => {
                // SAFETY: plain timer call on the app's own window.
                unsafe {
                    let _ = KillTimer(Some(self.hwnd()), TIMER_AUTO);
                }
            }
        }
    }

    // --- sensor -----------------------------------------------------------

    /// Subscribe to / drop the light sensor according to the configuration.
    fn sync_sensor(&mut self, auto: &Auto, now: i64) {
        if auto.enabled && auto.use_sensor && sensor_available() {
            if self.sensor.is_none()
                && let Some((sub, lux)) = Subscription::start(self.hwnd)
            {
                self.sensor = Some(sub);
                self.filter = None;
                self.last_lux = lux;
            }
            let bounds = level_bounds(auto);
            if self.sensor.is_some() && (self.filter.is_none() || self.filter_bounds != bounds) {
                self.filter = Some(AmbientFilter::new(bounds.0, bounds.1, MIN_STEP_PCT));
                self.filter_bounds = bounds;
                self.sensor_level = None;
                if let Some(lux) = self.last_lux {
                    self.feed(lux, now);
                }
            }
        } else if self.sensor.take().is_some() {
            self.filter = None;
            self.sensor_level = None;
            self.last_lux = None;
            // SAFETY: plain timer call on the app's own window.
            unsafe {
                let _ = KillTimer(Some(self.hwnd()), TIMER_SENSOR);
            }
        }
    }

    /// Feed the filter; whether it produced a new level.
    fn feed(&mut self, lux: f32, now: i64) -> bool {
        let Some(level) = self
            .filter
            .as_mut()
            .and_then(|f| f.feed(f64::from(lux), now))
        else {
            return false;
        };
        self.sensor_level = Some(level);
        true
    }

    /// A reading arrived (`WM_APP_AUTO_SENSOR`); true if the planner must run.
    pub fn on_sensor(&mut self) -> bool {
        let lux = sensor::take_lux();
        self.last_lux = Some(lux);
        let changed = self.feed(lux, now_unix());
        // A held-back jump is only believed after `HOLD_SECS`; the sensor may stay silent
        // meanwhile, so judge once more then.
        // SAFETY: plain timer call on the app's own window.
        unsafe {
            SetTimer(
                Some(self.hwnd()),
                TIMER_SENSOR,
                (HOLD_SECS as u32 + 1) * 1000,
                None,
            );
        }
        changed
    }

    /// The one-shot settle timer fired (`TIMER_SENSOR`); true if the planner must run.
    pub fn on_sensor_settle(&mut self) -> bool {
        // SAFETY: plain timer call on the app's own window.
        unsafe {
            let _ = KillTimer(Some(self.hwnd()), TIMER_SENSOR);
        }
        self.last_lux.is_some_and(|lux| self.feed(lux, now_unix()))
    }

    // --- weather ----------------------------------------------------------

    /// A weather result arrived (`WM_APP_AUTO_WEATHER`); the caller plans again.
    pub fn on_weather(&mut self) {
        self.weather.on_result(now_unix());
    }

    /// What the Settings window shows about the last weather fetch.
    pub fn weather_status(&self) -> WeatherStatus {
        self.weather.status()
    }

    // --- location ---------------------------------------------------------

    /// Ask Windows for the location when enabled and due (at most daily).
    fn sync_location(&mut self, auto: &Auto, now: i64) {
        // Color automation needs the location too.
        if !((auto.enabled || auto.color_enabled)
            && auto.location_source == LocationSource::Windows)
        {
            // So that enabling it later asks right away.
            self.location_try = 0;
            return;
        }
        if self.location_busy
            || !location_due(self.location_try, now, configured_location(auto).is_some())
        {
            return;
        }
        self.location_try = now;
        self.location_busy = true;
        log("auto location: requesting");
        location::request(self.hwnd);
    }

    /// A location result arrived (`WM_APP_AUTO_LOCATION`): the new rounded coordinates, if
    /// the request succeeded and Windows is still the configured source.
    pub fn on_location(&mut self, auto: &Auto) -> Option<(f64, f64)> {
        self.location_busy = false;
        match location::take()? {
            Ok((lat, lon)) if auto.location_source == LocationSource::Windows => {
                let c = (round_coord(lat), round_coord(lon));
                log(&format!("auto location: {:.2}, {:.2}", c.0, c.1));
                Some(c)
            }
            Ok(_) => None,
            Err(e) => {
                log(&format!("auto location: failed: {e}"));
                None
            }
        }
    }
}
