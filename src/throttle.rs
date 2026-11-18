//! Pure logic for DDC write discipline (spec §7.2) and value scaling (§7.1).
//! Time is passed in explicitly so the logic is deterministic in tests.

use std::time::{Duration, Instant};

/// Convert a raw VCP value to percent using the monitor's reported maximum.
/// `max == 0` is treated as 100.
pub fn raw_to_percent(raw: u32, max: u32) -> u8 {
    let max = if max == 0 { 100 } else { max as u64 };
    ((raw as u64 * 100 + max / 2) / max).min(100) as u8
}

/// Convert percent (clamped to 0–100) to a raw VCP value for the given maximum.
pub fn percent_to_raw(percent: u8, max: u32) -> u32 {
    let max = if max == 0 { 100 } else { max as u64 };
    ((percent.min(100) as u64 * max + 50) / 100) as u32
}

/// What a DDC worker should do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Write this raw value now.
    Write(u32),
    /// Something is pending but the rate limit forbids writing before this instant.
    WaitUntil(Instant),
    /// Nothing to do.
    Idle,
}

/// Coalescing "latest value wins" slot with a per-monitor rate limit.
#[derive(Debug)]
pub struct WriteThrottle {
    interval: Duration,
    last_write: Option<Instant>,
    confirmed: Option<u32>,
    pending: Option<u32>,
}

impl WriteThrottle {
    pub fn new(interval: Duration) -> Self {
        WriteThrottle {
            interval,
            last_write: None,
            confirmed: None,
            pending: None,
        }
    }

    pub fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
    }

    /// Request a raw value; overwrites any pending request.
    pub fn request(&mut self, raw: u32) {
        self.pending = Some(raw);
    }

    /// Decide the next step. Returns `Write` only if a pending value differs from
    /// the last confirmed one and the interval since the last write has elapsed;
    /// a pending value equal to the confirmed one is dropped.
    /// Returning `Write` takes the value out of the slot and records `now` as the write time.
    pub fn next(&mut self, now: Instant) -> Next {
        let Some(v) = self.pending else {
            return Next::Idle;
        };
        if self.confirmed == Some(v) {
            self.pending = None;
            return Next::Idle;
        }
        if let Some(last) = self.last_write {
            let ready = last + self.interval;
            if ready > now {
                return Next::WaitUntil(ready);
            }
        }
        self.pending = None;
        self.last_write = Some(now);
        Next::Write(v)
    }

    /// Record a value known to be on the monitor (successful write or read).
    pub fn confirm(&mut self, raw: u32) {
        self.confirmed = Some(raw);
    }

    /// Forget the confirmed value (e.g. after a failed write), so the next request is always written.
    pub fn invalidate(&mut self) {
        self.confirmed = None;
    }

    pub fn confirmed(&self) -> Option<u32> {
        self.confirmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_roundtrip() {
        for max in [100u32, 255, 50] {
            for p in 0..=100u8 {
                let raw = percent_to_raw(p, max);
                assert!(raw <= max);
                let back = raw_to_percent(raw, max);
                if max >= 100 {
                    assert_eq!(back, p, "max {max} p {p}");
                } else {
                    assert!((back as i32 - p as i32).abs() <= 1);
                }
            }
        }
        assert_eq!(raw_to_percent(500, 100), 100);
        assert_eq!(raw_to_percent(40, 0), 40);
        assert_eq!(percent_to_raw(200, 255), 255);
        assert_eq!(raw_to_percent(128, 255), 50);
    }

    #[test]
    fn coalesces_and_rate_limits() {
        let t0 = Instant::now();
        let iv = Duration::from_millis(80);
        let mut t = WriteThrottle::new(iv);
        assert_eq!(t.next(t0), Next::Idle);
        t.request(10);
        t.request(20);
        t.request(30);
        assert_eq!(t.next(t0), Next::Write(30));
        t.confirm(30);
        t.request(40);
        assert_eq!(
            t.next(t0 + Duration::from_millis(10)),
            Next::WaitUntil(t0 + iv)
        );
        t.request(50);
        assert_eq!(t.next(t0 + iv), Next::Write(50));
        assert_eq!(t.next(t0 + iv), Next::Idle);
    }

    #[test]
    fn equal_to_confirmed_dropped_and_invalidate() {
        let t0 = Instant::now();
        let mut t = WriteThrottle::new(Duration::ZERO);
        t.confirm(42);
        t.request(42);
        assert_eq!(t.next(t0), Next::Idle);
        t.request(42);
        t.invalidate();
        assert_eq!(t.confirmed(), None);
        assert_eq!(t.next(t0), Next::Write(42));
    }
}
