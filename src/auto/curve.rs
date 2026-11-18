//! Day/night level as a function of sun elevation.
//!
//! The same curve serves brightness (percent) and colour temperature (kelvin):
//! `day` and `night` are plain `f32` values in whatever unit the caller uses.

/// Default elevation below which it is "night" (end of civil twilight).
pub const DEFAULT_LOW_DEG: f64 = -6.0;
/// Default elevation above which it is "day".
pub const DEFAULT_HIGH_DEG: f64 = 6.0;
/// Default strength of [`cloud_dim`]: 30 % dimmer under a fully overcast sky.
pub const DEFAULT_CLOUD_STRENGTH: f32 = 0.3;

/// `night` below `low_deg`, `day` above `high_deg`, smoothstep in between.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayCurve {
    pub day: f32,
    pub night: f32,
    pub low_deg: f64,
    pub high_deg: f64,
}

impl DayCurve {
    /// Curve with the default civil-twilight transition band (-6..+6 degrees).
    pub fn new(day: f32, night: f32) -> Self {
        DayCurve {
            day,
            night,
            low_deg: DEFAULT_LOW_DEG,
            high_deg: DEFAULT_HIGH_DEG,
        }
    }

    /// Level for a sun elevation in degrees. Monotonic in the elevation.
    pub fn level(&self, elevation_deg: f64) -> f32 {
        let span = self.high_deg - self.low_deg;
        let x = if span > 0.0 {
            ((elevation_deg - self.low_deg) / span).clamp(0.0, 1.0)
        } else if elevation_deg >= self.high_deg {
            1.0
        } else {
            0.0
        };
        let s = (x * x * (3.0 - 2.0 * x)) as f32;
        self.night + (self.day - self.night) * s
    }

    /// Same curve with the daytime level lowered for cloud cover (see [`cloud_dim`]),
    /// never below the night level.
    pub fn with_clouds(&self, cloud_cover_pct: f32, strength: f32) -> Self {
        let floor = self.night.min(self.day);
        DayCurve {
            day: cloud_dim(self.day, cloud_cover_pct, strength).max(floor),
            ..*self
        }
    }
}

/// Lower a daytime level on overcast skies:
/// `value_day * (1 - strength * cover)` with `cover = cloud_cover_pct / 100` and
/// both inputs clamped (`cover` to 0..=1, `strength` to 0..=1).
/// With the default strength 0.3, a fully overcast sky gives 70 % of the clear-sky level.
pub fn cloud_dim(value_day: f32, cloud_cover_pct: f32, strength: f32) -> f32 {
    let cover = (cloud_cover_pct / 100.0).clamp(0.0, 1.0);
    value_day * (1.0 - strength.clamp(0.0, 1.0) * cover)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plateaus_and_midpoint() {
        let c = DayCurve::new(80.0, 30.0);
        assert_eq!(c.level(-30.0), 30.0);
        assert_eq!(c.level(-6.0), 30.0);
        assert_eq!(c.level(6.0), 80.0);
        assert_eq!(c.level(60.0), 80.0);
        assert!((c.level(0.0) - 55.0).abs() < 1e-4);
    }

    #[test]
    fn monotonic_and_works_for_kelvin() {
        let c = DayCurve::new(6500.0, 4000.0);
        let mut prev = c.level(-10.0);
        for i in -99..100 {
            let v = c.level(f64::from(i) / 10.0);
            assert!(v >= prev);
            prev = v;
        }
        let inv = DayCurve::new(30.0, 80.0);
        assert!(inv.level(0.0) > 30.0 && inv.level(0.0) < 80.0);
    }

    #[test]
    fn degenerate_band_is_a_step() {
        let c = DayCurve {
            low_deg: 0.0,
            high_deg: 0.0,
            ..DayCurve::new(80.0, 30.0)
        };
        assert_eq!(c.level(-0.1), 30.0);
        assert_eq!(c.level(0.1), 80.0);
    }

    #[test]
    fn clouds() {
        assert_eq!(cloud_dim(80.0, 0.0, 0.3), 80.0);
        assert!((cloud_dim(80.0, 100.0, DEFAULT_CLOUD_STRENGTH) - 56.0).abs() < 1e-4);
        assert!((cloud_dim(80.0, 50.0, 0.3) - 68.0).abs() < 1e-4);
        assert_eq!(cloud_dim(80.0, 250.0, 5.0), 0.0);
        let c = DayCurve::new(80.0, 30.0).with_clouds(100.0, 1.0);
        assert_eq!(c.day, 30.0); // floored at night
        assert_eq!(c.night, 30.0);
    }
}
