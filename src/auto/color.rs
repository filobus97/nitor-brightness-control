//! Colour temperature to RGB multipliers and gamma ramps (pure).

/// Coldest and warmest supported temperatures (kelvin).
pub const MIN_K: u32 = 1000;
/// Neutral white point: multipliers are exactly `(1, 1, 1)` here.
pub const MAX_K: u32 = 6500;
/// Below this, Windows clamps gamma ramps (they are rejected by `SetDeviceGammaRamp`)
/// unless the `GdiIcmGammaRange` registry value is set; treated as experimental.
pub const WARM_LIMIT_K: u32 = 4000;

/// True when `kelvin` needs the experimental extended gamma range.
pub fn needs_experimental_warm(kelvin: u32) -> bool {
    kelvin < WARM_LIMIT_K
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
}
