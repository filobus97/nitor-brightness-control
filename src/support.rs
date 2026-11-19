//! The occasional "support this project" reminder: the pure rules (the popup is in
//! `win::support`). Nothing is ever sent anywhere; the only state is three numbers in
//! `Nitor.ini`.
//!
//! "Use" is awake running time. It is sampled from `QueryUnbiasedInterruptTime` (which
//! excludes sleep and hibernation) only at events that happen anyway (the hourly update tick,
//! suspend/resume, probe completion, opening the flyout or Settings, shutdown), never by a
//! timer of its own, so the reminder costs no wake-ups. It may appear a little later than the
//! threshold, never earlier.

/// The Ko-fi page.
pub const KOFI_URL: &str = "https://ko-fi.com/pippo97";
/// First reminder: after 10 hours of use.
pub const FIRST_MINUTES: u64 = 600;
/// Then every 100 hours of use after the last one shown.
pub const INTERVAL_MINUTES: u64 = 6000;

/// One minute in `QueryUnbiasedInterruptTime` units (100 ns).
const MINUTE_100NS: u64 = 600_000_000;

/// Whole minutes between two samples of the unbiased interrupt time, and the sample to keep
/// as the new mark: it advances by whole minutes only, so no fraction is ever lost.
pub fn take_minutes(mark: u64, now: u64) -> (u64, u64) {
    let minutes = now.saturating_sub(mark) / MINUTE_100NS;
    (minutes, mark + minutes * MINUTE_100NS)
}

/// Whether the reminder is due.
pub fn due(enabled: bool, usage: u64, next: u64) -> bool {
    enabled && usage >= next
}

/// When the reminder is next due after one was handled at `usage`.
pub fn next_after(usage: u64) -> u64 {
    usage.saturating_add(INTERVAL_MINUTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_keep_the_fraction() {
        let m = MINUTE_100NS;
        assert_eq!(take_minutes(100, 100 + m - 1), (0, 100));
        assert_eq!(take_minutes(100, 100 + 3 * m + 5), (3, 100 + 3 * m));
        assert_eq!(take_minutes(500, 100), (0, 500));
    }

    #[test]
    fn thresholds() {
        assert!(!due(true, 599, FIRST_MINUTES));
        assert!(due(true, 600, FIRST_MINUTES));
        assert!(!due(false, 10_000, FIRST_MINUTES));
        // First at 600, then 6000 after the one shown (110 h, 210 h, ...).
        let mut next = FIRST_MINUTES;
        let mut shown = Vec::new();
        for usage in (0..=13_000).step_by(100) {
            if due(true, usage, next) {
                shown.push(usage);
                next = next_after(usage);
            }
        }
        assert_eq!(shown, vec![600, 6600, 12_600]);
        // A late check pushes the next one back from when it was shown.
        assert_eq!(next_after(700), 6700);
    }
}
