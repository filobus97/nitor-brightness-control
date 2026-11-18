//! Sun position from the NOAA solar calculator equations (Meeus-based), pure math.
//!
//! Accuracy is well under a degree for the years 1900-2100, which is far more
//! than a brightness schedule needs.

const DEG: f64 = std::f64::consts::PI / 180.0;
/// Coarse scan step used by [`next_crossing`].
const STEP_SECS: i64 = 600;
/// Search horizon of [`next_crossing`]: a bit over two days.
const HORIZON_SECS: i64 = 49 * 3600;

/// Geometric elevation of the sun's centre above the horizon, in degrees
/// (no atmospheric refraction; use -0.833 for "official" sunrise/sunset).
/// `lat_deg` is positive north, `lon_deg` positive east, time is unix seconds (UTC).
pub fn sun_elevation(lat_deg: f64, lon_deg: f64, unix_secs: i64) -> f64 {
    let jd = unix_secs as f64 / 86400.0 + 2_440_587.5;
    let t = (jd - 2_451_545.0) / 36525.0;

    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let (ms, m2s, m3s) = (
        (m * DEG).sin(),
        (2.0 * m * DEG).sin(),
        (3.0 * m * DEG).sin(),
    );
    let center = ms * (1.914602 - t * (0.004817 + 0.000014 * t))
        + m2s * (0.019993 - 0.000101 * t)
        + m3s * 0.000289;
    let omega = (125.04 - 1934.136 * t) * DEG;
    let app_long = l0 + center - 0.00569 - 0.00478 * omega.sin();
    let mean_obliq =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let obliq = (mean_obliq + 0.00256 * omega.cos()) * DEG;
    let decl = (obliq.sin() * (app_long * DEG).sin()).asin();

    let y = (obliq / 2.0).tan().powi(2);
    let l0r = l0 * DEG;
    let eq_time = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * ms + 4.0 * e * y * ms * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * m2s)
            .to_degrees(); // minutes

    let minutes = unix_secs.rem_euclid(86400) as f64 / 60.0;
    let solar_minutes = (minutes + eq_time + 4.0 * lon_deg).rem_euclid(1440.0);
    let hour_angle = (solar_minutes / 4.0 - 180.0) * DEG;

    let lat = lat_deg * DEG;
    let cos_zenith = lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos();
    90.0 - cos_zenith.clamp(-1.0, 1.0).acos().to_degrees()
}

/// First time after `from_unix` (within ~48 h) at which the sun's elevation crosses
/// `elevation_deg` going up (`rising`) or down. Coarse 10-minute scan plus bisection
/// to one second. `None` when it does not cross in the horizon (polar day/night);
/// a crossing that stays within a 10-minute grazing window may be missed.
pub fn next_crossing(
    lat: f64,
    lon: f64,
    from_unix: i64,
    elevation_deg: f64,
    rising: bool,
) -> Option<i64> {
    // `hit` is true on the far side of the crossing (above when rising, below when falling).
    let hit = |t: i64| {
        let above = sun_elevation(lat, lon, t) >= elevation_deg;
        above == rising
    };
    let mut prev = from_unix;
    if hit(prev) {
        // Already past the crossing: wait for the opposite side first.
        let mut t = prev;
        while hit(t) {
            t += STEP_SECS;
            if t > from_unix + HORIZON_SECS {
                return None;
            }
        }
        prev = t;
    }
    let mut t = prev;
    while t < from_unix + HORIZON_SECS {
        let next = t + STEP_SECS;
        if hit(next) {
            let (mut lo, mut hi) = (t, next);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if hit(mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(hi);
        }
        t = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds of a UTC date and time (days-from-civil algorithm).
    pub(crate) fn utc(y: i64, m: i64, d: i64, hh: i64, mm: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        ((era * 146_097 + doe - 719_468) * 24 + hh) * 60 * 60 + mm * 60
    }

    fn near(actual: Option<i64>, expected: i64) {
        let a = actual.expect("crossing expected");
        assert!(
            (a - expected).abs() <= 180,
            "off by {} s (got {a}, want {expected})",
            a - expected
        );
    }

    const SUNRISE: f64 = -0.833;

    #[test]
    fn greenwich_equinox() {
        let (lat, lon) = (51.4779, 0.0);
        let day = utc(2024, 3, 20, 0, 0);
        near(
            next_crossing(lat, lon, day, SUNRISE, true),
            day + 6 * 3600 + 4 * 60,
        );
        near(
            next_crossing(lat, lon, day, SUNRISE, false),
            day + 18 * 3600 + 15 * 60,
        );
        let noon = sun_elevation(lat, lon, day + 12 * 3600 + 7 * 60 + 30);
        assert!((noon - 38.7).abs() < 0.3, "{noon}");
    }

    #[test]
    fn rome_summer_solstice() {
        let (lat, lon) = (41.9028, 12.4964);
        let day = utc(2024, 6, 21, 0, 0);
        near(
            next_crossing(lat, lon, day, SUNRISE, true),
            day + 3 * 3600 + 35 * 60,
        );
        near(
            next_crossing(lat, lon, day, SUNRISE, false),
            day + 18 * 3600 + 46 * 60,
        );
    }

    #[test]
    fn oslo_winter_solstice() {
        let (lat, lon) = (59.9139, 10.7522);
        let day = utc(2024, 12, 21, 0, 0);
        near(
            next_crossing(lat, lon, day, SUNRISE, true),
            day + 8 * 3600 + 18 * 60,
        );
        near(
            next_crossing(lat, lon, day, SUNRISE, false),
            day + 14 * 3600 + 12 * 60,
        );
    }

    #[test]
    fn tromso_polar_night_and_midnight_sun() {
        let (lat, lon) = (69.6492, 18.9553);
        let dec = utc(2024, 12, 21, 0, 0);
        assert_eq!(next_crossing(lat, lon, dec, SUNRISE, true), None);
        let jun = utc(2024, 6, 21, 0, 0);
        assert_eq!(next_crossing(lat, lon, jun, SUNRISE, false), None);
    }

    #[test]
    fn crossing_is_after_start_and_direction_respected() {
        let (lat, lon) = (41.9028, 12.4964);
        // Start at midday: next rising crossing is tomorrow morning, next falling is tonight.
        let noon = utc(2024, 6, 21, 11, 0);
        let up = next_crossing(lat, lon, noon, 0.0, true).unwrap();
        let down = next_crossing(lat, lon, noon, 0.0, false).unwrap();
        assert!(down > noon && down < up);
        assert!(sun_elevation(lat, lon, up + 60) > 0.0);
        assert!(sun_elevation(lat, lon, down + 60) < 0.0);
    }
}
