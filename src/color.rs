//! Colors, the default palette, and the WCAG/OKLCH contrast safeguard.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const WHITE: Rgb = Rgb::new(0xFF, 0xFF, 0xFF);
    pub const BLACK: Rgb = Rgb::new(0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }

    /// Parse `#RRGGBB` or `RRGGBB` (case-insensitive).
    pub fn from_hex(s: &str) -> Option<Self> {
        let h = s.trim();
        let h = h.strip_prefix('#').unwrap_or(h);
        if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        Some(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// Format as `#RRGGBB` (uppercase).
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Default per-monitor palette, assigned in order to new monitors.
pub const PALETTE: [Rgb; 7] = [
    Rgb::new(0x37, 0x8A, 0xDD), // Blue
    Rgb::new(0xD8, 0x5A, 0x30), // Coral
    Rgb::new(0x1D, 0x9E, 0x75), // Teal
    Rgb::new(0x7F, 0x77, 0xDD), // Purple
    Rgb::new(0xBA, 0x75, 0x17), // Amber
    Rgb::new(0xD4, 0x53, 0x7E), // Pink
    Rgb::new(0x88, 0x87, 0x80), // Gray
];

/// Pick the first palette color not in `used`; if all are used, cycle by `used.len()`.
pub fn next_palette_color(used: &[Rgb]) -> Rgb {
    PALETTE
        .iter()
        .copied()
        .find(|c| !used.contains(c))
        .unwrap_or(PALETTE[used.len() % PALETTE.len()])
}

fn to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

fn luminance(c: Rgb) -> f32 {
    0.2126 * to_linear(c.r) + 0.7152 * to_linear(c.g) + 0.0722 * to_linear(c.b)
}

/// WCAG 2.x contrast ratio between two colors (1.0 ..= 21.0).
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// sRGB -> OKLCH `(L, C, h)` with `h` in radians.
fn to_oklch(c: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    let ll = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
    let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
    let bb = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    (ll, a.hypot(bb), bb.atan2(a))
}

/// OKLCH -> linear RGB (possibly out of gamut).
fn oklch_to_linear(l: f32, c: f32, h: f32) -> [f32; 3] {
    let (a, b) = (c * h.cos(), c * h.sin());
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_93 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_4 * s_,
        -0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
}

fn in_gamut(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|&v| (-0.0005..=1.0005).contains(&v))
}

/// OKLCH to sRGB, reducing chroma until the color fits the gamut.
fn oklch_to_rgb(l: f32, c: f32, h: f32) -> Rgb {
    let mut rgb = oklch_to_linear(l, c, h);
    if !in_gamut(rgb) {
        let (mut lo, mut hi) = (0.0_f32, c);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(oklch_to_linear(l, mid, h)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb = oklch_to_linear(l, lo, h);
    }
    Rgb::new(
        from_linear(rgb[0]),
        from_linear(rgb[1]),
        from_linear(rgb[2]),
    )
}

/// Return `fg` unchanged if its contrast against `bg` is at least `min_ratio`;
/// otherwise adjust only its OKLCH lightness (keeping hue and, as far as the
/// sRGB gamut allows, chroma) by the smallest amount that passes. Moves toward
/// lighter on dark backgrounds and darker on light ones.
pub fn ensure_contrast(fg: Rgb, bg: Rgb, min_ratio: f32) -> Rgb {
    if contrast_ratio(fg, bg) >= min_ratio {
        return fg;
    }
    let (l0, c, h) = to_oklch(fg);
    let lighter_first = luminance(bg) < 0.5;
    for lighter in [lighter_first, !lighter_first] {
        let target = if lighter { 1.0 } else { 0.0 };
        let at = |t: f32| oklch_to_rgb(l0 + (target - l0) * t, c, h);
        if contrast_ratio(at(1.0), bg) < min_ratio {
            continue;
        }
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if contrast_ratio(at(mid), bg) >= min_ratio {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        return at(hi);
    }
    if contrast_ratio(Rgb::WHITE, bg) >= contrast_ratio(Rgb::BLACK, bg) {
        Rgb::WHITE
    } else {
        Rgb::BLACK
    }
}

/// Black or white, whichever has more contrast on `bg` (for pill text).
pub fn text_on(bg: Rgb) -> Rgb {
    if contrast_ratio(Rgb::WHITE, bg) >= contrast_ratio(Rgb::BLACK, bg) {
        Rgb::WHITE
    } else {
        Rgb::BLACK
    }
}

/// Mix `a` toward `b` by `t` (0.0 = a, 1.0 = b) in sRGB. Used for dimmed states.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb::new(f(a.r, b.r), f(a.g, b.g), f(a.b, b.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DARK: Rgb = Rgb::new(0x20, 0x20, 0x20);
    const LIGHT: Rgb = Rgb::new(0xF3, 0xF3, 0xF3);

    #[test]
    fn hex_roundtrip() {
        assert_eq!(Rgb::from_hex("#378add"), Some(Rgb::new(0x37, 0x8A, 0xDD)));
        assert_eq!(Rgb::from_hex("378ADD").unwrap().to_hex(), "#378ADD");
        assert_eq!(Rgb::from_hex("#12345"), None);
        assert_eq!(Rgb::from_hex("#GGGGGG"), None);
        assert_eq!(Rgb::from_hex(""), None);
    }

    #[test]
    fn palette_picking() {
        assert_eq!(next_palette_color(&[]), PALETTE[0]);
        assert_eq!(next_palette_color(&[PALETTE[0], PALETTE[2]]), PALETTE[1]);
        let all = PALETTE.to_vec();
        assert_eq!(next_palette_color(&all), PALETTE[0]);
    }

    #[test]
    fn contrast_basics() {
        assert!((contrast_ratio(Rgb::WHITE, Rgb::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Rgb::WHITE, Rgb::WHITE) - 1.0).abs() < 0.001);
    }

    #[test]
    fn palette_reaches_3() {
        for bg in [DARK, LIGHT] {
            for c in PALETTE {
                let r = ensure_contrast(c, bg, 3.0);
                assert!(
                    contrast_ratio(r, bg) >= 3.0,
                    "{} on {}",
                    c.to_hex(),
                    bg.to_hex()
                );
            }
        }
    }

    #[test]
    fn passing_unchanged() {
        assert_eq!(ensure_contrast(Rgb::WHITE, DARK, 3.0), Rgb::WHITE);
        assert_eq!(ensure_contrast(Rgb::BLACK, LIGHT, 3.0), Rgb::BLACK);
    }

    #[test]
    fn hue_preserved_and_minimal() {
        let blue = Rgb::new(0x10, 0x40, 0xC0);
        let r = ensure_contrast(blue, DARK, 3.0);
        assert!(r.b > r.r && r.b > r.g);
        assert!(contrast_ratio(r, DARK) >= 3.0);
        assert!(contrast_ratio(r, DARK) < 3.5);
        let r = ensure_contrast(PALETTE[0], LIGHT, 3.0);
        assert!(r.b > r.r && r.b >= r.g);
    }

    #[test]
    fn impossible_falls_back() {
        assert_eq!(
            ensure_contrast(Rgb::new(0x80, 0x80, 0x80), DARK, 21.0),
            Rgb::WHITE
        );
        assert_eq!(
            ensure_contrast(Rgb::new(0x80, 0x80, 0x80), LIGHT, 21.0),
            Rgb::BLACK
        );
    }

    #[test]
    fn text_and_mix() {
        assert_eq!(text_on(Rgb::BLACK), Rgb::WHITE);
        assert_eq!(text_on(Rgb::WHITE), Rgb::BLACK);
        assert_eq!(mix(Rgb::BLACK, Rgb::WHITE, 0.0), Rgb::BLACK);
        assert_eq!(mix(Rgb::BLACK, Rgb::WHITE, 1.0), Rgb::WHITE);
        assert_eq!(mix(Rgb::BLACK, Rgb::WHITE, 0.5), Rgb::new(128, 128, 128));
    }
}
