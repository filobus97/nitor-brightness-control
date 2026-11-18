//! Calibrated linked brightness: the master slider (0..=100) maps onto each
//! monitor's own `lo..=hi` range.

/// Order a range so `lo <= hi`.
fn ordered(lo: u8, hi: u8) -> (u8, u8) {
    if lo <= hi { (lo, hi) } else { (hi, lo) }
}

/// Monitor value for a master position: `lo + (hi - lo) * master / 100`, rounded.
pub fn link_value(master: u8, lo: u8, hi: u8) -> u8 {
    let (lo, hi) = ordered(lo, hi);
    let master = u32::from(master.min(100));
    let span = u32::from(hi - lo);
    (u32::from(lo) + (span * master + 50) / 100) as u8
}

/// Inverse of [`link_value`]: master position for a monitor value, clamped to 0..=100.
/// A degenerate range (`lo == hi`) carries no information and maps to 0.
pub fn link_master(value: u8, lo: u8, hi: u8) -> u8 {
    let (lo, hi) = ordered(lo, hi);
    if lo == hi {
        return 0;
    }
    let off = u32::from(value.clamp(lo, hi) - lo);
    ((off * 100 + u32::from(hi - lo) / 2) / u32::from(hi - lo)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_master_onto_each_range() {
        assert_eq!(link_value(0, 20, 80), 20);
        assert_eq!(link_value(100, 20, 80), 80);
        assert_eq!(link_value(50, 20, 80), 50);
        assert_eq!(link_value(0, 0, 100), 0);
        assert_eq!(link_value(37, 0, 100), 37);
        assert_eq!(link_value(100, 0, 100), 100);
    }

    #[test]
    fn degenerate_and_reversed_ranges() {
        assert_eq!(link_value(70, 40, 40), 40);
        assert_eq!(link_value(100, 80, 20), 80);
        assert_eq!(link_value(0, 80, 20), 20);
        assert_eq!(link_master(55, 40, 40), 0);
        assert_eq!(link_master(80, 80, 20), 100);
    }

    #[test]
    fn inverse_clamps_and_round_trips() {
        assert_eq!(link_master(10, 20, 80), 0);
        assert_eq!(link_master(95, 20, 80), 100);
        assert_eq!(link_master(50, 20, 80), 50);
        for m in 0..=100u8 {
            assert_eq!(link_master(link_value(m, 0, 100), 0, 100), m);
        }
        // With a narrower range the round trip may be off by at most rounding.
        for m in 0..=100u8 {
            let back = link_master(link_value(m, 20, 80), 20, 80);
            assert!(back.abs_diff(m) <= 1, "{m} -> {back}");
        }
    }
}
