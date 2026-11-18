//! Ambient light sensor mapping and filtering (pure; the caller supplies readings and time).

/// Illuminance at or below which the level is the minimum.
pub const LUX_DARK: f64 = 10.0;
/// Illuminance at or above which the level is the maximum.
pub const LUX_BRIGHT: f64 = 10_000.0;
/// A reading further than this many decades (factor ~2) from the smoothed value is an outlier.
const OUTLIER_DECADES: f64 = 0.3;
/// An outlier must persist this long (seconds) before it is believed; shorter dips
/// (a hand or sleeve over the sensor) are ignored.
pub const HOLD_SECS: i64 = 4;
/// Smoothing time constant in seconds.
const TAU_SECS: f64 = 3.0;

/// Log-scale mapping of lux to a level in `min..=max`: [`LUX_DARK`] and below give
/// `min`, [`LUX_BRIGHT`] and above give `max`, linear in log10(lux) in between.
pub fn lux_to_level(lux: f64, min: f32, max: f32) -> f32 {
    let lo = LUX_DARK.log10();
    let hi = LUX_BRIGHT.log10();
    let t = ((lux.max(1.0).log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
    min + (max - min) * t
}

/// Turns raw sensor readings into occasional new targets.
///
/// Readings are smoothed exponentially in the log domain. Readings that jump by more
/// than a factor of ~2 are held back for [`HOLD_SECS`] seconds: if the old value
/// returns first they are discarded, otherwise the new value is adopted at once.
/// A target is emitted only when it differs from the last emitted one by at least `step`.
#[derive(Clone, Debug)]
pub struct AmbientFilter {
    min: f32,
    max: f32,
    step: f32,
    smoothed: Option<f64>,
    last_t: i64,
    outlier_since: Option<i64>,
    last_sent: Option<f32>,
}

impl AmbientFilter {
    /// `min`/`max` bound the output level, `step` is the hysteresis (e.g. 2.0 percent).
    pub fn new(min: f32, max: f32, step: f32) -> Self {
        AmbientFilter {
            min,
            max,
            step,
            smoothed: None,
            last_t: 0,
            outlier_since: None,
            last_sent: None,
        }
    }

    /// Feed one reading taken at `unix_secs`; returns a new target level when it moved
    /// by at least the step since the last one returned (the first reading always returns).
    pub fn feed(&mut self, lux: f64, unix_secs: i64) -> Option<f32> {
        let x = if lux.is_finite() {
            lux.max(1.0).log10()
        } else {
            0.0
        };
        let dt = (unix_secs - self.last_t).max(0) as f64;
        self.last_t = unix_secs;
        let s = match self.smoothed {
            None => x,
            Some(s) if (x - s).abs() > OUTLIER_DECADES => {
                let since = *self.outlier_since.get_or_insert(unix_secs);
                if unix_secs - since < HOLD_SECS {
                    return None;
                }
                self.outlier_since = None;
                x
            }
            Some(s) => {
                self.outlier_since = None;
                s + (1.0 - (-dt / TAU_SECS).exp()) * (x - s)
            }
        };
        self.smoothed = Some(s);
        let level = lux_to_level(10f64.powf(s), self.min, self.max);
        match self.last_sent {
            Some(prev) if (level - prev).abs() < self.step => None,
            _ => {
                self.last_sent = Some(level);
                Some(level)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(lux_to_level(0.0, 20.0, 100.0), 20.0);
        assert_eq!(lux_to_level(10.0, 20.0, 100.0), 20.0);
        assert_eq!(lux_to_level(100_000.0, 20.0, 100.0), 100.0);
        assert!((lux_to_level(10f64.powf(2.5), 20.0, 100.0) - 60.0).abs() < 0.01);
        assert_eq!(lux_to_level(f64::NAN.max(0.0), 20.0, 100.0), 20.0);
    }

    #[test]
    fn first_reading_emits_then_hysteresis() {
        let mut f = AmbientFilter::new(20.0, 100.0, 2.0);
        assert!(f.feed(300.0, 0).is_some());
        assert_eq!(f.feed(305.0, 1), None);
        assert_eq!(f.feed(300.0, 2), None);
    }

    #[test]
    fn hand_over_sensor_is_ignored() {
        let mut f = AmbientFilter::new(20.0, 100.0, 2.0);
        f.feed(500.0, 0);
        for t in 1..10 {
            assert_eq!(f.feed(500.0, t), None);
        }
        // 3 s of shadow, then back to normal.
        for t in 10..13 {
            assert_eq!(f.feed(5.0, t), None);
        }
        assert_eq!(f.feed(500.0, 13), None);
        assert_eq!(f.feed(500.0, 14), None);
    }

    #[test]
    fn sustained_change_is_adopted() {
        let mut f = AmbientFilter::new(20.0, 100.0, 2.0);
        let first = f.feed(2000.0, 0).unwrap();
        let mut out = None;
        for t in 1..20 {
            out = out.or(f.feed(20.0, t));
        }
        let new = out.expect("a sustained drop must emit");
        assert!(new < first - 20.0);
    }

    #[test]
    fn slow_drift_is_smoothed_and_stepped() {
        let mut f = AmbientFilter::new(0.0, 100.0, 5.0);
        let mut emitted = 0;
        let mut lux = 3000.0;
        for t in 0..120 {
            lux *= 0.97;
            if f.feed(lux, t).is_some() {
                emitted += 1;
            }
        }
        assert!(emitted > 1 && emitted < 15, "{emitted}");
    }
}
