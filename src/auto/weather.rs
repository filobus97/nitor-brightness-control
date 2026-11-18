//! Cloud cover from the Open-Meteo forecast (pure: URL, parsing, lookups, cadence; the
//! HTTP request and its timing live in `win::auto::weather`).

use crate::json::{Json, Parser, field};

/// A good forecast is refreshed no sooner than this after the last fetch.
pub const REFRESH_SECS: i64 = 3 * 3600;
/// After a failed fetch, retry no sooner than this.
pub const RETRY_SECS: i64 = 30 * 60;
/// A forecast is not used for planning later than this after it was fetched.
pub const MAX_AGE_SECS: i64 = 36 * 3600;
/// Cover is planned in steps of this many percentage points.
pub const BUCKET_PCT: u8 = 10;

const HOUR: i64 = 3600;

/// The request URL: coordinates rounded to 1 decimal (about 10 km) are all that is sent.
pub fn forecast_url(lat: f64, lon: f64) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.1}&longitude={lon:.1}&hourly=cloud_cover&forecast_days=2&timezone=GMT"
    )
}

/// Hourly cloud cover (percent) from `start` (unix, on the hour) on, and when it was fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forecast {
    pub fetched: i64,
    start: i64,
    cover: Vec<u8>,
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// Unix time of an Open-Meteo `YYYY-MM-DDTHH:MM` stamp (GMT).
fn parse_time(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (h, min) = time.split_once(':')?;
    let (h, min) = (h.parse::<i64>().ok()?, min.parse::<i64>().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&day) && h < 24 && min < 60)
        .then(|| days_from_civil(y, m, day) * 86400 + h * HOUR + min * 60)
}

/// The forecast in an API answer fetched at `fetched`; `None` if it is not usable (not JSON,
/// no hourly series, irregular hours). Accepts `cloud_cover` and the older `cloudcover`.
pub fn parse(json: &str, fetched: i64) -> Option<Forecast> {
    let Json::Obj(top) = Parser::new(json).value()? else {
        return None;
    };
    let Some(Json::Obj(hourly)) = field(&top, "hourly") else {
        return None;
    };
    let (Some(Json::Arr(times)), Some(Json::Arr(values))) = (
        field(hourly, "time"),
        field(hourly, "cloud_cover").or_else(|| field(hourly, "cloudcover")),
    ) else {
        return None;
    };
    // Null entries (no data yet) end the series.
    let cover: Vec<u8> = values
        .iter()
        .map_while(|v| match v {
            Json::Num(n) => Some(n.round().clamp(0.0, 100.0) as u8),
            _ => None,
        })
        .take(times.len())
        .collect();
    let stamp = |k: usize| match times.get(k) {
        Some(Json::Str(s)) => parse_time(s),
        _ => None,
    };
    let start = stamp(0)?;
    if cover.is_empty() || (cover.len() > 1 && stamp(1)? != start + HOUR) {
        return None;
    }
    Some(Forecast {
        fetched,
        start,
        cover,
    })
}

/// Cover rounded down to its [`BUCKET_PCT`] step: the value planning works with.
pub fn bucket(cover: u8) -> u8 {
    cover / BUCKET_PCT * BUCKET_PCT
}

impl Forecast {
    fn fresh(&self, now: i64) -> bool {
        (0..=MAX_AGE_SECS).contains(&(now - self.fetched))
    }

    /// Cloud cover (percent) at `now`, interpolated between hours; `None` when the forecast
    /// is stale or does not cover `now`.
    pub fn cover_at(&self, now: i64) -> Option<u8> {
        if !self.fresh(now) || now < self.start {
            return None;
        }
        let k = ((now - self.start) / HOUR) as usize;
        let a = f64::from(*self.cover.get(k)?);
        let Some(&b) = self.cover.get(k + 1) else {
            return (now == self.start + k as i64 * HOUR).then_some(a as u8);
        };
        let f = ((now - self.start) % HOUR) as f64 / HOUR as f64;
        Some((a + (f64::from(b) - a) * f).round() as u8)
    }

    /// The bucketed cover of the hour containing `now` (what the planner uses).
    pub fn planning_cover(&self, now: i64) -> Option<u8> {
        if !self.fresh(now) || now < self.start {
            return None;
        }
        let k = ((now - self.start) / HOUR) as usize;
        self.cover.get(k).copied().map(bucket)
    }

    /// The next hour boundary at which [`Forecast::planning_cover`] differs from its value
    /// now (the data running out counts), or when the forecast goes stale; `None` if there
    /// is no forecast now.
    pub fn next_change(&self, now: i64) -> Option<i64> {
        let current = self.planning_cover(now)?;
        let expiry = self.fetched + MAX_AGE_SECS + 1;
        let mut t = now + HOUR - (now - self.start).rem_euclid(HOUR);
        while t < expiry {
            if self.planning_cover(t) != Some(current) {
                return Some(t);
            }
            t += HOUR;
        }
        Some(expiry)
    }
}

/// What the Settings window shows about the last fetch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    None,
    /// Local time `(hour, minute)` of the last good fetch and the cloud cover then.
    Ok {
        at: (u8, u8),
        cover: u8,
    },
    Failed(String),
}

/// When the next fetch is due: after the last good one, or after the last failed attempt.
pub fn next_fetch(last_try: i64, ok: bool) -> i64 {
    last_try + if ok { REFRESH_SECS } else { RETRY_SECS }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-11-18T00:00 GMT.
    const T0: i64 = 1_794_960_000;

    fn json(cover_key: &str) -> String {
        let times: Vec<String> = (0..48)
            .map(|h| format!("\"2026-11-{:02}T{:02}:00\"", 18 + h / 24, h % 24))
            .collect();
        let vals: Vec<String> = (0..48).map(|h| (h * 2).to_string()).collect();
        format!(
            r#"{{"latitude":41.9,"hourly_units":{{"cloud_cover":"%"}},"hourly":{{"time":[{}],"{cover_key}":[{}]}}}}"#,
            times.join(","),
            vals.join(",")
        )
    }

    #[test]
    fn url_rounds_and_sends_only_coordinates() {
        assert_eq!(
            forecast_url(41.9028, -12.4964),
            "https://api.open-meteo.com/v1/forecast?latitude=41.9&longitude=-12.5&hourly=cloud_cover&forecast_days=2&timezone=GMT"
        );
    }

    #[test]
    fn time_parsing() {
        assert_eq!(parse_time("2026-11-18T00:00"), Some(T0));
        assert_eq!(parse_time("1970-01-01T01:30"), Some(5400));
        assert_eq!(parse_time("2024-03-01T00:00"), Some(1_709_251_200));
        assert_eq!(parse_time("nope"), None);
        assert_eq!(parse_time("2026-13-01T00:00"), None);
    }

    #[test]
    fn parses_both_key_names() {
        for key in ["cloud_cover", "cloudcover"] {
            let f = parse(&json(key), T0 + 600).unwrap();
            assert_eq!(f.start, T0);
            assert_eq!(f.cover.len(), 48);
            assert_eq!(f.cover[5], 10);
        }
        assert_eq!(parse(&json("temperature_2m"), 0), None);
        assert_eq!(parse("not json", 0), None);
        assert_eq!(parse(r#"{"hourly":{"time":[],"cloud_cover":[]}}"#, 0), None);
        let irregular =
            r#"{"hourly":{"time":["2026-11-18T00:00","2026-11-18T05:00"],"cloud_cover":[1,2]}}"#;
        assert_eq!(parse(irregular, 0), None);
        let nulls = r#"{"hourly":{"time":["2026-11-18T00:00","2026-11-18T01:00"],"cloud_cover":[40,null]}}"#;
        assert_eq!(parse(nulls, T0).unwrap().cover, [40]);
    }

    #[test]
    fn interpolates_and_expires() {
        let f = parse(&json("cloud_cover"), T0 + 600).unwrap();
        assert_eq!(f.cover_at(T0 + 3600 * 5), Some(10));
        assert_eq!(f.cover_at(T0 + 3600 * 5 + 1800), Some(11)); // 10..12 halfway
        let g = parse(&json("cloud_cover"), T0 + 20 * 3600).unwrap();
        assert_eq!(g.cover_at(T0 + 3600 * 47), Some(94));
        assert_eq!(g.cover_at(T0 + 3600 * 47 + 1), None); // beyond the data
        assert_eq!(f.cover_at(T0 - 1), None);
        let late = T0 + 600 + MAX_AGE_SECS;
        assert!(f.cover_at(late).is_some());
        assert_eq!(f.cover_at(late + 1), None);
        assert_eq!(f.cover_at(T0 + 599), None); // clock went back before the fetch
    }

    #[test]
    fn planning_changes_on_hour_boundaries() {
        let f = parse(&json("cloud_cover"), T0 + 600).unwrap();
        assert_eq!(bucket(59), 50);
        // Hour 5 is 10 %, hour 6 is 12 % (same bucket), hour 10 is 20 %.
        assert_eq!(f.planning_cover(T0 + 5 * 3600 + 1800), Some(10));
        assert_eq!(f.next_change(T0 + 5 * 3600 + 1800), Some(T0 + 10 * 3600));
        // A flat forecast changes only at expiry.
        let flat = Forecast {
            fetched: T0,
            start: T0,
            cover: vec![30; 48],
        };
        assert_eq!(flat.next_change(T0 + 100), Some(T0 + MAX_AGE_SECS + 1));
        assert_eq!(flat.next_change(T0 + MAX_AGE_SECS + 5), None);
    }

    #[test]
    fn cadence() {
        assert_eq!(next_fetch(1000, true), 1000 + 3 * 3600);
        assert_eq!(next_fetch(1000, false), 1000 + 1800);
    }
}
