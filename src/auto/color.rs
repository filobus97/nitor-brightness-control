//! Colour temperature to RGB multipliers and gamma ramps (pure).

use super::curve::DayCurve;
use crate::config::Auto;

/// Coldest and warmest supported temperatures (kelvin).
pub const MIN_K: u32 = 1000;
/// Neutral white point: multipliers are exactly `(1, 1, 1)` here.
pub const MAX_K: u32 = 6500;
/// Below this, Windows clamps gamma ramps (they are rejected by `SetDeviceGammaRamp`)
/// unless the `GdiIcmGammaRange` registry value is set; treated as experimental.
pub const WARM_LIMIT_K: u32 = 4000;

/// Warmest value the colour sliders offer (below [`WARM_LIMIT_K`] only when experimental).
pub const UI_MIN_K: u32 = 2700;
/// The flyout slider moves in steps of this many kelvin, from [`UI_MIN_K`] to [`MAX_K`].
pub const UI_STEP_K: u32 = 50;
/// Number of steps of the flyout slider (its positions are `0..=UI_STEPS`).
pub const UI_STEPS: u8 = ((MAX_K - UI_MIN_K) / UI_STEP_K) as u8;
/// Smallest kelvin change worth applying; the planner wakes only for steps this big.
pub const MIN_DELTA_K: f32 = 100.0;
/// A DDC monitor's gains are written at most this often (seconds): they live in its EEPROM.
pub const DDC_MIN_INTERVAL_SECS: u64 = 300;

/// True when `kelvin` needs the experimental extended gamma range.
pub fn needs_experimental_warm(kelvin: u32) -> bool {
    kelvin < WARM_LIMIT_K
}

/// Flyout slider position (nearest step) of a temperature.
pub fn kelvin_step(kelvin: u32) -> u8 {
    let k = kelvin.clamp(UI_MIN_K, MAX_K) - UI_MIN_K;
    ((k + UI_STEP_K / 2) / UI_STEP_K) as u8
}

/// Temperature of a flyout slider position.
pub fn step_kelvin(step: u8) -> u32 {
    UI_MIN_K + u32::from(step.min(UI_STEPS)) * UI_STEP_K
}

/// What a white pixel looks like at `kelvin`, for drawing (`[r, g, b]`).
pub fn tint(kelvin: u32) -> [u8; 3] {
    rgb_multipliers(kelvin).map(|m| (m * 255.0).round() as u8)
}

/// How a display gets its colour temperature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// RGB video gains over DDC/CI (VCP 0x16/0x18/0x1A).
    Hardware,
    /// `SetDeviceGammaRamp`.
    Gamma,
    /// HDR / advanced colour is on: gamma ramps do not apply.
    Hdr,
}

/// The temperature a gamma ramp may ask for: Windows rejects ramps warmer than
/// [`WARM_LIMIT_K`] unless the extended range is enabled (`extended`).
pub fn gamma_kelvin(kelvin: u32, extended: bool) -> u32 {
    if extended {
        kelvin
    } else {
        kelvin.max(WARM_LIMIT_K)
    }
}

/// New raw value of one video-gain channel: `original * multiplier`, rounded, at most `max`.
pub fn gain_target(original: u32, max: u32, multiplier: f32) -> u32 {
    let v = (original as f32 * multiplier.clamp(0.0, 1.0)).round() as u32;
    v.min(max.max(original))
}

/// Day/night colour curve (kelvin) of the configuration.
pub fn curve(auto: &Auto) -> DayCurve {
    DayCurve::new(auto.day_kelvin as f32, auto.night_kelvin as f32)
}

/// Windows Night Light state from the `Data` blob of its CloudStore value. Community
/// documented, undocumented by Microsoft: a `43 42 01 00` header and byte 18 equal to
/// `0x15` while on (`0x13` while off). `false` on anything unexpected.
pub fn night_light_on(data: &[u8]) -> bool {
    data.len() >= 19 && data[..4] == [0x43, 0x42, 0x01, 0x00] && data[18] == 0x15
}

/// Tanner Helland's blackbody approximation, 0..=255 per channel.
fn helland(kelvin: f64) -> [f64; 3] {
    let t = kelvin / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
    };
    let g = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };
    [r, g, b].map(|c| c.clamp(0.0, 255.0))
}

/// Per-channel multipliers in `0.0..=1.0` for a colour temperature, clamped to
/// [`MIN_K`]..=[`MAX_K`] and normalised so that [`MAX_K`] is `[1, 1, 1]`.
pub fn rgb_multipliers(kelvin: u32) -> [f32; 3] {
    let k = f64::from(kelvin.clamp(MIN_K, MAX_K));
    let (c, white) = (helland(k), helland(f64::from(MAX_K)));
    [0, 1, 2].map(|i| (c[i] / white[i]).clamp(0.0, 1.0) as f32)
}

/// Gamma ramp (red, green, blue; 256 entries each, identity is `i * 257`) for a
/// colour temperature, ready for `SetDeviceGammaRamp`.
pub fn gamma_ramp(kelvin: u32) -> [[u16; 256]; 3] {
    let m = rgb_multipliers(kelvin);
    let mut ramp = [[0u16; 256]; 3];
    for (channel, mult) in ramp.iter_mut().zip(m) {
        for (i, v) in channel.iter_mut().enumerate() {
            *v = (i as f32 * 257.0 * mult).round() as u16;
        }
    }
    ramp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_at_6500() {
        assert_eq!(rgb_multipliers(6500), [1.0, 1.0, 1.0]);
        assert_eq!(rgb_multipliers(9000), [1.0, 1.0, 1.0]);
        let r = gamma_ramp(6500);
        for c in &r {
            for (i, v) in c.iter().enumerate() {
                assert_eq!(*v, (i * 257) as u16);
            }
        }
    }

    #[test]
    fn slider_steps() {
        assert_eq!((kelvin_step(UI_MIN_K), kelvin_step(MAX_K)), (0, UI_STEPS));
        assert_eq!(UI_STEPS, 76);
        assert_eq!(kelvin_step(100), 0);
        assert_eq!(kelvin_step(9000), UI_STEPS);
        assert_eq!(kelvin_step(4537), 37);
        assert_eq!(step_kelvin(37), 4550);
        assert_eq!(step_kelvin(200), MAX_K);
        for s in 0..=UI_STEPS {
            assert_eq!(kelvin_step(step_kelvin(s)), s);
        }
        assert_eq!(tint(MAX_K), [255, 255, 255]);
        assert!(tint(UI_MIN_K)[2] < tint(UI_MIN_K)[1]);
    }

    #[test]
    fn warmer_reduces_blue_most() {
        let m = rgb_multipliers(3400);
        assert_eq!(m[0], 1.0);
        assert!(m[2] < m[1] && m[1] < m[0]);
        assert!((0.5..0.6).contains(&m[2]), "{m:?}"); // ~0.54 at 3400 K
        let h = rgb_multipliers(1000);
        assert_eq!(h[2], 0.0);
        assert_eq!(rgb_multipliers(10), h); // clamped to 1000 K
    }

    #[test]
    fn monotonic_in_kelvin() {
        let mut prev = rgb_multipliers(1000);
        for k in (1100..=6500).step_by(100) {
            let m = rgb_multipliers(k);
            assert!(m[1] >= prev[1] - 1e-6 && m[2] >= prev[2] - 1e-6, "{k}");
            prev = m;
        }
    }

    #[test]
    fn ramp_is_monotonic_and_scaled() {
        let r = gamma_ramp(4000);
        assert_eq!(r[0][255], 65535);
        assert!(r[2][255] < r[1][255] && r[1][255] < r[0][255]);
        assert!(r[2].windows(2).all(|w| w[0] <= w[1]));
        assert!(needs_experimental_warm(3999) && !needs_experimental_warm(WARM_LIMIT_K));
    }

    #[test]
    fn gamma_clamps_unless_extended() {
        assert_eq!(gamma_kelvin(2700, false), 4000);
        assert_eq!(gamma_kelvin(5000, false), 5000);
        assert_eq!(gamma_kelvin(2700, true), 2700);
    }

    #[test]
    fn gains_scale_and_stay_in_range() {
        assert_eq!(gain_target(50, 100, 1.0), 50);
        assert_eq!(gain_target(50, 100, 0.54), 27);
        assert_eq!(gain_target(50, 100, 0.0), 0);
        assert_eq!(gain_target(80, 100, 2.0), 80);
        // A monitor whose gain already exceeds the reported maximum is never raised.
        assert_eq!(gain_target(120, 100, 1.0), 120);
    }

    #[test]
    fn curve_uses_configured_kelvin() {
        let a = Auto {
            day_kelvin: 6500,
            night_kelvin: 3400,
            ..Auto::default()
        };
        let c = curve(&a);
        assert_eq!((c.day, c.night), (6500.0, 3400.0));
        assert_eq!(c.level(-30.0), 3400.0);
        assert_eq!(c.level(30.0), 6500.0);
    }

    #[test]
    fn night_light_blob() {
        let mut blob = vec![0u8; 41];
        blob[..4].copy_from_slice(&[0x43, 0x42, 0x01, 0x00]);
        blob[18] = 0x15;
        assert!(night_light_on(&blob));
        blob[18] = 0x13;
        assert!(!night_light_on(&blob));
        blob[18] = 0x15;
        blob[0] = 0;
        assert!(!night_light_on(&blob));
        assert!(!night_light_on(&[]));
        assert!(!night_light_on(&[0x43, 0x42, 0x01, 0x00, 0x15]));
    }
}
