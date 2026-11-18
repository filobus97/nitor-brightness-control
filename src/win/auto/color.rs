//! Colour temperature (Nitor Plus): per-display method choice and application.
//!
//! Software first: a gamma ramp on the display's DC always works and never writes to the
//! monitor. Only a monitor whose `color_method` is `ddc` gets its red/green/blue video gains
//! (VCP 0x16/0x18/0x1A) scaled by its DDC worker (see `ddc::ColorRequest`: probing, EEPROM
//! throttle, read-back check and restore live there); one that does not really apply them is
//! put back and falls back to gamma for the session. Displays in HDR are skipped.
//!
//! No polling: the planner (`crate::auto::plan`, same twilight curve as brightness) says
//! when the temperature will have moved by 100 K; `TIMER_COLOR` sleeps until then.

use super::engine::now_unix;
use super::{TIMER_COLOR, WM_APP_AUTO_COLOR};
use crate::auto::color::{
    MAX_K, MIN_DELTA_K, Method, curve, gamma_kelvin, gamma_ramp, night_light_on,
};
use crate::auto::drive::{location, override_until, timer_delay_ms};
use crate::auto::plan::{PlanInput, plan};
use crate::config::{ColorMethodPref, Config};
use crate::win::ddc::{ColorRequest, DdcWorker};
use crate::win::util::{log, wide};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateDCW, DeleteDC};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_ROUTINE_FLAGS, RRF_RT_REG_BINARY,
    RRF_RT_REG_DWORD, RegGetValueW,
};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::UI::ColorSystem::SetDeviceGammaRamp;
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SW_HIDE, SetTimer};
use windows::core::PCWSTR;

/// How long a rescan or exit waits for the monitors to take their original gains back.
const RESTORE_WAIT: Duration = Duration::from_millis(1500);
const ICM_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM";
const NIGHT_LIGHT_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate";

/// What the colour engine needs to know about one monitor.
pub struct ColorView<'a> {
    pub id: &'a str,
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub device_name: &'a str,
    /// The DDC worker, for external monitors (`None`: built-in panel or not controllable).
    pub ddc: Option<&'a DdcWorker>,
    /// HDR / advanced colour is on.
    pub hdr: bool,
    /// The user's choice of method for this monitor.
    pub pref: ColorMethodPref,
}

/// Colour state of one display.
#[derive(Default)]
struct Disp {
    /// Decided method; `None` while a DDC probe is out or before the first decision.
    method: Option<Method>,
    /// The method choice the display was last brought to its color under.
    pref: Option<ColorMethodPref>,
    /// Kelvin last requested (hardware) or set (gamma, after the warm limit).
    shown: Option<u32>,
    /// A gamma ramp was set (an identity ramp is due on restore).
    ramp: bool,
}

pub struct ColorEngine {
    hwnd: isize,
    /// Kelvin the planner last settled on (shared by all displays).
    kelvin: Option<f32>,
    /// Flyout override while color is automatic: `(kelvin, unix time it ends)`. Memory only.
    hold: Option<(u32, i64)>,
    /// The temperature the displays were last brought to (neutral when none).
    current: u32,
    /// `gamma_range_extended()` and the option, as of the last planning round.
    extended: bool,
    displays: HashMap<String, Disp>,
    /// Monitors whose video gains did not work this session: they use gamma.
    unsupported: HashSet<String>,
}

impl ColorEngine {
    pub fn new(hwnd: HWND) -> ColorEngine {
        ColorEngine {
            hwnd: hwnd.0 as isize,
            kelvin: None,
            hold: None,
            current: MAX_K,
            extended: false,
            displays: HashMap::new(),
            unsupported: HashSet::new(),
        }
    }

    /// Whether monitor `id`'s hardware color did not work this session.
    pub fn unsupported(&self, id: &str) -> bool {
        self.unsupported.contains(id)
    }

    /// The method decided for monitor `id` (`None` until decided).
    pub fn method(&self, id: &str) -> Option<Method> {
        self.displays.get(id).and_then(|d| d.method)
    }

    /// The temperature the displays show now (6500 K: neutral).
    pub fn current(&self) -> u32 {
        self.current
    }

    /// The flyout slider is being dragged: show `kelvin` through gamma ramps only; DDC
    /// monitors wait for the final value (their gains live in an EEPROM).
    pub fn preview(&mut self, views: &[ColorView], kelvin: u32) {
        for v in views {
            self.apply(v, kelvin, true, true);
        }
    }

    /// The user picked `kelvin` in the flyout while color is automatic: hold the plan off
    /// for as long as a manual brightness change would (see `Auto::override_mode`).
    pub fn hold(&mut self, cfg: &Config, kelvin: u32) {
        self.hold = Some((kelvin, override_until(&cfg.auto, now_unix())));
    }

    /// One planning round: bring every display to the manual temperature (flyout), or the
    /// planned one, or back to neutral while color automation is off or paused
    /// (`pause_until`), and re-arm the timer. `user`: the round follows a change the user made
    /// (flyout, Settings), so monitors take it at once instead of at the EEPROM-friendly pace.
    pub fn plan(
        &mut self,
        cfg: &Config,
        views: &[ColorView],
        pause_until: Option<i64>,
        user: bool,
    ) {
        let now = now_unix();
        let a = &cfg.auto;
        self.extended = a.experimental_warm && gamma_range_extended();
        if !a.color_enabled || self.hold.is_some_and(|(_, t)| t <= now) {
            self.hold = None;
            // The plan starts afresh after a hold.
            self.kelvin = None;
        }
        let manual = match (a.color_enabled, self.hold) {
            (true, hold) => hold.map(|(k, t)| (k, Some(t))),
            (false, _) => a
                .manual_kelvin
                .map(|k| (u32::from(k), None))
                .filter(|&(k, _)| k < MAX_K),
        };
        if let Some((k, until)) = manual {
            self.current = k;
            for v in views {
                self.apply(v, k, false, user);
            }
            self.arm(until, now);
            return;
        }
        let paused = pause_until.filter(|&t| t > now);
        if !a.color_enabled || paused.is_some() {
            self.neutral(views);
            self.arm(paused.filter(|_| a.color_enabled), now);
            return;
        }
        let Some((lat, lon)) = location(a) else {
            self.arm(None, now);
            return;
        };
        let p = plan(&PlanInput {
            lat,
            lon,
            curve: curve(a),
            applied: self.kelvin,
            min_delta: MIN_DELTA_K,
            override_until: None,
            pause_until: None,
            now,
        });
        if let Some(k) = p.target {
            self.kelvin = Some(k);
        }
        if let Some(k) = self.kelvin {
            self.current = k.round() as u32;
            for v in views {
                self.apply(v, self.current, false, user);
            }
        }
        self.arm(Some(p.next_wake_unix), now);
    }

    /// Give `v` the temperature `kelvin` by its method, deciding the method first. With
    /// `live` (a drag in progress) only gamma ramps are written.
    fn apply(&mut self, v: &ColorView, kelvin: u32, live: bool, user: bool) {
        let extended = self.extended;
        let unsupported = self.unsupported.contains(v.id);
        let d = self.displays.entry(v.id.to_string()).or_default();
        if d.pref.is_some_and(|p| p != v.pref) {
            // The method changed: the old one's state goes back first.
            if let Some(w) = v.ddc {
                w.set_color(ColorRequest::Restore);
            }
            if std::mem::take(&mut d.ramp) {
                set_ramp(v.device_name, MAX_K);
            }
            (d.method, d.shown) = (None, None);
            log(&format!("color id={} method choice {:?}", v.id, v.pref));
        }
        d.pref = Some(v.pref);
        let ddc = v
            .ddc
            .filter(|_| v.pref == ColorMethodPref::Ddc && !unsupported);
        if d.method.is_none() && ddc.is_none() {
            let m = if v.hdr { Method::Hdr } else { Method::Gamma };
            d.method = Some(m);
            log(&format!("color id={} method={m:?}", v.id));
        }
        match (d.method, ddc) {
            (Some(Method::Gamma), _) => {
                let target = gamma_kelvin(kelvin, extended);
                if d.shown != Some(target) {
                    d.shown = Some(target);
                    d.ramp = true;
                    let ok = set_ramp(v.device_name, target);
                    log(&format!("color id={} {target}K gamma ok={ok}", v.id));
                }
            }
            (Some(Method::Hdr), _) | (_, None) => {}
            (_, Some(_)) if live => {}
            // Hardware, or still probing: the worker probes before its first write.
            (_, Some(w)) => {
                if d.shown != Some(kelvin) {
                    if d.method.is_none() && d.shown.is_none() {
                        log(&format!("color id={} probing DDC gains", v.id));
                    }
                    d.shown = Some(kelvin);
                    w.set_color(ColorRequest::Apply { kelvin, user });
                    log(&format!(
                        "color id={} {kelvin}K ddc requested ({})",
                        v.id,
                        if user { "user" } else { "auto" }
                    ));
                }
            }
        }
    }

    /// The worker of monitor `id` reported its gains: true if colors need planning again
    /// (the monitor falls back to gamma). `available` is false when the gains are missing,
    /// rejected, or were not what was written when read back.
    pub fn on_gains(&mut self, id: &str, available: bool) -> bool {
        if !available {
            self.unsupported.insert(id.to_string());
        }
        let Some(d) = self.displays.get_mut(id) else {
            return false;
        };
        if available {
            d.method = Some(Method::Hardware);
            log(&format!("color id={id} method=Hardware"));
            false
        } else {
            (d.method, d.shown) = (None, None);
            log(&format!("color id={id} DDC gains unavailable, using gamma"));
            true
        }
    }

    /// The user picked Hardware for monitor `id`: forget an earlier failure of this session
    /// so the monitor is tried again.
    pub fn select_hardware(&mut self, id: &str, ddc: Option<&DdcWorker>) {
        if self.unsupported.remove(id) {
            if let Some(w) = ddc {
                w.retry_color();
            }
            log(&format!("color id={id} hardware chosen again, retrying"));
        }
    }

    /// "Restore monitor colors": the worker writes the saved original gains (or equal
    /// values) at once, and the monitor goes on with software color; the caller sets that
    /// method in the config.
    pub fn restore_monitor(&mut self, id: &str, ddc: Option<&DdcWorker>) {
        if let Some(w) = ddc {
            w.set_color(ColorRequest::Reset);
        }
        // The worker's restore is the old method's cleanup: `apply` must not queue another.
        if let Some(d) = self.displays.get_mut(id) {
            (d.pref, d.method, d.shown) = (Some(ColorMethodPref::Gamma), None, None);
        }
        log(&format!(
            "color id={id} monitor colors restored by the user"
        ));
    }

    /// Back to neutral (6500 K) on every display that was changed; idempotent.
    fn neutral(&mut self, views: &[ColorView]) {
        self.kelvin = None;
        self.current = MAX_K;
        for v in views {
            let Some(d) = self.displays.get_mut(v.id) else {
                continue;
            };
            if d.shown.take().is_none() && !d.ramp {
                continue;
            }
            if let Some(w) = v.ddc {
                w.set_color(ColorRequest::Restore);
            }
            if std::mem::take(&mut d.ramp) {
                set_ramp(v.device_name, MAX_K);
            }
            log(&format!("color id={} neutral", v.id));
        }
    }

    /// Before the monitors are released (rescan, exit): restore everything and wait briefly
    /// for the monitors, then forget all state.
    pub fn release(&mut self, views: &[ColorView]) {
        self.neutral(views);
        let deadline = Instant::now() + RESTORE_WAIT;
        for w in views.iter().filter_map(|v| v.ddc) {
            w.wait_color_restored(deadline);
        }
        self.displays.clear();
        self.arm(None, 0);
    }

    /// Set (or clear) the planner timer for colour.
    fn arm(&self, wake: Option<i64>, now: i64) {
        let hwnd = HWND(self.hwnd as *mut c_void);
        match wake {
            Some(t) => {
                // SAFETY: plain timer call on the app's own window; replaces the old timer.
                unsafe {
                    SetTimer(Some(hwnd), TIMER_COLOR, timer_delay_ms(t + 1, now), None);
                }
            }
            None => {
                // SAFETY: plain timer call on the app's own window.
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_COLOR);
                }
            }
        }
    }
}

/// Set the gamma ramp of display `device_name` to `kelvin`. False if Windows refused.
fn set_ramp(device_name: &str, kelvin: u32) -> bool {
    let name = wide(device_name);
    let ramp = gamma_ramp(kelvin);
    // SAFETY: `name` is NUL-terminated and outlives the calls; the DC is deleted again;
    // `ramp` is the contiguous WORD[3][256] SetDeviceGammaRamp expects.
    unsafe {
        let dc = CreateDCW(PCWSTR::null(), PCWSTR(name.as_ptr()), PCWSTR::null(), None);
        if dc.is_invalid() {
            return false;
        }
        let ok = SetDeviceGammaRamp(dc, ramp.as_ptr().cast()).as_bool();
        let _ = DeleteDC(dc);
        ok
    }
}

/// Read a registry value into `buf`; its length on success.
fn read_value(
    root: HKEY,
    key: &str,
    name: &str,
    kind: REG_ROUTINE_FLAGS,
    buf: &mut [u8],
) -> Option<usize> {
    let (key, name) = (wide(key), wide(name));
    let mut size = buf.len() as u32;
    // SAFETY: both strings are NUL-terminated; `buf` is valid for `size` bytes.
    let r = unsafe {
        RegGetValueW(
            root,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            kind,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (r == ERROR_SUCCESS).then_some(size as usize)
}

/// Whether Windows accepts gamma ramps warmer than 4000 K (`GdiIcmGammaRange` = 256).
pub fn gamma_range_extended() -> bool {
    let mut buf = [0u8; 4];
    read_value(
        HKEY_LOCAL_MACHINE,
        ICM_KEY,
        "GdiIcmGammaRange",
        RRF_RT_REG_DWORD,
        &mut buf,
    )
    .is_some_and(|n| n == 4 && u32::from_le_bytes(buf) == 256)
}

/// Best effort: whether Windows Night Light is on (it would fight the colors). `false`
/// when the (undocumented) setting cannot be read or understood.
pub fn night_light_active() -> bool {
    let mut buf = [0u8; 512];
    read_value(
        HKEY_CURRENT_USER,
        NIGHT_LIGHT_KEY,
        "Data",
        RRF_RT_REG_BINARY,
        &mut buf,
    )
    .is_some_and(|n| night_light_on(&buf[..n]))
}

/// Ask for administrator rights (UAC) to enable the extended gamma range, in the
/// background; posts `WM_APP_AUTO_COLOR` to `hwnd` when `reg.exe` has finished or was refused.
pub fn request_warm_range(hwnd: HWND) {
    let hwnd = hwnd.0 as isize;
    std::thread::spawn(move || {
        let (verb, file) = (wide("runas"), wide("reg.exe"));
        let args = wide(&format!(
            "add \"HKLM\\{ICM_KEY}\" /v GdiIcmGammaRange /t REG_DWORD /d 256 /f"
        ));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(args.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: the strings outlive the calls; the shell wants COM on this thread; the
        // process handle (if any) is ours to wait for and close.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            if ShellExecuteExW(&mut info).is_ok() && !info.hProcess.is_invalid() {
                WaitForSingleObject(info.hProcess, 120_000);
                let _ = CloseHandle(info.hProcess);
            }
            let _ = PostMessageW(
                Some(HWND(hwnd as *mut c_void)),
                WM_APP_AUTO_COLOR,
                WPARAM(0),
                LPARAM(0),
            );
        }
    });
}
