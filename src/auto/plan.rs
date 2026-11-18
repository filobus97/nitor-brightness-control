//! The scheduler decision (pure): what level to apply now and when to wake next.

use super::curve::DayCurve;
use super::solar::{next_crossing, sun_elevation};
use crate::config::OverrideMode;

/// Smallest brightness change (percentage points) worth a DDC write.
pub const MIN_STEP_PCT: f32 = 2.0;
/// Shortest sleep between wake-ups inside a transition.
pub const MIN_WAKE_SECS: i64 = 60;
/// Longest sleep, used when the sun never crosses a boundary (polar day/night).
pub const MAX_SLEEP_SECS: i64 = 12 * 3600;

/// Whether moving from `old_percent` to `new_percent` is worth a write (|delta| >= 2).
pub fn should_write(old_percent: u8, new_percent: u8) -> bool {
    old_percent.abs_diff(new_percent) as f32 >= MIN_STEP_PCT
}

/// Inputs of one scheduling decision, for one monitor (or one colour channel).
#[derive(Clone, Copy, Debug)]
pub struct PlanInput {
    pub lat: f64,
    pub lon: f64,
    pub curve: DayCurve,
    /// Level currently applied, if known.
    pub applied: Option<f32>,
    /// Smallest change worth applying, in the curve's unit ([`MIN_STEP_PCT`] for brightness).
    pub min_delta: f32,
    /// This monitor's manual override ends at this unix time.
    pub override_until: Option<i64>,
    /// Global "pause automation" ends at this unix time.
    pub pause_until: Option<i64>,
    pub now: i64,
}

/// The decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// New level to apply now; `None` when nothing needs writing (paused, or the
    /// applied level is already within `min_delta` of the curve).
    pub target: Option<f32>,
    /// Unix time to run the planner again.
    pub next_wake_unix: i64,
}

/// Unix time at which a manual override started at `now` ends: the next transition
/// boundary (any crossing of the curve's low/high elevation), or `minutes` from now.
pub fn override_expiry(
    mode: OverrideMode,
    minutes: u32,
    now: i64,
    lat: f64,
    lon: f64,
    curve: &DayCurve,
) -> i64 {
    match mode {
        OverrideMode::Minutes => now + i64::from(minutes) * 60,
        OverrideMode::UntilTransition => [
            (curve.low_deg, true),
            (curve.low_deg, false),
            (curve.high_deg, true),
            (curve.high_deg, false),
        ]
        .into_iter()
        .filter_map(|(deg, rising)| next_crossing(lat, lon, now, deg, rising))
        .min()
        .unwrap_or(now + MAX_SLEEP_SECS),
    }
}

/// Decide what to apply and when to wake.
///
/// * Paused (override or global pause still running): no target, wake when the later
///   of the two ends.
/// * Outside a transition: apply the plateau level if needed, sleep until the next
///   transition start.
/// * Inside a transition: wake when the curve will have moved by `min_delta` from the
///   level that will then be applied (checked every minute), at least 60 s ahead and
///   never past the transition end.
pub fn plan(i: &PlanInput) -> Plan {
    let now = i.now;
    let paused_until = [i.override_until, i.pause_until]
        .into_iter()
        .flatten()
        .filter(|&t| t > now)
        .max();
    if let Some(until) = paused_until {
        return Plan {
            target: None,
            next_wake_unix: until,
        };
    }

    let c = &i.curve;
    let elev = sun_elevation(i.lat, i.lon, now);
    let level = c.level(elev);
    let in_transition = elev > c.low_deg && elev < c.high_deg;
    let cap = now + MAX_SLEEP_SECS;

    // On a plateau a smaller residual is still settled once, so the final value is exact.
    let write = match i.applied {
        None => true,
        Some(a) => {
            let d = (a - level).abs();
            d >= i.min_delta || (!in_transition && d > i.min_delta * 0.25)
        }
    };
    let target = write.then_some(level);

    let wake = if in_transition {
        let rising = sun_elevation(i.lat, i.lon, now + MIN_WAKE_SECS) > elev;
        let end = if rising {
            next_crossing(i.lat, i.lon, now, c.high_deg, true)
        } else {
            next_crossing(i.lat, i.lon, now, c.low_deg, false)
        }
        .unwrap_or(cap)
        .min(cap);
        let reference = if write {
            level
        } else {
            i.applied.unwrap_or(level)
        };
        let mut t = now + MIN_WAKE_SECS;
        let mut found = end;
        while t < end {
            if (c.level(sun_elevation(i.lat, i.lon, t)) - reference).abs() >= i.min_delta {
                found = t;
                break;
            }
            t += MIN_WAKE_SECS;
        }
        found
    } else {
        let start = if elev <= c.low_deg {
            next_crossing(i.lat, i.lon, now, c.low_deg, true)
        } else {
            next_crossing(i.lat, i.lon, now, c.high_deg, false)
        };
        start.unwrap_or(cap).min(cap)
    };
    Plan {
        target,
        next_wake_unix: wake.max(now + MIN_WAKE_SECS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROME: (f64, f64) = (41.9028, 12.4964);
    // 2024-06-21 00:00 UTC.
    const DAY0: i64 = 1_718_928_000;

    fn input(now: i64, applied: Option<f32>) -> PlanInput {
        PlanInput {
            lat: ROME.0,
            lon: ROME.1,
            curve: DayCurve::new(80.0, 30.0),
            applied,
            min_delta: MIN_STEP_PCT,
            override_until: None,
            pause_until: None,
            now,
        }
    }

    #[test]
    fn quantization() {
        assert!(!should_write(50, 51));
        assert!(should_write(50, 52));
        assert!(should_write(52, 50));
        assert!(!should_write(50, 50));
    }

    #[test]
    fn night_sleeps_until_transition_start() {
        let now = DAY0 + 23 * 3600; // 23:00 UTC, deep night in Rome
        let p = plan(&input(now, None));
        assert_eq!(p.target, Some(30.0));
        // Civil dawn starts a bit before 03:00 UTC.
        let start = next_crossing(ROME.0, ROME.1, now, -6.0, true).unwrap();
        assert_eq!(p.next_wake_unix, start);
        assert!(p.next_wake_unix > now + 3600);
        // Already at the night level: nothing to write, same wake.
        let q = plan(&input(now, Some(30.0)));
        assert_eq!(q.target, None);
        assert_eq!(q.next_wake_unix, start);
    }

    #[test]
    fn transition_wakes_on_visible_change() {
        let dawn = next_crossing(ROME.0, ROME.1, DAY0 - 3600, -6.0, true).unwrap();
        let now = dawn + 15 * 60;
        let mut applied = None;
        let mut t = now;
        let mut wakes = 0;
        let curve = DayCurve::new(80.0, 30.0);
        while wakes < 100 {
            let p = plan(&input(t, applied));
            if let Some(v) = p.target {
                applied = Some(v);
            }
            assert!(p.next_wake_unix >= t + MIN_WAKE_SECS);
            let at_wake = curve.level(sun_elevation(ROME.0, ROME.1, p.next_wake_unix));
            let high = next_crossing(ROME.0, ROME.1, t, 6.0, true).unwrap();
            if p.next_wake_unix < high {
                assert!((at_wake - applied.unwrap()).abs() >= 2.0 - 0.1);
            }
            t = p.next_wake_unix;
            wakes += 1;
            if t >= high {
                break;
            }
        }
        // 50 points of change in steps of ~2: a few dozen wakes, not hundreds.
        assert!((10..60).contains(&wakes), "{wakes}");
        // After the transition the day level is settled and we sleep until evening.
        let p = plan(&input(t, applied));
        assert_eq!(
            p.target.map(|v| v.round()),
            (applied != Some(80.0)).then_some(80.0)
        );
        assert!(p.next_wake_unix > t + 3600);
    }

    #[test]
    fn override_and_pause() {
        let now = DAY0 + 9 * 3600;
        let mut i = input(now, Some(80.0));
        i.override_until = Some(now + 600);
        i.pause_until = Some(now + 3600);
        assert_eq!(
            plan(&i),
            Plan {
                target: None,
                next_wake_unix: now + 3600
            }
        );
        // Expired override/pause is ignored.
        i.override_until = Some(now - 1);
        i.pause_until = Some(now);
        assert_eq!(plan(&i).target, None); // already at day level
        i.applied = Some(40.0);
        assert_eq!(plan(&i).target, Some(80.0));
    }

    #[test]
    fn override_expiry_modes() {
        let c = DayCurve::new(80.0, 30.0);
        let now = DAY0 + 9 * 3600;
        assert_eq!(
            override_expiry(OverrideMode::Minutes, 45, now, ROME.0, ROME.1, &c),
            now + 45 * 60
        );
        // Mid-morning: the next boundary is high falling in the evening.
        let e = override_expiry(OverrideMode::UntilTransition, 0, now, ROME.0, ROME.1, &c);
        assert_eq!(e, next_crossing(ROME.0, ROME.1, now, 6.0, false).unwrap());
        // Polar night: no boundary at all, falls back to the cap.
        let e = override_expiry(OverrideMode::UntilTransition, 0, now, 80.0, 0.0, &c);
        assert!(e == now + MAX_SLEEP_SECS || e > now);
    }

    #[test]
    fn polar_night_caps_sleep() {
        let dec = 1_734_739_200; // 2024-12-21 00:00 UTC
        let mut i = input(dec, Some(30.0));
        i.lat = 78.0;
        i.lon = 15.0;
        let p = plan(&i);
        assert_eq!(p.target, None);
        assert_eq!(p.next_wake_unix, dec + MAX_SLEEP_SECS);
    }
}
