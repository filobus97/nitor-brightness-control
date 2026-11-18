//! DDC/CI worker threads (spec §7). One worker per physical monitor; the UI
//! thread only posts requests and receives `DdcEvent`s through `notify`.

use super::monitors::PhysicalHandle;
#[cfg(feature = "plus")]
use super::util::log;
use super::wmi::Wmi;
#[cfg(feature = "plus")]
use crate::auto::color::{DDC_MIN_INTERVAL_SECS, gain_target, rgb_multipliers};
use crate::throttle::{WriteThrottle, percent_to_raw, raw_to_percent};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use windows::Win32::Devices::Display::{GetVCPFeatureAndVCPFeatureReply, SetVCPFeature};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

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
    /// Plus: the monitor answered to the red, green and blue video gains (colour by hardware).
    GainsReady,
    /// Plus: the video gains are missing or rejected; colour must use gamma ramps.
    GainsUnavailable,
}

/// Called from the worker thread with `(key, event)`. Must be cheap and non-blocking.
pub type Notify = Arc<dyn Fn(u64, DdcEvent) + Send + Sync>;

const VCP_BRIGHTNESS: u8 = 0x10;
/// VCP codes of the red, green and blue video gains.
#[cfg(feature = "plus")]
const VCP_GAINS: [u8; 3] = [0x16, 0x18, 0x1A];
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
    #[cfg(feature = "plus")]
    color: ColorSlot,
}

/// What the colour automation asks of a worker (latest request wins).
#[cfg(feature = "plus")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorRequest {
    /// Scale the original gains to this colour temperature (kelvin).
    Apply(u32),
    /// Put the original gains back, immediately.
    Restore,
}

/// Video-gain channels of a monitor, known after the probe.
#[cfg(feature = "plus")]
#[derive(Clone, Copy)]
enum Gains {
    Untried,
    Unavailable,
    Ready { original: [u32; 3], max: [u32; 3] },
}

/// Colour state of one worker (a request slot plus the gain bookkeeping).
#[cfg(feature = "plus")]
struct ColorSlot {
    request: Option<ColorRequest>,
    gains: Gains,
    /// Gain values on the monitor after the last read or write.
    current: [u32; 3],
    /// `current` differs from the original gains (a restore is due).
    modified: bool,
    /// A write failed: no more colour writes, only the restore.
    broken: bool,
    /// EEPROM protection: no gain write before this instant.
    next_write: Option<Instant>,
}

#[cfg(feature = "plus")]
enum ColorStep {
    Do(Action),
    WaitUntil(Instant),
    Idle,
}

#[cfg(feature = "plus")]
impl State {
    /// Decide what the colour request needs now; takes the request when it acts on it.
    fn color_step(&mut self, now: Instant) -> ColorStep {
        let c = &mut self.color;
        let Some(request) = c.request else {
            return ColorStep::Idle;
        };
        let Gains::Ready { original, max } = c.gains else {
            if matches!(c.gains, Gains::Untried) && matches!(request, ColorRequest::Apply(_)) {
                return ColorStep::Do(Action::Probe);
            }
            c.request = None;
            return ColorStep::Idle;
        };
        match request {
            ColorRequest::Restore => {
                c.request = None;
                if c.modified {
                    ColorStep::Do(Action::Gains {
                        targets: original,
                        original,
                        kelvin: None,
                    })
                } else {
                    ColorStep::Idle
                }
            }
            ColorRequest::Apply(kelvin) => {
                let mult = rgb_multipliers(kelvin);
                let targets = [0, 1, 2].map(|i| gain_target(original[i], max[i], mult[i]));
                if c.broken || targets == c.current {
                    c.request = None;
                    return ColorStep::Idle;
                }
                match c.next_write.filter(|&t| t > now) {
                    Some(t) => ColorStep::WaitUntil(t),
                    None => {
                        c.request = None;
                        ColorStep::Do(Action::Gains {
                            targets,
                            original,
                            kelvin: Some(kelvin),
                        })
                    }
                }
            }
        }
    }
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// How a worker talks to its monitor.
pub enum Backend {
    /// External monitor over DDC/CI; the worker owns the handle.
    Ddc(PhysicalHandle),
    /// Built-in display through WMI (percent scale, max 100). The WMI `InstanceName` of
    /// the panel, or `None` for the first active one.
    Wmi(Option<String>),
}

pub struct DdcWorker {
    shared: Shared,
}

impl DdcWorker {
    /// Start a worker for `backend`. `key` is echoed in every event so the
    /// UI can ignore events from workers of a previous scan.
    pub fn spawn(
        key: u64,
        backend: Backend,
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
                #[cfg(feature = "plus")]
                color: ColorSlot {
                    request: None,
                    gains: Gains::Untried,
                    current: [0; 3],
                    modified: false,
                    broken: false,
                    next_write: None,
                },
            }),
            Condvar::new(),
        ));
        let s2 = shared.clone();
        std::thread::Builder::new()
            .name(format!("ddc-{key}"))
            .spawn(move || run(key, backend, s2, notify))
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

    /// Plus: queue a colour request; replaces any pending one (latest wins). The gain
    /// writes are throttled for the monitor's EEPROM, a restore is not.
    #[cfg(feature = "plus")]
    pub fn set_color(&self, request: ColorRequest) {
        lock(&self.shared.0).color.request = Some(request);
        self.shared.1.notify_all();
    }

    /// Plus: block until a requested restore is done, or `deadline` passes (exit, rescan).
    #[cfg(feature = "plus")]
    pub fn wait_color_restored(&self, deadline: Instant) {
        let (m, cv) = &*self.shared;
        let mut st = lock(m);
        while st.color.request.is_some() || st.color.modified {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            st = cv
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
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
    /// Plus: read the three video gains.
    #[cfg(feature = "plus")]
    Probe,
    /// Plus: bring the gains to `targets` (`kelvin`: the temperature, `None` for a restore).
    #[cfg(feature = "plus")]
    Gains {
        targets: [u32; 3],
        original: [u32; 3],
        kelvin: Option<u32>,
    },
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
            #[cfg(feature = "plus")]
            match st.color_step(now) {
                ColorStep::Do(action) => return Some(action),
                ColorStep::WaitUntil(t) => {
                    let d = t.saturating_duration_since(now);
                    timeout = Some(timeout.map_or(d, |w: Duration| w.min(d)));
                }
                // A request may just have been dropped: wake `wait_color_restored`.
                ColorStep::Idle => cv.notify_all(),
            }
        }
        st = match timeout {
            Some(d) => cv.wait_timeout(st, d).unwrap_or_else(|e| e.into_inner()).0,
            None => cv.wait(st).unwrap_or_else(|e| e.into_inner()),
        };
    }
}

/// A monitor's brightness channel: raw values with a maximum.
enum Panel {
    Ddc(PhysicalHandle),
    /// `None`: WMI could not be connected, every operation fails.
    Wmi(Option<Wmi>),
}

impl Panel {
    /// Build the panel on the worker thread (WMI needs COM initialized there).
    fn open(backend: Backend) -> Panel {
        match backend {
            Backend::Ddc(h) => Panel::Ddc(h),
            Backend::Wmi(instance) => {
                // SAFETY: once per worker thread, which never uninitializes COM.
                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                Panel::Wmi(Wmi::connect(instance))
            }
        }
    }

    /// One read with one retry. Returns `(current, max)`.
    fn read_raw(&self) -> Option<(u32, u32)> {
        retry(|| match self {
            Panel::Ddc(h) => ddc_read(h),
            Panel::Wmi(w) => w.as_ref()?.read().map(|p| (p.into(), 100)),
        })
    }

    /// Plus: one retried read of each video gain. Returns `(current, max)` of all three.
    #[cfg(feature = "plus")]
    fn read_gains(&self) -> Option<([u32; 3], [u32; 3])> {
        let Panel::Ddc(h) = self else { return None };
        let (mut cur, mut max) = ([0; 3], [0; 3]);
        for (i, code) in VCP_GAINS.into_iter().enumerate() {
            let (c, m) = retry(|| ddc_read_code(h, code))?;
            (cur[i], max[i]) = (c, m);
            if m == 0 {
                return None;
            }
        }
        Some((cur, max))
    }

    /// Plus: one write of video gain `channel` with one retry.
    #[cfg(feature = "plus")]
    fn write_gain(&self, channel: usize, raw: u32) -> bool {
        let Panel::Ddc(h) = self else { return false };
        retry(|| ddc_write_code(h, VCP_GAINS[channel], raw).then_some(())).is_some()
    }

    /// One write with one retry.
    fn write_raw(&self, raw: u32) -> bool {
        retry(|| match self {
            Panel::Ddc(h) => ddc_write(h, raw).then_some(()),
            Panel::Wmi(w) => w.as_ref()?.set(raw as u8).then_some(()),
        })
        .is_some()
    }
}

/// Run `f`, and once more after `RETRY_DELAY` if it failed.
fn retry<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    f().or_else(|| {
        std::thread::sleep(RETRY_DELAY);
        f()
    })
}

/// One DDC read of VCP 0x10. Returns `(current, max)`.
fn ddc_read(h: &PhysicalHandle) -> Option<(u32, u32)> {
    ddc_read_code(h, VCP_BRIGHTNESS)
}

/// One DDC read of a VCP code. Returns `(current, max)`.
fn ddc_read_code(h: &PhysicalHandle, code: u8) -> Option<(u32, u32)> {
    let (mut cur, mut max) = (0u32, 0u32);
    // SAFETY: valid physical-monitor handle; out-pointers are valid locals.
    let ok =
        unsafe { GetVCPFeatureAndVCPFeatureReply(h.raw(), code, None, &mut cur, Some(&mut max)) };
    (ok != 0).then_some((cur, max))
}

/// One DDC write of VCP 0x10.
fn ddc_write(h: &PhysicalHandle, raw: u32) -> bool {
    ddc_write_code(h, VCP_BRIGHTNESS, raw)
}

/// One DDC write of a VCP code.
fn ddc_write_code(h: &PhysicalHandle, code: u8, raw: u32) -> bool {
    // SAFETY: valid physical-monitor handle.
    unsafe { SetVCPFeature(h.raw(), code, raw) != 0 }
}

/// Read and record the result in shared state; returns the percent on success.
fn do_read(p: &Panel, shared: &Shared) -> Option<u8> {
    let (cur, max) = p.read_raw()?;
    let mut st = lock(&shared.0);
    st.max = Some(max);
    st.throttle.confirm(cur);
    Some(raw_to_percent(cur, max))
}

fn run(key: u64, backend: Backend, shared: Shared, notify: Notify) {
    let panel = Panel::open(backend);
    while let Some(action) = next_action(&shared) {
        // No lock is held during DDC I/O or notify.
        match action {
            Action::Read => match do_read(&panel, &shared) {
                Some(p) => notify(key, DdcEvent::Value(p)),
                None => notify(key, DdcEvent::Unresponsive),
            },
            #[cfg(feature = "plus")]
            Action::Probe => {
                let found = panel.read_gains();
                {
                    let mut st = lock(&shared.0);
                    st.color.gains = match found {
                        Some((original, max)) => {
                            st.color.current = original;
                            Gains::Ready { original, max }
                        }
                        None => Gains::Unavailable,
                    };
                }
                notify(
                    key,
                    if found.is_some() {
                        DdcEvent::GainsReady
                    } else {
                        DdcEvent::GainsUnavailable
                    },
                );
            }
            #[cfg(feature = "plus")]
            Action::Gains {
                targets,
                original,
                kelvin,
            } => {
                let mut now_vals = lock(&shared.0).color.current;
                let mut ok = true;
                for (i, &target) in targets.iter().enumerate() {
                    // Only a channel that really changes is written (EEPROM).
                    if target != now_vals[i] {
                        ok = panel.write_gain(i, target);
                        if !ok {
                            break;
                        }
                        now_vals[i] = target;
                    }
                }
                {
                    let mut st = lock(&shared.0);
                    st.color.current = now_vals;
                    st.color.modified = now_vals != original;
                    st.color.next_write =
                        Some(Instant::now() + Duration::from_secs(DDC_MIN_INTERVAL_SECS));
                    // A failed colour write puts the original gains back (not retried).
                    if !ok && kelvin.is_some() {
                        st.color.broken = true;
                        st.color.request = Some(ColorRequest::Restore);
                    }
                }
                log(&format!(
                    "color ddc key={key} kelvin={kelvin:?} gains={now_vals:?} ok={ok}"
                ));
                if !ok && kelvin.is_some() {
                    notify(key, DdcEvent::GainsUnavailable);
                }
                // Wakes `wait_color_restored`.
                shared.1.notify_all();
            }
            Action::Write(raw, max) => {
                if panel.write_raw(raw) {
                    lock(&shared.0).throttle.confirm(raw);
                    notify(key, DdcEvent::Written(raw_to_percent(raw, max)));
                } else {
                    lock(&shared.0).throttle.invalidate();
                    match do_read(&panel, &shared) {
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
    // `panel` drops here (destroying a physical monitor handle / releasing WMI).
}

/// Blocking single read with one retry, for standalone `list`/`get`. Returns percent.
pub fn read_once(handle: &PhysicalHandle) -> Option<u8> {
    retry(|| ddc_read(handle)).map(|(cur, max)| raw_to_percent(cur, max))
}
