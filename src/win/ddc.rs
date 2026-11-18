//! DDC/CI worker threads (spec §7). One worker per physical monitor; the UI
//! thread only posts requests and receives `DdcEvent`s through `notify`.

use super::monitors::PhysicalHandle;
use crate::throttle::{WriteThrottle, percent_to_raw, raw_to_percent};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use windows::Win32::Devices::Display::{GetVCPFeatureAndVCPFeatureReply, SetVCPFeature};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdcEvent {
    /// Successful read of VCP 0x10, in percent.
    Value(u8),
    /// The last requested value was written, in percent.
    Written(u8),
    /// Read or write failed twice (one retry after 100 ms).
    Unresponsive,
    /// Reads work but writes are rejected (game/HDR preset, driver lock).
    ReadOnly,
}

/// Called from the worker thread with `(key, event)`. Must be cheap and non-blocking.
pub type Notify = Arc<dyn Fn(u64, DdcEvent) + Send + Sync>;

const VCP_BRIGHTNESS: u8 = 0x10;
const RETRY_DELAY: Duration = Duration::from_millis(100);

struct State {
    throttle: WriteThrottle,
    read_pending: bool,
    paused: bool,
    stop: bool,
    /// Monitor maximum for VCP 0x10, known after the first successful read.
    max: Option<u32>,
    /// Requested percent that could not yet be converted (max unknown).
    pending_percent: Option<u8>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct DdcWorker {
    shared: Shared,
}

impl DdcWorker {
    /// Start a worker that owns `handle`. `key` is echoed in every event so the
    /// UI can ignore events from workers of a previous scan.
    pub fn spawn(
        key: u64,
        handle: PhysicalHandle,
        write_interval: Duration,
        notify: Notify,
    ) -> DdcWorker {
        let shared: Shared = Arc::new((
            Mutex::new(State {
                throttle: WriteThrottle::new(write_interval),
                read_pending: false,
                paused: false,
                stop: false,
                max: None,
                pending_percent: None,
            }),
            Condvar::new(),
        ));
        let s2 = shared.clone();
        std::thread::Builder::new()
            .name(format!("ddc-{key}"))
            .spawn(move || run(key, handle, s2, notify))
            .expect("spawn ddc thread");
        DdcWorker { shared }
    }

    /// Queue a read (coalesced with other pending reads).
    pub fn read(&self) {
        lock(&self.shared.0).read_pending = true;
        self.shared.1.notify_all();
    }

    /// Queue a write in percent; replaces any pending write (latest value wins).
    pub fn write(&self, percent: u8) {
        {
            let mut st = lock(&self.shared.0);
            match st.max {
                Some(max) => {
                    st.pending_percent = None;
                    st.throttle.request(percent_to_raw(percent, max));
                }
                None => {
                    // Need the monitor's maximum first.
                    st.pending_percent = Some(percent);
                    st.read_pending = true;
                }
            }
        }
        self.shared.1.notify_all();
    }

    pub fn set_write_interval(&self, interval: Duration) {
        lock(&self.shared.0).throttle.set_interval(interval);
        self.shared.1.notify_all();
    }

    /// While paused (display off), writes stay pending and are not sent.
    pub fn set_paused(&self, paused: bool) {
        lock(&self.shared.0).paused = paused;
        self.shared.1.notify_all();
    }
}

impl Drop for DdcWorker {
    /// Signal the thread to stop after its current operation; does not join.
    fn drop(&mut self) {
        lock(&self.shared.0).stop = true;
        self.shared.1.notify_all();
    }
}

enum Action {
    Read,
    Write(u32, u32),
}

/// Block until there is something to do; `None` means stop.
fn next_action(shared: &Shared) -> Option<Action> {
    let (m, cv) = &**shared;
    let mut st = lock(m);
    loop {
        if st.stop {
            return None;
        }
        let mut timeout = None;
        if !st.paused {
            if st.read_pending {
                st.read_pending = false;
                return Some(Action::Read);
            }
            if let (Some(p), Some(max)) = (st.pending_percent.take(), st.max) {
                st.throttle.request(percent_to_raw(p, max));
            }
            let now = Instant::now();
            match st.throttle.next(now) {
                crate::throttle::Next::Write(raw) => {
                    // `max` is always known once a raw value was requested.
                    return Some(Action::Write(raw, st.max.unwrap_or(100)));
                }
                crate::throttle::Next::WaitUntil(t) => {
                    timeout = Some(t.saturating_duration_since(now))
                }
                crate::throttle::Next::Idle => {}
            }
        }
        st = match timeout {
            Some(d) => cv.wait_timeout(st, d).unwrap_or_else(|e| e.into_inner()).0,
            None => cv.wait(st).unwrap_or_else(|e| e.into_inner()),
        };
    }
}

/// One DDC read of VCP 0x10 with one retry. Returns `(current, max)`.
fn read_raw(h: &PhysicalHandle) -> Option<(u32, u32)> {
    for attempt in 0..2 {
        let (mut cur, mut max) = (0u32, 0u32);
        // SAFETY: valid physical-monitor handle; out-pointers are valid locals.
        let ok = unsafe {
            GetVCPFeatureAndVCPFeatureReply(h.raw(), VCP_BRIGHTNESS, None, &mut cur, Some(&mut max))
        };
        if ok != 0 {
            return Some((cur, max));
        }
        if attempt == 0 {
            std::thread::sleep(RETRY_DELAY);
        }
    }
    None
}

/// One DDC write of VCP 0x10 with one retry.
fn write_raw(h: &PhysicalHandle, raw: u32) -> bool {
    for attempt in 0..2 {
        // SAFETY: valid physical-monitor handle.
        if unsafe { SetVCPFeature(h.raw(), VCP_BRIGHTNESS, raw) } != 0 {
            return true;
        }
        if attempt == 0 {
            std::thread::sleep(RETRY_DELAY);
        }
    }
    false
}

/// Read and record the result in shared state; returns the percent on success.
fn do_read(h: &PhysicalHandle, shared: &Shared) -> Option<u8> {
    let (cur, max) = read_raw(h)?;
    let mut st = lock(&shared.0);
    st.max = Some(max);
    st.throttle.confirm(cur);
    Some(raw_to_percent(cur, max))
}

fn run(key: u64, handle: PhysicalHandle, shared: Shared, notify: Notify) {
    while let Some(action) = next_action(&shared) {
        // No lock is held during DDC I/O or notify.
        match action {
            Action::Read => match do_read(&handle, &shared) {
                Some(p) => notify(key, DdcEvent::Value(p)),
                None => notify(key, DdcEvent::Unresponsive),
            },
            Action::Write(raw, max) => {
                if write_raw(&handle, raw) {
                    lock(&shared.0).throttle.confirm(raw);
                    notify(key, DdcEvent::Written(raw_to_percent(raw, max)));
                } else {
                    lock(&shared.0).throttle.invalidate();
                    match do_read(&handle, &shared) {
                        Some(p) => {
                            notify(key, DdcEvent::ReadOnly);
                            notify(key, DdcEvent::Value(p));
                        }
                        None => notify(key, DdcEvent::Unresponsive),
                    }
                }
            }
        }
    }
    // `handle` drops here, destroying the physical monitor.
}

/// Blocking single read with one retry, for standalone `list`/`get`. Returns percent.
pub fn read_once(handle: &PhysicalHandle) -> Option<u8> {
    read_raw(handle).map(|(cur, max)| raw_to_percent(cur, max))
}
