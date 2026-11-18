//! Per-monitor automation step (pure): what to write now and when to look again.
//!
//! The Windows engine collects the inputs (config, clock, sensor level, the monitor's
//! current value and link range), calls [`step`] for every monitor and applies the writes.

use super::curve::{DEFAULT_CLOUD_STRENGTH, DayCurve};
use super::plan::{MAX_SLEEP_SECS, MIN_STEP_PCT, PlanInput, override_expiry, plan, should_write};
use crate::config::{Auto, OverrideMode};
use crate::link::link_value;

/// A location is refreshed at most this often (seconds) once known.
pub const LOCATION_REFRESH_SECS: i64 = 24 * 3600;
/// After a failed attempt (no coordinates yet) retry no sooner than this.
pub const LOCATION_RETRY_SECS: i64 = 10 * 60;

/// What produced a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Solar,
    Sensor,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Solar => "solar",
            Reason::Sensor => "sensor",
        }
    }
}

/// Automation memory of one monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MonitorState {
    /// Master level (percent) the automation last settled on.
    pub applied: Option<f32>,
    /// A manual change holds automation off until this unix time.
    pub override_until: Option<i64>,
}

/// Inputs shared by all monitors in one planning round.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    pub auto: &'a Auto,
    pub now: i64,
    /// Global "pause automation" end.
    pub pause_until: Option<i64>,
    /// Level from the ambient light sensor; replaces the solar curve while `Some`.
    pub sensor_level: Option<f32>,
    /// Bucketed cloud cover (percent) from the weather forecast, when enabled and fresh;
    /// lowers the solar day level. Ignored while a sensor level is present.
    pub cloud_cover: Option<u8>,
}

/// Result of one [`step`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Monitor value to write now, with the reason.
    pub write: Option<(u8, Reason)>,
    /// Unix time this monitor needs attention again; `None`: only events can change it.
    pub next_wake: Option<i64>,
}

/// The configured location, if both coordinates are set.
pub fn location(auto: &Auto) -> Option<(f64, f64)> {
    Some((auto.latitude?, auto.longitude?))
}

/// The day/night brightness curve of the configuration.
pub fn curve(auto: &Auto) -> DayCurve {
    DayCurve::new(
        f32::from(auto.day_brightness),
        f32::from(auto.night_brightness),
    )
}

/// Bounds `(min, max)` of the master level (sensor output is clamped to them).
pub fn level_bounds(auto: &Auto) -> (f32, f32) {
    let (d, n) = (
        f32::from(auto.day_brightness),
        f32::from(auto.night_brightness),
    );
    (d.min(n), d.max(n))
}

/// Record a manual change at `now`: forget the applied level and hold automation off
/// for the configured time.
pub fn user_override(st: &mut MonitorState, auto: &Auto, now: i64) {
    st.applied = None;
    st.override_until = Some(override_until(auto, now));
}

/// When a manual change made at `now` stops holding automation off.
pub fn override_until(auto: &Auto, now: i64) -> i64 {
    match location(auto) {
        Some((lat, lon)) => override_expiry(
            auto.override_mode,
            auto.override_minutes,
            now,
            lat,
            lon,
            &curve(auto),
        ),
        None if auto.override_mode == OverrideMode::Minutes => {
            now + i64::from(auto.override_minutes) * 60
        }
        None => now + MAX_SLEEP_SECS,
    }
}

/// Decide for one monitor. `link` is its calibrated `(min, max)`, `current` its value now.
pub fn step(st: &mut MonitorState, ctx: &Context, link: (u8, u8), current: u8) -> Step {
    let now = ctx.now;
    let held = [st.override_until, ctx.pause_until]
        .into_iter()
        .flatten()
        .filter(|&t| t > now)
        .max();
    if let Some(until) = held {
        return Step {
            write: None,
            next_wake: Some(until),
        };
    }
    st.override_until = None;

    let (target, reason, next_wake) = if let Some(level) = ctx.sensor_level {
        let (lo, hi) = level_bounds(ctx.auto);
        let level = level.clamp(lo, hi);
        let due = st.applied.is_none_or(|a| (a - level).abs() >= MIN_STEP_PCT);
        (due.then_some(level), Reason::Sensor, None)
    } else if let Some((lat, lon)) = location(ctx.auto) {
        let base = curve(ctx.auto);
        let p = plan(&PlanInput {
            lat,
            lon,
            curve: ctx.cloud_cover.map_or(base, |c| {
                base.with_clouds(f32::from(c), DEFAULT_CLOUD_STRENGTH)
            }),
            applied: st.applied,
            min_delta: MIN_STEP_PCT,
            override_until: None,
            pause_until: None,
            now,
        });
        (p.target, Reason::Solar, Some(p.next_wake_unix))
    } else {
        (None, Reason::Solar, None)
    };

    let write = target.and_then(|level| {
        st.applied = Some(level);
        let master = level.round().clamp(0.0, 100.0) as u8;
        let value = link_value(master, link.0, link.1);
        should_write(current, value).then_some((value, reason))
    });
    Step { write, next_wake }
}

/// Smaller of two optional wake times.
pub fn earliest(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Timer delay in milliseconds for a wake at `wake` (at least 1 s, within `SetTimer`'s range).
pub fn timer_delay_ms(wake: i64, now: i64) -> u32 {
    (wake - now).clamp(1, i64::from(i32::MAX) / 1000) as u32 * 1000
}

/// Whether a (new) location request is due.
pub fn location_due(last_try: i64, now: i64, have_coordinates: bool) -> bool {
    let wait = if have_coordinates {
        LOCATION_REFRESH_SECS
    } else {
        LOCATION_RETRY_SECS
    };
    now - last_try >= wait
}

/// Coordinates rounded to 2 decimals (about 1 km, plenty for sunrise times).
pub fn round_coord(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2024-06-21 00:00 UTC.
    const DAY0: i64 = 1_718_928_000;

    fn auto() -> Auto {
        Auto {
            enabled: true,
            latitude: Some(41.9),
            longitude: Some(12.5),
            ..Auto::default()
        }
    }

    fn ctx(a: &Auto, now: i64) -> Context<'_> {
        Context {
            auto: a,
            now,
            pause_until: None,
            sensor_level: None,
            cloud_cover: None,
        }
    }

    #[test]
    fn solar_writes_then_settles() {
        let a = auto();
        let mut st = MonitorState::default();
        let noon = DAY0 + 10 * 3600;
        let s = step(&mut st, &ctx(&a, noon), (0, 100), 50);
        assert_eq!(s.write, Some((80, Reason::Solar)));
        assert!(s.next_wake.is_some_and(|t| t > noon));
        // Already there: no further write.
        let s = step(&mut st, &ctx(&a, noon + 60), (0, 100), 80);
        assert_eq!(s.write, None);
    }

    #[test]
    fn small_delta_is_not_written_but_settles() {
        let a = auto();
        let mut st = MonitorState::default();
        let s = step(&mut st, &ctx(&a, DAY0 + 10 * 3600), (0, 100), 79);
        assert_eq!(s.write, None);
        assert_eq!(st.applied, Some(80.0));
    }

    #[test]
    fn link_range_maps_master() {
        let a = auto();
        let mut st = MonitorState::default();
        let s = step(&mut st, &ctx(&a, DAY0 + 10 * 3600), (20, 60), 0);
        assert_eq!(s.write, Some((52, Reason::Solar))); // 20 + 40 * 0.8
    }

    #[test]
    fn override_and_pause_hold_back() {
        let a = auto();
        let now = DAY0 + 10 * 3600;
        let mut st = MonitorState::default();
        user_override(&mut st, &a, now);
        let until = st.override_until.unwrap();
        assert!(until > now);
        let s = step(&mut st, &ctx(&a, now + 5), (0, 100), 10);
        assert_eq!(
            s,
            Step {
                write: None,
                next_wake: Some(until)
            }
        );
        // After it ends the plan resumes and re-applies.
        let s = step(&mut st, &ctx(&a, until + 1), (0, 100), 10);
        assert!(s.write.is_some());
        assert_eq!(st.override_until, None);
        // Global pause.
        let mut c = ctx(&a, now);
        c.pause_until = Some(now + 3600);
        let mut st = MonitorState::default();
        assert_eq!(step(&mut st, &c, (0, 100), 10).next_wake, Some(now + 3600));
    }

    #[test]
    fn override_minutes_and_no_location() {
        let mut a = auto();
        a.override_mode = OverrideMode::Minutes;
        a.override_minutes = 30;
        let mut st = MonitorState::default();
        user_override(&mut st, &a, 1000);
        assert_eq!(st.override_until, Some(1000 + 1800));
        a.latitude = None;
        user_override(&mut st, &a, 1000);
        assert_eq!(st.override_until, Some(1000 + 1800));
        a.override_mode = OverrideMode::UntilTransition;
        user_override(&mut st, &a, 1000);
        assert_eq!(st.override_until, Some(1000 + MAX_SLEEP_SECS));
    }

    #[test]
    fn sensor_replaces_solar_and_is_clamped() {
        let a = auto();
        let now = DAY0 + 10 * 3600;
        let mut c = ctx(&a, now);
        c.sensor_level = Some(10.0); // below night (30)
        let mut st = MonitorState::default();
        let s = step(&mut st, &c, (0, 100), 80);
        assert_eq!(s.write, Some((30, Reason::Sensor)));
        assert_eq!(s.next_wake, None);
        c.sensor_level = Some(31.0); // within a step of applied
        assert_eq!(step(&mut st, &c, (0, 100), 30).write, None);
        c.sensor_level = Some(95.0); // above day (80)
        assert_eq!(
            step(&mut st, &c, (0, 100), 30).write,
            Some((80, Reason::Sensor))
        );
    }

    #[test]
    fn clouds_lower_the_day_level_but_not_the_sensor() {
        let a = auto();
        let noon = DAY0 + 10 * 3600;
        let mut c = ctx(&a, noon);
        c.cloud_cover = Some(100);
        let mut st = MonitorState::default();
        let s = step(&mut st, &c, (0, 100), 50);
        assert_eq!(s.write, Some((56, Reason::Solar))); // 80 * 0.7
        // Sensor mode wins: the sensor already measures the real light.
        c.sensor_level = Some(70.0);
        let mut st = MonitorState::default();
        assert_eq!(
            step(&mut st, &c, (0, 100), 50).write,
            Some((70, Reason::Sensor))
        );
    }

    #[test]
    fn no_location_no_sensor_is_idle() {
        let mut a = auto();
        a.latitude = None;
        let mut st = MonitorState::default();
        let s = step(&mut st, &ctx(&a, DAY0), (0, 100), 10);
        assert_eq!(
            s,
            Step {
                write: None,
                next_wake: None
            }
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(earliest(Some(5), Some(3)), Some(3));
        assert_eq!(earliest(None, Some(3)), Some(3));
        assert_eq!(earliest(Some(4), None), Some(4));
        assert_eq!(earliest(None, None), None);
        assert_eq!(timer_delay_ms(100, 100), 1000);
        assert_eq!(timer_delay_ms(160, 100), 60_000);
        assert!(timer_delay_ms(i64::MAX / 2, 0) <= i32::MAX as u32);
        assert!(location_due(0, LOCATION_RETRY_SECS, false));
        assert!(!location_due(0, LOCATION_REFRESH_SECS - 1, true));
        assert!(location_due(1000, 1000 + LOCATION_REFRESH_SECS, true));
        assert!(!location_due(1000, 1000 + LOCATION_RETRY_SECS - 1, false));
        assert_eq!(round_coord(41.90277), 41.9);
        assert_eq!(round_coord(-12.4964), -12.5);
    }
}
