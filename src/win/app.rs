//! UI thread: hidden window, tray icons, menu, wheel, system events (spec §4.1–4.3, §8–10).
//!
//! Layout of this file:
//! - `Row`, target resolution, `query` (list/get) and `build_entries`: shared with the standalone path.
//! - `App`: the single state object, owned by a thread-local; one method per event.
//! - `present`: render + add/modify one tray icon.
//! - window creation, window procedure, context menu, and the `run`/`standalone` entry points.

#[cfg(feature = "plus")]
use super::auto::{self, MonitorView};
use super::ddc::{self, Backend, DdcEvent, DdcWorker, Notify};
use super::flyout::{self, Action, FlyoutRow};
use super::icon::{self, IconContent, IconRenderer, IconSpec, OwnedIcon};
use super::instance::{self, WINDOW_CLASS};
use super::monitors::{self, FoundMonitor};
use super::settings::{self, MonitorEdit, MonitorInfo, MonitorState, Snapshot, UpdateStatus};
use super::support;
use super::theme::{self, Theme};
use super::tray::{self, TrayEvent, WM_APP_TRAY};
use super::update::{self as updater, InstallError};
use super::util::log;
use super::wheel::{self, WM_APP_WHEEL, WheelEvent};
use super::wmi::{self, Wmi};
use super::{autostart, util};
#[cfg(feature = "plus")]
use crate::auto::color::{GainBaseline, MAX_K, UI_MIN_K, WARM_LIMIT_K};
use crate::cli::{self, BrightnessOp, Command, Target};
use crate::color::{Rgb, next_palette_color};
#[cfg(feature = "plus")]
use crate::config::{ColorMethodPref, LocationSource};
use crate::config::{Config, MonitorSettings};
use crate::display_id::display_key;
use crate::i18n::{Key, Lang, tr, tr_with};
use crate::link::link_value;
use crate::startup::{self, StaleFix};
use crate::update::{self, Release};
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{POWERBROADCAST_SETTING, RegisterPowerSettingNotification};
use windows::Win32::System::SystemServices::{
    GUID_CONSOLE_DISPLAY_STATE, GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::{NIN_SELECT, NINF_KEY};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW,
    DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer, MENU_ITEM_FLAGS,
    MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND,
    PBT_POWERSETTINGCHANGE, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterWindowMessageW, SetForegroundWindow, SetMenuDefaultItem, SetTimer, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
    WM_COPYDATA, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_MOUSEMOVE, WM_NULL,
    WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE, WM_THEMECHANGED, WM_TIMER, WNDCLASSEXW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

/// Posted by DDC workers when events are waiting in `App::queue`.
const WM_APP_DDC: u32 = WM_APP + 3;
/// Posted by update threads when events are waiting in `App::update_queue`.
const WM_APP_UPDATE: u32 = WM_APP + 4;
/// `NIN_SELECT | NINF_KEY`: select via keyboard.
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const TIMER_WHEEL: usize = 1;
const TIMER_RESCAN: usize = 2;
/// Hourly: start an automatic update check if 24 h passed (survives sleep).
const TIMER_UPDATE: usize = 3;
/// One-shot: re-scan because monitors did not answer (see `startup::retry_delay_ms`).
const TIMER_RETRY: usize = 4;
/// One-shot: the delayed first scan of a start at logon.
const TIMER_START: usize = 11;
const UPDATE_POLL_MS: u32 = 60 * 60 * 1000;
const WHEEL_POLL_MS: u32 = 100;
/// Brightness notifications this soon after Nitor's own change are treated as echoes.
const OWN_CHANGE_WINDOW: Duration = Duration::from_secs(1);
/// One wheel notch.
const WHEEL_DELTA: i32 = 120;

/// Tray uID of the placeholder icon shown when no monitor is controllable.
const PLACEHOLDER_UID: u32 = 1;
const PLACEHOLDER_COLOR: Rgb = Rgb::new(0x88, 0x87, 0x80);

const ID_RESCAN: u32 = 1;
const ID_SETTINGS: u32 = 2;
const ID_AUTOSTART: u32 = 3;
const ID_EXIT: u32 = 4;
const ID_INTERNAL: u32 = 5;
const ID_CHECK_UPDATES: u32 = 6;
const ID_UPDATE: u32 = 7;
#[cfg(feature = "plus")]
const ID_AUTO_TOGGLE: u32 = 8;

/// Registered `TaskbarCreated` message (0 until registered).
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Shared data: what `list`/`get` and target resolution need to know.
// ---------------------------------------------------------------------------

/// The user-visible facts about one monitor.
struct Row {
    id: String,
    friendly: String,
    serial: Option<String>,
    alias: Option<String>,
    hmonitor: HMONITOR,
    color: Rgb,
    value: Option<u8>,
    /// Controllable (answered its first read).
    active: bool,
    /// Built-in (laptop) panel.
    internal: bool,
    /// Built-in panel left alone because `internal_display` is off.
    internal_disabled: bool,
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    #[cfg(feature = "plus")]
    device_name: String,
    /// HDR / advanced colour is on.
    #[cfg(feature = "plus")]
    hdr: bool,
}

impl Row {
    fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.friendly)
    }
}

/// A discovered monitor plus the backend to probe it with (`None`: not controllable).
struct Entry {
    row: Row,
    backend: Option<Backend>,
}

fn settings_of(config: &Config, id: &str) -> MonitorSettings {
    config.monitors.get(id).cloned().unwrap_or_default()
}

/// Turn enumerated monitors into rows in tray order (settings order, then left edge),
/// assigning palette colors to monitors without one (stored into `config`; the flag
/// says whether that changed it). Hidden and `skip`ped monitors get no backend; built-in
/// panels get WMI when `config.general.internal_display` is on, else none. A monitor is
/// built-in if DisplayConfig says so or if its device path matches one of the WMI
/// `wmi` instance names (which then also selects that WMI instance).
fn build_entries(
    config: &mut Config,
    mut found: Vec<FoundMonitor>,
    skip: &[String],
    wmi: &[String],
) -> (Vec<Entry>, bool) {
    found.sort_by_cached_key(|f| {
        (
            settings_of(config, &f.stable_id).order.unwrap_or(u32::MAX),
            f.rect.left,
        )
    });
    let mut used: Vec<Rgb> = found
        .iter()
        .filter_map(|f| settings_of(config, &f.stable_id).color)
        .collect();
    let mut changed = false;
    let mut entries = Vec::new();
    for mut f in found {
        let s = settings_of(config, &f.stable_id);
        let color = s.color.unwrap_or_else(|| {
            let c = next_palette_color(&used);
            used.push(c);
            config
                .monitors
                .entry(f.stable_id.clone())
                .or_default()
                .color = Some(c);
            changed = true;
            c
        });
        let key = display_key(&f.device_path);
        let matched = key
            .as_ref()
            .and_then(|k| wmi.iter().find(|n| display_key(n).as_ref() == Some(k)));
        log(&format!(
            "monitor id={} name={:?} path={} tech={} internal={} wmi={} ddc={}",
            f.stable_id,
            f.friendly_name,
            f.device_path,
            f.output_tech,
            f.internal,
            matched.is_some(),
            f.handle.is_some()
        ));
        f.internal |= matched.is_some();
        if f.internal {
            f.friendly_name = tr(pick_lang(config), Key::BuiltInDisplay).to_string();
        }
        let usable = !s.hidden && !skip.contains(&f.stable_id);
        let backend = match (usable, f.internal) {
            (false, _) => None,
            (true, false) => f.handle.take().map(Backend::Ddc),
            (true, true) => config
                .general
                .internal_display
                .then(|| Backend::Wmi(matched.cloned())),
        };
        entries.push(Entry {
            row: Row {
                id: f.stable_id,
                friendly: f.friendly_name,
                serial: f.serial,
                alias: s.alias,
                hmonitor: f.hmonitor,
                color,
                value: None,
                active: false,
                internal: f.internal,
                internal_disabled: usable && f.internal && !config.general.internal_display,
                #[cfg(feature = "plus")]
                device_name: f.device_name,
                #[cfg(feature = "plus")]
                hdr: f.advanced_color,
            },
            backend,
        });
    }
    (entries, changed)
}

/// `InstanceName`s WMI lists for active brightness-capable panels. Runs on the calling
/// thread (needs COM); one local query, about 10-50 ms, which is fine at scan time.
/// Skipped (WMI isn't even loaded) unless a built-in panel is possible: the setting is on,
/// or a monitor that DisplayConfig doesn't call built-in has no DDC/CI handle.
fn wmi_instances(internal_display: bool, found: &[FoundMonitor]) -> Vec<String> {
    if !internal_display && !found.iter().any(|f| !f.internal && f.handle.is_none()) {
        return Vec::new();
    }
    let found = Wmi::connect(None)
        .map(|w| w.instances())
        .unwrap_or_default();
    log(&format!("wmi instances: {found:?}"));
    found
}

fn describe(target: &Target) -> String {
    match target {
        Target::Default => "default".into(),
        Target::All => "all".into(),
        Target::Cursor => "cursor".into(),
        Target::Index(n) => format!("#{n}"),
        Target::Name(s) => s.clone(),
    }
}

fn cursor_monitor() -> HMONITOR {
    let mut pt = POINT::default();
    // SAFETY: `pt` is a valid out-pointer.
    unsafe {
        let _ = GetCursorPos(&mut pt);
        MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
    }
}

/// Indices of the rows a target refers to (empty: no match).
fn resolve(target: &Target, rows: &[&Row]) -> Vec<usize> {
    let all = 0..rows.len();
    match target {
        Target::Default => all
            .into_iter()
            .find(|&i| rows[i].active)
            .into_iter()
            .collect(),
        Target::All => all.filter(|&i| rows[i].active).collect(),
        Target::Cursor => {
            let h = cursor_monitor();
            all.filter(|&i| rows[i].active && rows[i].hmonitor == h)
                .collect()
        }
        Target::Index(n) if (1..=rows.len()).contains(n) => vec![n - 1],
        Target::Index(_) => Vec::new(),
        Target::Name(s) => resolve_name(&s.to_lowercase(), rows),
    }
}

/// Alias equals, else friendly-name substring, else serial or stable ID equals.
fn resolve_name(name: &str, rows: &[&Row]) -> Vec<usize> {
    let eq = |o: &Option<String>| o.as_deref().is_some_and(|v| v.to_lowercase() == name);
    let tiers: [&dyn Fn(&Row) -> bool; 3] = [
        &|r| eq(&r.alias),
        &|r| r.friendly.to_lowercase().contains(name),
        &|r| eq(&r.serial) || r.id.to_lowercase() == name,
    ];
    for tier in tiers {
        let hits: Vec<usize> = (0..rows.len()).filter(|&i| tier(rows[i])).collect();
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}

fn no_match(target: &Target) -> String {
    format!("No monitor matches '{}'", describe(target))
}

/// `list` and `get`: `(exit code, output text)`.
fn query(cmd: &Command, rows: &[&Row]) -> (i32, String) {
    match cmd {
        Command::List => {
            let lines: Vec<String> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| list_line(i + 1, r))
                .collect();
            (0, lines.join("\n"))
        }
        Command::Get(target) => {
            let value = resolve(target, rows)
                .into_iter()
                .find_map(|i| rows[i].value);
            match value {
                Some(v) => (i32::from(v), v.to_string()),
                None => (1, no_match(target)),
            }
        }
        _ => (0, String::new()),
    }
}

fn list_line(n: usize, r: &Row) -> String {
    let state = match (r.active, r.value) {
        (true, Some(v)) => format!("{v}%"),
        (true, None) if r.internal => "built-in (no response)".into(),
        (true, None) => "–".into(),
        (false, _) if r.internal_disabled => "built-in (disabled)".into(),
        (false, _) => "not controllable".into(),
    };
    format!(
        "#{n}  {}  {}  {state}  {}",
        r.name(),
        r.id,
        r.color.to_hex()
    )
}

fn pick_lang(config: &Config) -> Lang {
    Lang::from_setting(&config.general.language)
        .unwrap_or_else(|| Lang::from_locale(&util::ui_locale()))
}

/// The result of an update thread.
enum UpdateEvent {
    /// `manual`: the user asked, so every outcome is reported.
    Checked {
        manual: bool,
        result: Result<Release, String>,
    },
    /// The new exe is in place; the path to start it from.
    Installed(Result<PathBuf, InstallError>),
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Health {
    Ok,
    Unresponsive,
    ReadOnly,
}

/// A tray icon as last shown.
struct Rendered {
    spec: IconSpec,
    tip: String,
    icon: OwnedIcon,
}

struct Monitor {
    row: Row,
    /// Tray uID (1-based position in tray order).
    uid: u32,
    /// Identifies the worker's events; globally increasing per spawn.
    key: u64,
    /// `None` when not controllable.
    worker: Option<DdcWorker>,
    health: Health,
    /// The first event arrived.
    probed: bool,
    /// Its first probe failed (DDC/CI may not have been ready yet).
    probe_failed: bool,
    /// Wheel delta not yet worth a full notch.
    remainder: i32,
    /// When Nitor last changed this monitor's brightness itself.
    last_adjust: Option<Instant>,
    /// `Some` while a tray icon exists.
    icon: Option<Rendered>,
}

struct App {
    hwnd: HWND,
    path: PathBuf,
    config: Config,
    lang: Lang,
    theme: Theme,
    icon_size: u32,
    renderer: IconRenderer,
    monitors: Vec<Monitor>,
    placeholder: Option<Rendered>,
    /// Events from DDC workers, drained on `WM_APP_DDC`.
    queue: Arc<Mutex<Vec<(u64, DdcEvent)>>>,
    next_key: u64,
    /// Monitors that crashed the previous detection; skipped by the first scan only.
    safe_skip: Vec<String>,
    probe_done: bool,
    display_off: bool,
    /// Shown once the first probe completes (needs a tray icon).
    balloons: Vec<(String, String)>,
    /// CLI commands to apply after the first probe.
    initial: Vec<Command>,
    /// Events from update threads, drained on `WM_APP_UPDATE`.
    update_queue: Arc<Mutex<Vec<UpdateEvent>>>,
    /// A check or install thread is running.
    update_busy: bool,
    /// A newer release than the running version, if one was found.
    available: Option<Release>,
    /// The version last announced by balloon (once per version and session).
    announced: Option<String>,
    /// What Settings shows about the update check.
    update_status: UpdateStatus,
    /// The check of this launch has started (later ones follow the 24 h cadence).
    launch_checked: bool,
    /// `util::awake_time` at the last usage sample (kept to whole minutes, see `count_usage`).
    awake_mark: u64,
    /// Automatic re-scans done since the last manual rescan or successful probe.
    retry_step: usize,
    /// Nitor Plus automation engine.
    #[cfg(feature = "plus")]
    auto: auto::Engine,
    /// Nitor Plus colour temperature.
    #[cfg(feature = "plus")]
    color: auto::ColorEngine,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` on the app, unless it does not exist yet or is already borrowed (re-entrancy).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut app| app.as_mut().map(f))
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl App {
    fn new(
        hwnd: HWND,
        path: PathBuf,
        mut config: Config,
        renderer: IconRenderer,
        initial: Vec<Command>,
        updated: bool,
    ) -> App {
        let lang = pick_lang(&config);
        let app_name = || tr(lang, Key::AppName).to_string();
        let mut balloons = Vec::new();
        if updated {
            balloons.push((
                app_name(),
                tr_with(lang, Key::Updated, update::current_version()),
            ));
        }
        match autostart::fix_stale() {
            Some(StaleFix::Moved) => {
                balloons.push((app_name(), tr(lang, Key::AutostartFixed).to_string()));
            }
            Some(StaleFix::Switched) => balloons.push((
                app_name(),
                tr_with(
                    lang,
                    Key::AutostartSwitched,
                    &update::product_title("Nitor"),
                ),
            )),
            None => {}
        }
        config.general.autostart = autostart::is_enabled();
        let safe_skip = std::mem::take(&mut config.general.detection_in_progress);
        if !safe_skip.is_empty() {
            balloons.push((app_name(), tr(lang, Key::SafeModeText).to_string()));
        }
        App {
            hwnd,
            path,
            config,
            lang,
            theme: Theme::read(),
            icon_size: icon::tray_icon_size(),
            renderer,
            monitors: Vec::new(),
            placeholder: None,
            queue: Arc::default(),
            next_key: 1,
            safe_skip,
            probe_done: false,
            display_off: false,
            balloons,
            initial,
            update_queue: Arc::default(),
            update_busy: false,
            available: None,
            announced: None,
            update_status: UpdateStatus::Idle,
            launch_checked: false,
            retry_step: 0,
            awake_mark: util::awake_time(),
            #[cfg(feature = "plus")]
            auto: auto::Engine::new(hwnd),
            #[cfg(feature = "plus")]
            color: auto::ColorEngine::new(hwnd),
        }
    }

    /// Add the awake minutes since the last sample to `usage_minutes` (saved only when it
    /// changed). There is deliberately no timer for this: it is called from events that
    /// happen anyway (the hourly update tick, suspend/resume, probe completion, opening the
    /// flyout or Settings, shutdown), so it adds no CPU load or wake-ups; a reminder may
    /// therefore come a little after its threshold.
    fn count_usage(&mut self) {
        let (minutes, mark) = crate::support::take_minutes(self.awake_mark, util::awake_time());
        self.awake_mark = mark;
        if minutes > 0 {
            self.config.general.usage_minutes += minutes;
            self.save();
        }
    }

    /// Count usage, then show the support reminder if it is due, Windows accepts
    /// notifications now (else the next event tries again) and no popup is open.
    fn support_check(&mut self) {
        self.count_usage();
        let g = &self.config.general;
        if !self.probe_done
            || support::is_open()
            || !crate::support::due(g.support_reminders, g.usage_minutes, g.support_next_minutes)
            || !util::accepts_notifications()
        {
            return;
        }
        let anchor = self
            .balloon_uid()
            .and_then(|uid| tray::rect(self.hwnd, uid))
            .map(|r| POINT {
                x: (r.left + r.right) / 2,
                y: (r.top + r.bottom) / 2,
            });
        if support::show(anchor, self.lang) {
            log(&format!(
                "support reminder: shown at {} min of use",
                self.config.general.usage_minutes
            ));
        }
    }

    /// The popup closed: whatever the way, the next one is far away; "Don't remind me
    /// again" switches the reminders off.
    fn on_support_closed(&mut self, reason: support::Reason, dont_remind: bool) {
        self.count_usage();
        let g = &mut self.config.general;
        g.support_next_minutes = crate::support::next_after(g.usage_minutes);
        if dont_remind {
            g.support_reminders = false;
        }
        log(&format!(
            "support reminder: closed ({reason:?}){}",
            if dont_remind { ", reminders off" } else { "" }
        ));
        self.save();
        self.update_settings();
    }

    fn save(&self) {
        let _ = self.config.save(&self.path);
    }

    fn rows(&self) -> Vec<&Row> {
        self.monitors.iter().map(|m| &m.row).collect()
    }

    // --- scanning and probing ---------------------------------------------

    /// Drop workers and remove every tray icon.
    fn clear(&mut self) {
        for m in &self.monitors {
            if m.icon.is_some() {
                tray::remove(self.hwnd, m.uid);
            }
        }
        if self.placeholder.take().is_some() {
            tray::remove(self.hwnd, PLACEHOLDER_UID);
        }
        // Monitors and displays go back to their own colors before the workers stop.
        #[cfg(feature = "plus")]
        self.color
            .release(&color_views(&self.monitors, &self.config));
        self.monitors.clear();
        lock(&self.queue).clear();
        self.stop_wheel();
    }

    /// Enumerate monitors, record the probe in the config (crash safety, §7.4) and
    /// start one worker per probeable monitor.
    fn scan(&mut self) {
        self.clear();
        self.probe_done = false;
        let skip = std::mem::take(&mut self.safe_skip);
        let found = monitors::enumerate();
        let wmi = wmi_instances(self.config.general.internal_display, &found);
        let (entries, changed) = build_entries(&mut self.config, found, &skip, &wmi);
        for e in &entries {
            let backend = match &e.backend {
                Some(Backend::Ddc(_)) => "ddc".to_string(),
                Some(Backend::Wmi(i)) => format!("wmi {i:?}"),
                None => "none".to_string(),
            };
            log(&format!("backend id={} {backend}", e.row.id));
        }
        let probing: Vec<String> = entries
            .iter()
            .filter(|e| e.backend.is_some())
            .map(|e| e.row.id.clone())
            .collect();
        let any = !probing.is_empty();
        self.config.general.detection_in_progress = probing;
        if changed || any {
            self.save();
        }

        let interval = Duration::from_millis(self.config.general.write_interval_ms.into());
        for (i, e) in entries.into_iter().enumerate() {
            let key = self.next_key;
            self.next_key += 1;
            let worker = e.backend.map(|b| self.spawn_worker(key, b, interval));
            #[cfg(feature = "plus")]
            if let Some(w) = &worker {
                w.set_baseline(stored_gains(&self.config, &e.row.id));
            }
            self.monitors.push(Monitor {
                row: e.row,
                uid: i as u32 + 1,
                key,
                worker,
                health: Health::Ok,
                probed: false,
                probe_failed: false,
                remainder: 0,
                last_adjust: None,
                icon: None,
            });
        }
        for w in self.monitors.iter().filter_map(|m| m.worker.as_ref()) {
            w.read();
        }
        if !any {
            self.finish_probe();
        }
        self.update_settings();
    }

    fn spawn_worker(&self, key: u64, backend: Backend, interval: Duration) -> DdcWorker {
        let queue = self.queue.clone();
        // HWND is not Send; carry it as an integer.
        let hwnd = self.hwnd.0 as isize;
        let notify: Notify = Arc::new(move |key, event| {
            lock(&queue).push((key, event));
            // SAFETY: posting to a window handle is safe even if the window is gone.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut c_void)),
                    WM_APP_DDC,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
        let worker = DdcWorker::spawn(key, backend, interval, notify);
        worker.set_paused(self.display_off);
        worker
    }

    fn on_ddc(&mut self) {
        let events = std::mem::take(&mut *lock(&self.queue));
        for (key, event) in events {
            // Events of workers from an old scan are ignored.
            if let Some(i) = self.monitors.iter().position(|m| m.key == key) {
                self.apply_event(i, event);
            }
        }
        let all_probed = self.monitors.iter().all(|m| m.probed || m.worker.is_none());
        if !self.probe_done && all_probed {
            self.finish_probe();
        }
    }

    fn apply_event(&mut self, i: usize, event: DdcEvent) {
        #[cfg(feature = "plus")]
        if let DdcEvent::GainsReady(_) | DdcEvent::GainsUnavailable = event {
            let id = self.monitors[i].row.id.clone();
            // The first Hardware use of a monitor: keep its own gains for good.
            if let DdcEvent::GainsReady(Some(base)) = event {
                let s = self.config.monitors.entry(id.clone()).or_default();
                (s.ddc_gains, s.ddc_gains_max) = (Some(base.original), Some(base.max));
                self.save();
            }
            if self
                .color
                .on_gains(&id, matches!(event, DdcEvent::GainsReady(_)))
            {
                self.color_replan(false);
            }
            self.update_settings();
            return;
        }
        let m = &mut self.monitors[i];
        if m.worker.is_none() {
            return;
        }
        let first = !m.probed;
        m.probed = true;
        if first {
            log(&format!("probe id={} {event:?}", m.row.id));
        }
        if first && event == DdcEvent::Unresponsive && !m.row.internal {
            m.probe_failed = true;
            // Not controllable: no icon, release the backend. (A built-in panel keeps
            // a dimmed icon so the failure is visible.)
            m.worker = None;
            return;
        }
        m.row.active = true;
        match event {
            DdcEvent::Value(p) => {
                // A read queued before the user's latest change would undo it on screen.
                if !m
                    .last_adjust
                    .is_some_and(|t| t.elapsed() < OWN_CHANGE_WINDOW)
                {
                    m.row.value = Some(p);
                }
                if m.health != Health::ReadOnly {
                    m.health = Health::Ok;
                }
            }
            DdcEvent::Written(_) => m.health = Health::Ok,
            DdcEvent::Unresponsive => m.health = Health::Unresponsive,
            DdcEvent::ReadOnly => m.health = Health::ReadOnly,
            #[cfg(feature = "plus")]
            DdcEvent::GainsReady(_) | DdcEvent::GainsUnavailable => {}
        }
        self.refresh(i);
    }

    /// All monitors answered or failed: placeholder if needed, clear the crash marker,
    /// show pending balloons, then apply the CLI commands the process started with.
    fn finish_probe(&mut self) {
        self.probe_done = true;
        if self.monitors.iter().all(|m| !m.row.active) {
            self.refresh_placeholder();
        }
        self.config.general.detection_in_progress.clear();
        if !self.config.general.hint_shown {
            self.config.general.hint_shown = true;
            self.balloons.push((
                tr(self.lang, Key::FirstRunTitle).to_string(),
                tr(self.lang, Key::FirstRunText).to_string(),
            ));
        }
        self.save();

        if let Some(uid) = self.balloon_uid() {
            for (title, text) in self.balloons.drain(..) {
                tray::balloon(self.hwnd, uid, &title, &text, true);
            }
        }
        self.update_settings();
        self.plan_retry();
        self.maybe_auto_check();
        self.support_check();
        for cmd in std::mem::take(&mut self.initial) {
            self.exec(&cmd);
        }
        // Values are known now (startup, rescan, resume, display on): plan automation.
        #[cfg(feature = "plus")]
        self.auto_replan();
        util::trim_working_set();
    }

    /// Monitors that failed their first probe, or none controllable (DDC/CI often is not
    /// ready at logon): schedule another scan with backoff, until all answered or the
    /// schedule ends.
    fn plan_retry(&mut self) {
        let failed = self.monitors.iter().any(|m| m.probe_failed);
        let none = self.monitors.iter().all(|m| !m.row.active);
        // SAFETY: plain timer calls on our own window.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_RETRY);
            if !(failed || none) {
                self.retry_step = 0;
                return;
            }
            let Some(delay) = startup::retry_delay_ms(self.retry_step) else {
                log("probe: giving up automatic re-scans");
                return;
            };
            log(&format!(
                "probe: monitors not ready, re-scan #{} in {} s",
                self.retry_step + 1,
                delay / 1000
            ));
            self.retry_step += 1;
            SetTimer(Some(self.hwnd), TIMER_RETRY, delay, None);
        }
    }

    /// The tray icon a balloon can be attached to (none before the first icon exists).
    fn balloon_uid(&self) -> Option<u32> {
        self.monitors
            .iter()
            .find(|m| m.icon.is_some())
            .map(|m| m.uid)
            .or(self.placeholder.is_some().then_some(PLACEHOLDER_UID))
    }

    /// `quiet_time`: let Windows suppress it (automatic notices); the answer to a user's
    /// action passes `false`.
    fn balloon(&self, text: &str, quiet_time: bool) {
        if let Some(uid) = self.balloon_uid() {
            tray::balloon(
                self.hwnd,
                uid,
                tr(self.lang, Key::AppName),
                text,
                quiet_time,
            );
        }
    }

    /// The answer to a manual update action: Settings shows the status line itself, so
    /// only without Settings does it need a notification.
    fn answer_manual(&self, text: &str) {
        if !settings::is_open() {
            self.balloon(text, false);
        }
    }

    // --- updates ----------------------------------------------------------

    /// Run `job` on a thread and deliver its event to the UI thread.
    fn spawn_update_job(&mut self, job: impl FnOnce() -> UpdateEvent + Send + 'static) {
        self.update_busy = true;
        let queue = self.update_queue.clone();
        // HWND is not Send; carry it as an integer.
        let hwnd = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let event = job();
            lock(&queue).push(event);
            // SAFETY: posting to a window handle is safe even if the window is gone.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut c_void)),
                    WM_APP_UPDATE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
    }

    fn start_check(&mut self, manual: bool) {
        if self.update_busy {
            return;
        }
        log(&format!(
            "update check ({}): started",
            if manual { "manual" } else { "auto" }
        ));
        self.update_status = UpdateStatus::Checking;
        self.update_settings();
        self.spawn_update_job(move || UpdateEvent::Checked {
            manual,
            result: updater::check(),
        });
    }

    /// Start a quiet check if enabled: once per launch, then when 24 h passed since the last one.
    fn maybe_auto_check(&mut self) {
        let last = self.config.general.last_update_check;
        let now = now_secs();
        if !self.probe_done
            || !self.config.general.check_updates
            || self.update_busy
            || (self.launch_checked && !update::check_due(last, now))
        {
            return;
        }
        self.launch_checked = true;
        // Recorded when started, so a failing check is not retried every hour.
        self.config.general.last_update_check = now;
        self.save();
        self.start_check(false);
    }

    fn install_update(&mut self) {
        let Some(release) = self.available.clone() else {
            return;
        };
        if self.update_busy {
            return;
        }
        log(&format!("update install: {} started", release.tag));
        self.spawn_update_job(move || UpdateEvent::Installed(updater::install(&release)));
    }

    fn on_update(&mut self) {
        let events = std::mem::take(&mut *lock(&self.update_queue));
        for event in events {
            self.update_busy = false;
            match event {
                UpdateEvent::Checked { manual, result } => self.on_checked(manual, result),
                UpdateEvent::Installed(result) => self.on_installed(result),
            }
        }
    }

    fn on_checked(&mut self, manual: bool, result: Result<Release, String>) {
        let kind = if manual { "manual" } else { "auto" };
        match result {
            Ok(release) if update::is_newer(update::current_version(), release.version()) => {
                let version = release.version().to_string();
                log(&format!("update check ({kind}): {version} available"));
                let text = tr_with(self.lang, Key::UpdateAvailable, &version);
                if manual {
                    self.answer_manual(&text);
                } else if self.announced.as_deref() != Some(&version) {
                    self.balloon(&text, true);
                }
                self.announced = Some(version.clone());
                self.update_status = UpdateStatus::Available(version);
                self.available = Some(release);
            }
            Ok(release) => {
                log(&format!(
                    "update check ({kind}): up to date ({})",
                    release.tag
                ));
                self.available = None;
                let current = update::current_version();
                self.update_status = UpdateStatus::UpToDate(current.to_string());
                if manual {
                    self.answer_manual(&tr_with(self.lang, Key::UpToDate, current));
                }
            }
            Err(e) => {
                log(&format!("update check ({kind}): failed: {e}"));
                if manual {
                    self.answer_manual(&tr_with(self.lang, Key::UpdateFailed, &e));
                }
                self.update_status = UpdateStatus::Failed(e);
            }
        }
        self.update_settings();
    }

    fn on_installed(&mut self, result: Result<PathBuf, InstallError>) {
        let error = match result {
            Ok(exe) => match updater::restart(&exe) {
                // The new exe waits for this process to exit; close like Exit does.
                Ok(()) => {
                    log("update install: ok, restarting");
                    return self.on_menu(ID_EXIT);
                }
                Err(e) => e,
            },
            Err(InstallError::NotWritable) => {
                log("update install: folder not writable");
                String::new()
            }
            Err(InstallError::Failed(e)) => e,
        };
        if !error.is_empty() {
            log(&format!("update install: failed: {error}"));
            self.balloon(&tr_with(self.lang, Key::UpdateFailed, &error), false);
            self.update_status = UpdateStatus::Failed(error);
            self.update_settings();
        }
        // Nothing (more) to do automatically: offer the manual download.
        if let Some(release) = &self.available {
            util::open_url(&release.page);
        }
    }

    /// Reload the config from disk (manual edits apply) and scan again.
    fn rescan(&mut self) {
        // A rescan the user (or a system event) asked for starts the backoff afresh.
        self.retry_step = 0;
        self.rescan_now();
    }

    fn rescan_now(&mut self) {
        flyout::hide();
        // SAFETY: plain timer call on our own window.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_RESCAN);
        }
        if let Ok(config) = Config::load(&self.path) {
            self.config = config;
            self.config.general.autostart = autostart::is_enabled();
            self.lang = pick_lang(&self.config);
        }
        self.scan();
    }

    /// (Re)start the debounce timer for a rescan.
    fn schedule_rescan(&self, delay_ms: u32) {
        // SAFETY: plain timer call on our own window. Windows clamps tiny delays.
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_RESCAN, delay_ms, None);
        }
    }

    fn rescan_soon(&self) {
        self.schedule_rescan(self.config.general.rescan_delay_ms);
    }

    // --- tray icons -------------------------------------------------------

    fn spec_for(&self, m: &Monitor) -> IconSpec {
        let unresponsive = m.health == Health::Unresponsive;
        let content = match m.row.value {
            Some(v) if !unresponsive => IconContent::Value(v),
            _ => IconContent::Dash,
        };
        IconSpec {
            content,
            color: m.row.color,
            style: self.config.general.tray_style,
            taskbar_bg: self.theme.taskbar_bg(),
            badge: settings_of(&self.config, &m.row.id).badge,
            dimmed: unresponsive,
            size: self.icon_size,
        }
    }

    fn tooltip_for(&self, m: &Monitor) -> String {
        let name = m.row.name();
        let value = m.row.value.map_or("–".to_string(), |v| format!("{v}%"));
        match m.health {
            Health::Ok => match m.row.value {
                Some(_) => format!("{name} — {value}"),
                None => name.to_string(),
            },
            Health::Unresponsive if m.row.internal && m.row.value.is_none() => {
                format!("{name} — {}", tr(self.lang, Key::InternalNoResponse))
            }
            Health::Unresponsive => format!("{name} — {}", tr(self.lang, Key::Unresponsive)),
            Health::ReadOnly => format!(
                "{name} — {value} ({})\n{}",
                tr(self.lang, Key::ReadOnly),
                tr(self.lang, Key::ReadOnlyHint)
            ),
        }
    }

    /// Re-render monitor `i`'s icon if its look or tooltip changed (adds it if new).
    fn refresh(&mut self, i: usize) {
        let m = &self.monitors[i];
        if !m.row.active {
            return;
        }
        let (spec, tip) = (self.spec_for(m), self.tooltip_for(m));
        let m = &mut self.monitors[i];
        present(self.hwnd, &self.renderer, m.uid, &mut m.icon, spec, tip);
        self.update_flyout();
        self.update_settings();
    }

    fn refresh_placeholder(&mut self) {
        let spec = IconSpec {
            content: IconContent::Sun,
            color: PLACEHOLDER_COLOR,
            style: self.config.general.tray_style,
            taskbar_bg: self.theme.taskbar_bg(),
            badge: None,
            dimmed: true,
            size: self.icon_size,
        };
        let tip = tr(self.lang, Key::NoMonitors).to_string();
        present(
            self.hwnd,
            &self.renderer,
            PLACEHOLDER_UID,
            &mut self.placeholder,
            spec,
            tip,
        );
    }

    fn refresh_all(&mut self) {
        for i in 0..self.monitors.len() {
            self.refresh(i);
        }
        if self.placeholder.is_some() {
            self.refresh_placeholder();
        }
    }

    /// Explorer restarted: add every icon again.
    fn readd_icons(&mut self) {
        let shown = self
            .monitors
            .iter()
            .filter_map(|m| m.icon.as_ref().map(|r| (m.uid, r)))
            .chain(self.placeholder.as_ref().map(|r| (PLACEHOLDER_UID, r)));
        for (uid, r) in shown {
            tray::add(self.hwnd, uid, r.icon.0, &r.tip);
        }
    }

    // --- theme and DPI ----------------------------------------------------

    fn on_theme_change(&mut self, colors_changed: bool) {
        if colors_changed {
            self.theme = Theme::read();
            flyout::retheme();
            support::retheme();
            theme::set_menu_theme(self.theme.taskbar_light);
        }
        self.icon_size = icon::tray_icon_size();
        self.refresh_all();
    }

    // --- tray events and wheel -------------------------------------------

    /// Cache icon rectangles and install the wheel hook while the cursor is over an icon.
    fn arm_wheel(&self) {
        if wheel::is_armed() {
            return;
        }
        let rects: Vec<_> = self
            .monitors
            .iter()
            .filter(|m| m.icon.is_some())
            .filter_map(|m| tray::rect(self.hwnd, m.uid).map(|r| (m.uid, r)))
            .collect();
        wheel::arm(self.hwnd, &rects);
        // SAFETY: plain timer call on our own window.
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_WHEEL, WHEEL_POLL_MS, None);
        }
    }

    fn stop_wheel(&self) {
        wheel::disarm();
        // SAFETY: plain timer call on our own window.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_WHEEL);
        }
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_WHEEL => {
                let mut pt = POINT::default();
                // SAFETY: `pt` is a valid out-pointer.
                let _ = unsafe { GetCursorPos(&mut pt) };
                if !wheel::contains(pt) {
                    self.stop_wheel();
                }
            }
            TIMER_RESCAN => self.rescan(),
            TIMER_RETRY => {
                // SAFETY: plain timer call on our own window (one-shot).
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_RETRY);
                }
                self.rescan_now();
            }
            TIMER_START => {
                // SAFETY: plain timer call on our own window (one-shot).
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_START);
                }
                self.scan();
            }
            TIMER_UPDATE => {
                self.support_check();
                self.maybe_auto_check();
            }
            #[cfg(feature = "plus")]
            auto::TIMER_AUTO => self.auto_replan(),
            #[cfg(feature = "plus")]
            auto::TIMER_SENSOR => self.auto_sensor_settled(),
            #[cfg(feature = "plus")]
            auto::TIMER_COLOR => self.color_replan(false),
            _ => {}
        }
    }

    fn on_wheel(&mut self, ev: WheelEvent) {
        let Some(i) = self
            .monitors
            .iter()
            .position(|m| m.uid == ev.uid && m.row.active)
        else {
            return;
        };
        if self.monitors[i].row.value.is_none() {
            return;
        }
        let s = settings_of(&self.config, &self.monitors[i].row.id);
        let step = if ev.shift {
            1
        } else {
            i32::from(s.step.unwrap_or(self.config.general.wheel_step))
        };
        let m = &mut self.monitors[i];
        m.remainder += ev.delta;
        let notches = m.remainder / WHEEL_DELTA;
        m.remainder -= notches * WHEEL_DELTA;
        if notches != 0 {
            let delta = (notches * step).clamp(-100, 100) as i16;
            self.adjust(i, BrightnessOp::Add(delta));
        }
    }

    /// A change by the user (wheel, flyout, CLI): apply `op` to monitor `i`; with Plus it
    /// also holds automation off for that monitor.
    fn adjust(&mut self, i: usize, op: BrightnessOp) {
        let changed = self.set_value(i, op);
        #[cfg(feature = "plus")]
        if changed {
            self.auto
                .user_change(&self.monitors[i].row.id, &self.config.auto);
        }
        #[cfg(not(feature = "plus"))]
        let _ = changed;
    }

    /// Apply `op` to monitor `i`: update the value, redraw immediately, queue the DDC write.
    /// True if the value changed. (Automation calls this directly: not a user override.)
    fn set_value(&mut self, i: usize, op: BrightnessOp) -> bool {
        let s = settings_of(&self.config, &self.monitors[i].row.id);
        let m = &mut self.monitors[i];
        let (Some(current), Some(worker)) = (m.row.value, &m.worker) else {
            return false;
        };
        let new = op.apply(current, s.min, s.max);
        if new == current {
            return false;
        }
        m.row.value = Some(new);
        m.last_adjust = Some(Instant::now());
        worker.write(new);
        self.refresh(i);
        true
    }

    /// Left click: read every monitor again.
    fn read_all(&self) {
        for m in self.monitors.iter().filter(|m| m.row.active) {
            if let Some(w) = &m.worker {
                w.read();
            }
        }
    }

    // --- flyout -----------------------------------------------------------

    /// The flyout's rows: active monitors in tray order.
    fn flyout_rows(&self) -> Vec<FlyoutRow> {
        self.monitors
            .iter()
            .filter(|m| m.row.active)
            .map(|m| {
                let s = settings_of(&self.config, &m.row.id);
                FlyoutRow {
                    uid: m.uid,
                    name: m.row.name().to_string(),
                    color: m.row.color,
                    value: m.row.value,
                    enabled: m.health != Health::Unresponsive,
                    link_min: s.link_min,
                    link_max: s.link_max,
                }
            })
            .collect()
    }

    /// Push changed values/colors to an open flyout.
    fn update_flyout(&self) {
        if flyout::is_visible() {
            flyout::update(
                self.flyout_rows(),
                #[cfg(feature = "plus")]
                self.flyout_color(),
            );
        }
    }

    /// Left click on a tray icon: close the flyout if open, else open it at `anchor`.
    fn toggle_flyout(&mut self, anchor: POINT) {
        self.count_usage();
        if flyout::is_visible() {
            flyout::hide();
        } else if self.monitors.iter().any(|m| m.row.active) {
            if self.config.general.refresh_on_open {
                self.read_all();
            }
            let linked = self.config.general.linked;
            flyout::show(
                anchor,
                self.flyout_rows(),
                linked,
                self.lang,
                #[cfg(feature = "plus")]
                self.flyout_color(),
            );
        }
    }

    fn monitor_by_uid(&self, uid: u32) -> Option<usize> {
        self.monitors
            .iter()
            .position(|m| m.uid == uid && m.row.active)
    }

    fn set_color(&mut self, i: usize, color: Rgb) {
        let m = &mut self.monitors[i];
        m.row.color = color;
        let id = m.row.id.clone();
        self.config.monitors.entry(id).or_default().color = Some(color);
        self.refresh(i);
    }

    /// Put monitor `i` where the master slider at `master` places it (its calibrated range).
    fn apply_master(&mut self, i: usize, master: u8) {
        let s = settings_of(&self.config, &self.monitors[i].row.id);
        self.adjust(
            i,
            BrightnessOp::Set(link_value(master, s.link_min, s.link_max)),
        );
    }

    fn on_flyout(&mut self, action: Action) {
        match action {
            Action::SetBrightness(uid, v) => {
                if let Some(i) = self.monitor_by_uid(uid) {
                    self.adjust(i, BrightnessOp::Set(v));
                }
            }
            Action::SetAll(master) => {
                for i in 0..self.monitors.len() {
                    if self.monitors[i].row.active {
                        self.apply_master(i, master);
                    }
                }
            }
            Action::SetColor(uid, color) => {
                if let Some(i) = self.monitor_by_uid(uid) {
                    self.set_color(i, color);
                    self.save();
                }
            }
            Action::SetLinked(linked) => {
                self.config.general.linked = linked;
                self.save();
            }
            Action::SetLinkRange(uid, lo, hi) => {
                if let Some(i) = self.monitor_by_uid(uid) {
                    let id = self.monitors[i].row.id.clone();
                    let s = self.config.monitors.entry(id).or_default();
                    (s.link_min, s.link_max) = (lo, hi);
                    self.save();
                    // Show the effect of the new range right away.
                    self.apply_master(i, flyout::master());
                }
            }
            #[cfg(feature = "plus")]
            Action::PreviewKelvin(k) => self
                .color
                .preview(&color_views(&self.monitors, &self.config), k),
            #[cfg(feature = "plus")]
            Action::SetKelvin(k) => self.set_manual_kelvin(k),
            Action::Rescan => self.rescan(),
            Action::Settings => self.open_settings(),
        }
        // A value clamped by the monitor's min/max may not have changed anything above.
        self.update_flyout();
    }

    // --- menu -------------------------------------------------------------

    /// Open the Settings window (or bring it to the front).
    fn open_settings(&mut self) {
        self.count_usage();
        settings::open(self.settings_snapshot());
    }

    /// Open `Nitor.ini` in the default editor.
    fn open_config(&mut self) {
        if !self.path.exists() {
            self.save();
        }
        util::open_with_default_app(&self.path);
    }

    fn settings_snapshot(&self) -> Snapshot {
        let monitors = self
            .monitors
            .iter()
            .map(|m| {
                let state = if settings_of(&self.config, &m.row.id).hidden {
                    MonitorState::Hidden
                } else if m.row.internal_disabled {
                    MonitorState::BuiltInOff
                } else if m.worker.is_none() {
                    MonitorState::NotControllable
                } else {
                    match m.health {
                        Health::Ok => MonitorState::Active,
                        Health::Unresponsive => MonitorState::Unresponsive,
                        Health::ReadOnly => MonitorState::ReadOnly,
                    }
                };
                MonitorInfo {
                    id: m.row.id.clone(),
                    friendly: m.row.friendly.clone(),
                    color: m.row.color,
                    state,
                    internal: m.row.internal,
                    #[cfg(feature = "plus")]
                    color_method: self.color.method(&m.row.id),
                    #[cfg(feature = "plus")]
                    color_unsupported: self.color.unsupported(&m.row.id),
                    #[cfg(feature = "plus")]
                    hdr: m.row.hdr,
                }
            })
            .collect();
        Snapshot {
            config: self.config.clone(),
            lang: self.lang,
            update: self.update_status.clone(),
            config_path: self.path.display().to_string(),
            log_path: self.path.with_file_name("Nitor.log").display().to_string(),
            monitors,
            #[cfg(feature = "plus")]
            sensor_available: auto::sensor_available(),
            #[cfg(feature = "plus")]
            weather_status: self.auto.weather_status(),
            // Only a built-in panel can clash with Windows' adaptive brightness.
            #[cfg(feature = "plus")]
            adaptive_conflict: self.config.auto.enabled
                && self.monitors.iter().any(|m| m.row.internal && m.row.active)
                && auto::adaptive_brightness_on(),
            #[cfg(feature = "plus")]
            warm_range_ok: auto::gamma_range_extended(),
            #[cfg(feature = "plus")]
            night_light: self.config.auto.color_enabled && auto::night_light_active(),
        }
    }

    /// Push the current state to an open Settings window.
    fn update_settings(&self) {
        if settings::is_open() {
            settings::update(self.settings_snapshot());
        }
    }

    /// A change made in the Settings window: apply it, save it, show the result.
    fn on_settings(&mut self, action: settings::Action) {
        use settings::Action as A;
        match action {
            A::Autostart(on) => {
                let _ = autostart::set(on);
                self.config.general.autostart = autostart::is_enabled();
            }
            A::Language(lang) => {
                self.config.general.language = lang.map_or("auto", Lang::as_setting).into();
                self.lang = pick_lang(&self.config);
                self.refresh_all();
            }
            A::InternalDisplay(on) => {
                self.config.general.internal_display = on;
                self.save();
                self.scan();
            }
            A::CheckUpdates(on) => self.config.general.check_updates = on,
            A::CheckNow => self.start_check(true),
            A::Install => self.install_update(),
            A::OpenLog => {
                // Open Nitor.log itself, like the config file (created empty if missing).
                let log = self.path.with_file_name("Nitor.log");
                if !log.exists() {
                    let _ = std::fs::write(&log, b"");
                }
                util::open_with_default_app(&log);
            }
            A::TrayStyle(style) => {
                self.config.general.tray_style = style;
                self.refresh_all();
            }
            A::WheelStep(step) => self.config.general.wheel_step = step,
            A::RefreshOnOpen(on) => self.config.general.refresh_on_open = on,
            A::SupportReminders(on) => self.config.general.support_reminders = on,
            A::WriteInterval(ms) => {
                self.config.general.write_interval_ms = ms;
                let interval = Duration::from_millis(ms.into());
                for w in self.monitors.iter().filter_map(|m| m.worker.as_ref()) {
                    w.set_write_interval(interval);
                }
            }
            A::OpenConfig => self.open_config(),
            A::Monitor(id, edit) => self.edit_monitor(&id, edit),
            #[cfg(feature = "plus")]
            A::Auto(edit) => self.edit_auto(edit),
        }
        self.save();
        // Automation settings (or limits, ranges) may have changed: apply them now.
        #[cfg(feature = "plus")]
        self.replan(true);
        self.update_settings();
    }

    fn edit_monitor(&mut self, id: &str, edit: MonitorEdit) {
        let Some(i) = self.monitors.iter().position(|m| m.row.id == id) else {
            return;
        };
        let entry = self.config.monitors.entry(id.to_string()).or_default();
        match edit {
            MonitorEdit::Color(color) => self.set_color(i, color),
            #[cfg(feature = "plus")]
            MonitorEdit::ColorMethod(method) => {
                entry.color_method = method;
                if method == ColorMethodPref::Ddc {
                    let ddc = self.monitors[i].worker.as_ref();
                    self.color.select_hardware(id, ddc);
                }
            }
            #[cfg(feature = "plus")]
            MonitorEdit::RestoreColors => {
                entry.color_method = ColorMethodPref::Gamma;
                let ddc = self.monitors[i].worker.as_ref();
                self.color.restore_monitor(id, ddc);
            }
            MonitorEdit::Alias(alias) => {
                entry.alias = alias.clone();
                self.monitors[i].row.alias = alias;
                self.refresh(i);
            }
            MonitorEdit::Badge(badge) => {
                entry.badge = badge;
                self.refresh(i);
            }
            MonitorEdit::Limits(min, max) => (entry.min, entry.max) = (min, max),
            MonitorEdit::Shown(shown) => {
                entry.hidden = !shown;
                self.save();
                flyout::hide();
                self.scan();
            }
            MonitorEdit::MoveUp | MonitorEdit::MoveDown => {
                let j = if edit == MonitorEdit::MoveUp {
                    i.checked_sub(1)
                } else {
                    Some(i + 1).filter(|&j| j < self.monitors.len())
                };
                let Some(j) = j else { return };
                let mut ids: Vec<String> = self.monitors.iter().map(|m| m.row.id.clone()).collect();
                ids.swap(i, j);
                for (k, id) in ids.into_iter().enumerate() {
                    self.config.monitors.entry(id).or_default().order = Some(k as u32 + 1);
                }
                self.save();
                flyout::hide();
                self.scan();
            }
        }
    }

    fn on_menu(&mut self, cmd: u32) {
        match cmd {
            ID_RESCAN => self.rescan(),
            ID_SETTINGS => self.open_settings(),
            ID_INTERNAL => {
                self.config.general.internal_display = !self.config.general.internal_display;
                self.save();
                self.scan();
            }
            ID_CHECK_UPDATES => self.start_check(true),
            ID_UPDATE => self.install_update(),
            #[cfg(feature = "plus")]
            ID_AUTO_TOGGLE => {
                self.auto.toggle_pause();
                self.replan(true);
            }
            ID_AUTOSTART => {
                let _ = autostart::set(!autostart::is_enabled());
                self.config.general.autostart = autostart::is_enabled();
                self.save();
            }
            ID_EXIT => {
                // SAFETY: posting to our own window.
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            _ => {}
        }
        self.update_settings();
    }

    // --- power ------------------------------------------------------------

    /// The built-in brightness changed outside Nitor (Fn keys, Quick Settings):
    /// show the new value without writing it back.
    fn on_internal_brightness(&mut self, percent: u8) {
        let Some(i) = self
            .monitors
            .iter()
            .position(|m| m.row.internal && m.worker.is_some() && m.row.active)
        else {
            return;
        };
        // Windows also reports Nitor's own writes; while the user is adjusting, those
        // echoes would make the value jump back.
        if self.monitors[i]
            .last_adjust
            .is_some_and(|t| t.elapsed() < OWN_CHANGE_WINDOW)
        {
            return;
        }
        // A change Nitor did not make (Fn keys, Windows adaptive brightness) is the user's
        // word: automation steps back. Echoes of our own value equal the known one.
        #[cfg(feature = "plus")]
        if self.monitors[i]
            .row
            .value
            .is_some_and(|v| v.abs_diff(percent) >= 2)
        {
            self.auto
                .user_change(&self.monitors[i].row.id, &self.config.auto);
        }
        self.monitors[i].row.value = Some(percent.min(100));
        // Resync the worker's idea of the panel so the next wheel step is not skipped.
        if let Some(w) = &self.monitors[i].worker {
            w.read();
        }
        self.refresh(i);
    }

    fn on_display_state(&mut self, off: bool) {
        if off == self.display_off {
            return;
        }
        self.display_off = off;
        for w in self.monitors.iter().filter_map(|m| m.worker.as_ref()) {
            w.set_paused(off);
        }
        if !off {
            self.rescan_soon();
        }
    }

    // --- commands (CLI) ---------------------------------------------------

    /// Run commands in order; the result is the last exit code and all output lines.
    fn exec_all(&mut self, commands: &[Command]) -> (i32, String) {
        let mut code = 0;
        let mut lines = Vec::new();
        for cmd in commands {
            let (c, text) = self.exec(cmd);
            code = c;
            if !text.is_empty() {
                lines.push(text);
            }
        }
        (code, lines.join("\n"))
    }

    fn exec(&mut self, cmd: &Command) -> (i32, String) {
        match cmd {
            Command::List | Command::Get(_) => query(cmd, &self.rows()),
            Command::Brightness { target, op } => {
                self.for_target(target, |app, i| app.adjust(i, *op))
            }
            Command::Color { target, color } => {
                let result = self.for_target(target, |app, i| app.set_color(i, *color));
                self.save();
                result
            }
            Command::Rescan => {
                self.schedule_rescan(1);
                (0, String::new())
            }
            Command::Quit => {
                // SAFETY: posting to our own window.
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                (0, String::new())
            }
        }
    }

    /// Run `f` for every monitor `target` resolves to; exit code 2 if there is none.
    fn for_target(&mut self, target: &Target, mut f: impl FnMut(&mut App, usize)) -> (i32, String) {
        let hits = resolve(target, &self.rows());
        if hits.is_empty() {
            return (2, no_match(target));
        }
        for i in hits {
            f(self, i);
        }
        (0, String::new())
    }

    /// `WM_DESTROY`: remove icons, stop workers, save.
    fn shutdown(&mut self) {
        self.count_usage();
        if support::close() {
            let g = &mut self.config.general;
            g.support_next_minutes = crate::support::next_after(g.usage_minutes);
        }
        self.clear();
        self.config.general.detection_in_progress.clear();
        self.save();
    }
}

/// Nitor Plus automation glue; the logic lives in `super::auto` and `crate::auto`.
#[cfg(feature = "plus")]
impl App {
    /// Plan automation for the current monitors and config, and apply the writes (as
    /// automatic changes: they are not user overrides). Also the hook that makes settings
    /// edits (`config.auto`) take effect: `on_settings` calls it after saving.
    fn auto_replan(&mut self) {
        self.replan(false);
    }

    /// `auto_replan`, for a change the user just made (`user`): the monitors' gains take it
    /// at once instead of at the pace that spares their memory.
    fn replan(&mut self, user: bool) {
        let views: Vec<MonitorView> = self
            .monitors
            .iter()
            .enumerate()
            .filter(|(_, m)| m.row.active && m.worker.is_some())
            .filter_map(|(index, m)| {
                let s = settings_of(&self.config, &m.row.id);
                Some(MonitorView {
                    index,
                    id: m.row.id.clone(),
                    value: m.row.value?,
                    link: (s.link_min, s.link_max),
                })
            })
            .collect();
        for (i, value) in self.auto.plan(&self.config, &views) {
            self.set_value(i, BrightnessOp::Set(value));
        }
        self.color_replan(user);
    }

    /// Plan the colour temperature (once the first probe is done). Pausing automation or
    /// switching it off puts the colors back to neutral.
    fn color_replan(&mut self, user: bool) {
        if !self.probe_done {
            return;
        }
        self.color.plan(
            &self.config,
            &color_views(&self.monitors, &self.config),
            self.auto.pause_until(),
            user,
        );
    }

    /// The flyout's color slider: it starts at the temperature the displays show, and its
    /// warm end is the Settings floor. `None` hides it, as no display could take a color
    /// change (all in HDR).
    fn flyout_color(&self) -> Option<flyout::ColorRow> {
        self.monitors
            .iter()
            .any(|m| m.row.active && !m.row.hdr)
            .then(|| flyout::ColorRow {
                kelvin: self.color.current(),
                min_k: color_floor(self.config.auto.experimental_warm),
            })
    }

    /// The user chose a temperature in the flyout. While color is automatic it holds the
    /// plan off like a manual brightness change; otherwise it is the fixed temperature
    /// (neutral at 6500 K), kept in the config.
    fn set_manual_kelvin(&mut self, kelvin: u32) {
        if self.config.auto.color_enabled {
            self.color.hold(&self.config, kelvin);
        } else {
            self.config.auto.manual_kelvin = (kelvin < MAX_K).then_some(kelvin as u16);
            self.save();
        }
        self.color_replan(true);
    }

    /// A change from the Settings window; `on_settings` saves and replans afterwards.
    fn edit_auto(&mut self, edit: settings::AutoEdit) {
        use settings::AutoEdit as E;
        let a = &mut self.config.auto;
        match edit {
            E::Enabled(on) => a.enabled = on,
            E::Day(v) => a.day_brightness = v.min(100),
            E::Night(v) => a.night_brightness = v.min(100),
            E::UseSensor(on) => a.use_sensor = on,
            E::ColorEnabled(on) => a.color_enabled = on,
            E::DayKelvin(k) => a.day_kelvin = k.clamp(color_floor(a.experimental_warm), MAX_K),
            E::NightKelvin(k) => a.night_kelvin = k.clamp(color_floor(a.experimental_warm), MAX_K),
            E::Warm(on) => {
                a.experimental_warm = on;
                let floor = color_floor(on);
                (a.day_kelvin, a.night_kelvin) =
                    (a.day_kelvin.max(floor), a.night_kelvin.max(floor));
            }
            E::WarmRange => auto::request_warm_range(self.hwnd),
            E::Weather(on) => a.weather = on,
            E::Override(mode) => a.override_mode = mode,
            E::Source(source) => a.location_source = source,
            E::Latitude(v) => a.latitude = Some(v),
            E::Longitude(v) => a.longitude = Some(v),
            E::Place(lat, lon) => {
                (a.latitude, a.longitude) = (Some(lat), Some(lon));
                a.location_source = LocationSource::Manual;
            }
        }
    }

    fn auto_sensor(&mut self) {
        if self.auto.on_sensor() {
            self.auto_replan();
        }
    }

    fn auto_sensor_settled(&mut self) {
        if self.auto.on_sensor_settle() {
            self.auto_replan();
        }
    }

    /// A weather fetch finished: plan with the new forecast and refresh the Settings status.
    fn auto_weather(&mut self) {
        self.auto.on_weather();
        self.auto_replan();
        self.update_settings();
    }

    /// Store a location found by Windows into the config.
    fn auto_location(&mut self) {
        if let Some((lat, lon)) = self.auto.on_location(&self.config.auto) {
            let a = &mut self.config.auto;
            if (a.latitude, a.longitude) != (Some(lat), Some(lon)) {
                (a.latitude, a.longitude) = (Some(lat), Some(lon));
                self.save();
                self.update_settings();
            }
        }
        self.auto_replan();
    }
}

/// The colour engine's view of the monitors.
#[cfg(feature = "plus")]
fn color_views<'a>(monitors: &'a [Monitor], config: &Config) -> Vec<auto::ColorView<'a>> {
    monitors
        .iter()
        .map(|m| auto::ColorView {
            id: &m.row.id,
            device_name: &m.row.device_name,
            // External monitors only: built-in panels (WMI) take the gamma path.
            ddc: m.worker.as_ref().filter(|_| !m.row.internal),
            hdr: m.row.hdr,
            pref: config
                .monitors
                .get(&m.row.id)
                .map(|s| s.color_method)
                .unwrap_or_default(),
        })
        .collect()
}

/// The monitor's own gains saved in the config, if both they and their maxima are there.
#[cfg(feature = "plus")]
fn stored_gains(config: &Config, id: &str) -> Option<GainBaseline> {
    let s = config.monitors.get(id)?;
    Some(GainBaseline {
        original: s.ddc_gains?,
        max: s.ddc_gains_max?,
    })
}

/// Warmest temperature the sliders accept.
#[cfg(feature = "plus")]
fn color_floor(experimental_warm: bool) -> u32 {
    if experimental_warm {
        UI_MIN_K
    } else {
        WARM_LIMIT_K
    }
}

/// Render `spec` and show it as tray icon `uid`, adding the icon the first time.
/// Does nothing if neither the picture nor the tooltip changed.
fn present(
    hwnd: HWND,
    renderer: &IconRenderer,
    uid: u32,
    slot: &mut Option<Rendered>,
    spec: IconSpec,
    tip: String,
) {
    let same_spec = slot.as_ref().is_some_and(|r| r.spec == spec);
    if same_spec && slot.as_ref().is_some_and(|r| r.tip == tip) {
        return;
    }
    let added = slot.is_some();
    let icon = if same_spec {
        slot.take().map(|r| r.icon)
    } else {
        renderer.render(&spec).ok()
    };
    let Some(icon) = icon else { return };
    if !(added && tray::modify(hwnd, uid, icon.0, &tip)) {
        tray::add(hwnd, uid, icon.0, &tip);
    }
    // The previous OwnedIcon (if any) is destroyed here, after the shell switched over.
    *slot = Some(Rendered { spec, tip, icon });
}

// ---------------------------------------------------------------------------
// Window procedure
// ---------------------------------------------------------------------------

/// Called by the flyout's window procedure (the app is not borrowed there).
fn on_flyout_action(action: Action) {
    with_app(|a| a.on_flyout(action));
}

/// Called by the Settings window's window procedure (the app is not borrowed there).
fn on_settings_action(action: settings::Action) {
    with_app(|a| a.on_settings(action));
}

/// Called by the support popup's window procedure (the app is not borrowed there).
fn on_support_closed(reason: support::Reason, dont_remind: bool) {
    with_app(|a| a.on_support_closed(reason, dont_remind));
}

fn on_tray(hwnd: HWND, ev: TrayEvent) {
    match ev.msg {
        WM_MOUSEMOVE => {
            with_app(|a| a.arm_wheel());
        }
        NIN_SELECT | NIN_KEYSELECT => {
            with_app(|a| a.toggle_flyout(POINT { x: ev.x, y: ev.y }));
        }
        WM_CONTEXTMENU => {
            let Some((lang, internal, update_to)) = with_app(|a| {
                // Offer the built-in toggle only if the last scan found such a panel.
                let found = a.monitors.iter().any(|m| m.row.internal);
                (
                    a.lang,
                    found.then_some(a.config.general.internal_display),
                    a.available.as_ref().map(|r| r.version().to_string()),
                )
            }) else {
                return;
            };
            #[cfg(feature = "plus")]
            let auto_pause = with_app(|a| {
                (a.config.auto.enabled || a.config.auto.color_enabled).then(|| a.auto.paused())
            })
            .flatten();
            // The menu runs a nested message loop: the app must not be borrowed meanwhile.
            let menu = MenuState {
                lang,
                autostart: autostart::is_enabled(),
                internal,
                update_to,
                #[cfg(feature = "plus")]
                auto_pause,
            };
            let cmd = show_menu(hwnd, ev.x, ev.y, &menu);
            with_app(|a| a.on_menu(cmd));
        }
        _ => {}
    }
}

/// What the tray context menu shows.
struct MenuState {
    lang: Lang,
    autostart: bool,
    /// `Some(checked)` when the "built-in display" item should be shown.
    internal: Option<bool>,
    /// Version offered by the "Update to" item, if a newer release is known.
    update_to: Option<String>,
    /// Plus with automation (brightness or color) on: `Some(paused)` shows "Pause automation" / "Resume automation".
    #[cfg(feature = "plus")]
    auto_pause: Option<bool>,
}

/// Show the tray context menu at a screen point and return the chosen command id (0: none).
fn show_menu(hwnd: HWND, x: i32, y: i32, state: &MenuState) -> u32 {
    let lang = state.lang;
    // SAFETY: standard menu calls on a menu we create and destroy here; every text buffer
    // outlives the AppendMenuW call that copies it.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let add_text = |flags: MENU_ITEM_FLAGS, id: u32, text: &str| {
            let text = util::wide(text);
            let _ = AppendMenuW(menu, MF_STRING | flags, id as usize, PCWSTR(text.as_ptr()));
        };
        let add = |flags: MENU_ITEM_FLAGS, id: u32, key: Key| add_text(flags, id, tr(lang, key));
        add(MENU_ITEM_FLAGS(0), ID_RESCAN, Key::MenuRescan);
        add(MENU_ITEM_FLAGS(0), ID_SETTINGS, Key::MenuSettings);
        let checked = if state.autostart {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        add(checked, ID_AUTOSTART, Key::MenuStartWithWindows);
        if let Some(on) = state.internal {
            let checked = if on { MF_CHECKED } else { MENU_ITEM_FLAGS(0) };
            add(checked, ID_INTERNAL, Key::MenuInternalDisplay);
        }
        #[cfg(feature = "plus")]
        if let Some(paused) = state.auto_pause {
            let key = if paused {
                Key::MenuResumeAuto
            } else {
                Key::MenuPauseAuto
            };
            add(MENU_ITEM_FLAGS(0), ID_AUTO_TOGGLE, key);
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        if let Some(v) = &state.update_to {
            add_text(
                MENU_ITEM_FLAGS(0),
                ID_UPDATE,
                &tr_with(lang, Key::MenuUpdateTo, v),
            );
        }
        add(MENU_ITEM_FLAGS(0), ID_CHECK_UPDATES, Key::MenuCheckUpdates);
        if state.update_to.is_some() {
            let _ = SetMenuDefaultItem(menu, ID_UPDATE, 0);
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(MENU_ITEM_FLAGS(0), ID_EXIT, Key::MenuExit);

        // The menu only closes on an outside click if our window is foreground.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            x,
            y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as u32
    }
}

/// `WM_POWERBROADCAST`: resume and display on/off.
fn on_power(wparam: WPARAM, lparam: LPARAM) {
    match wparam.0 as u32 {
        PBT_APMSUSPEND => {
            // Close the awake-time sample before the PC sleeps.
            with_app(App::count_usage);
        }
        PBT_APMRESUMEAUTOMATIC => {
            with_app(|a| {
                a.support_check();
                #[cfg(feature = "plus")]
                a.auto.resume();
                a.rescan_soon();
            });
        }
        PBT_POWERSETTINGCHANGE if lparam.0 != 0 => {
            let p = lparam.0 as *const POWERBROADCAST_SETTING;
            // SAFETY: for this notification lparam points to a POWERBROADCAST_SETTING
            // followed by `DataLength` bytes of data (a u32 for both settings we watch).
            let (setting, data) = unsafe {
                let data = ((*p).DataLength >= 4)
                    .then(|| std::ptr::addr_of!((*p).Data).cast::<u32>().read_unaligned());
                ((*p).PowerSetting, data)
            };
            match data {
                Some(state) if setting == GUID_CONSOLE_DISPLAY_STATE => {
                    with_app(|a| a.on_display_state(state == 0));
                }
                Some(percent) if setting == GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS => {
                    with_app(|a| a.on_internal_brightness(percent.min(100) as u8));
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Whether a `WM_SETTINGCHANGE` lparam is the "ImmersiveColorSet" string.
fn is_color_change(lparam: LPARAM) -> bool {
    // SAFETY: for WM_SETTINGCHANGE a non-zero lparam is a NUL-terminated wide string.
    lparam.0 != 0
        && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }
            .is_ok_and(|s| s == "ImmersiveColorSet")
}

/// A command line forwarded by another instance.
fn on_copydata(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(fwd) = instance::decode(wparam, lparam) else {
        return LRESULT(0);
    };
    let (code, text) = match cli::parse(&fwd.args) {
        Ok(inv) => with_app(|a| a.exec_all(&inv.commands)).unwrap_or((1, String::new())),
        Err(e) => (2, format!("{e}\n{}", cli::USAGE)),
    };
    if !text.is_empty() {
        instance::reply(hwnd, fwd.reply_to, &text);
    }
    LRESULT(code as isize)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_APP_TRAY => on_tray(hwnd, tray::decode(wparam, lparam)),
        WM_APP_WHEEL => {
            let ev = wheel::decode(wparam, lparam);
            with_app(|a| a.on_wheel(ev));
        }
        WM_APP_DDC => {
            with_app(App::on_ddc);
        }
        WM_APP_UPDATE => {
            with_app(App::on_update);
        }
        #[cfg(feature = "plus")]
        auto::WM_APP_AUTO_SENSOR => {
            with_app(App::auto_sensor);
        }
        #[cfg(feature = "plus")]
        auto::WM_APP_AUTO_WEATHER => {
            with_app(App::auto_weather);
        }
        #[cfg(feature = "plus")]
        auto::WM_APP_AUTO_LOCATION => {
            with_app(App::auto_location);
        }
        // The administrator prompt for the extended gamma range is over: look again.
        #[cfg(feature = "plus")]
        auto::WM_APP_AUTO_COLOR => {
            with_app(|a| {
                a.color_replan(false);
                a.update_settings();
            });
        }
        // Clock or time zone changed: sun times shift.
        #[cfg(feature = "plus")]
        windows::Win32::UI::WindowsAndMessaging::WM_TIMECHANGE => {
            with_app(App::auto_replan);
        }
        WM_TIMER => {
            with_app(|a| a.on_timer(wparam.0));
        }
        WM_DISPLAYCHANGE => {
            with_app(|a| a.rescan_soon());
        }
        WM_POWERBROADCAST => {
            on_power(wparam, lparam);
            return LRESULT(1);
        }
        WM_SETTINGCHANGE => {
            let colors = is_color_change(lparam);
            with_app(|a| a.on_theme_change(colors));
        }
        // Safety net in case "ImmersiveColorSet" is missed.
        WM_THEMECHANGED | WM_SYSCOLORCHANGE => {
            with_app(|a| a.on_theme_change(true));
        }
        WM_DPICHANGED => {
            with_app(|a| a.on_theme_change(false));
        }
        WM_COPYDATA => return on_copydata(hwnd, wparam, lparam),
        WM_DESTROY => {
            with_app(App::shutdown);
            // SAFETY: ends the message loop of this thread.
            unsafe { PostQuitMessage(0) };
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with_app(App::readd_icons);
        }
        // SAFETY: default processing with the arguments we were given.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// Register [`WINDOW_CLASS`] and create the hidden top-level window (never shown;
/// not message-only so it receives the `TaskbarCreated` broadcast).
fn create_window() -> Option<HWND> {
    // SAFETY: plain Win32 registration and creation; `class` outlives both calls.
    unsafe {
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        let hinst = HINSTANCE(GetModuleHandleW(None).ok()?.0);
        let class = util::wide(WINDOW_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return None;
        }
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(class.as_ptr()),
            PCWSTR(class.as_ptr()),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        )
        .ok()
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Run the tray app until Exit/`quit`. `initial` are CLI commands to apply once
/// monitors are known; `updated`: this start follows a self-update; `autostart`: started by
/// Windows at logon (the first scan waits a moment for DDC/CI to come up). Returns the process exit code.
pub fn run(config_path: PathBuf, initial: Vec<Command>, updated: bool, autostart: bool) -> i32 {
    // SAFETY: process-wide setup on the UI thread before any window exists. DPI failure
    // is fine (the manifest sets it too).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        wmi::init_security();
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let config = Config::load(&config_path).unwrap_or_default();
    let Ok(renderer) = IconRenderer::new() else {
        return 1;
    };
    let Some(hwnd) = create_window() else {
        return 1;
    };
    theme::init_menus(hwnd, Theme::read().taskbar_light);
    flyout::init(on_flyout_action);
    settings::init(on_settings_action);
    support::init(on_support_closed);
    // SAFETY: valid window handle and GUID; the notification ends with the window.
    unsafe {
        let _ = RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        let _ = RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
    }

    let app = App::new(hwnd, config_path, config, renderer, initial, updated);
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    // SAFETY: plain timer call on our own window.
    unsafe {
        if autostart {
            SetTimer(Some(hwnd), TIMER_START, startup::AUTOSTART_DELAY_MS, None);
        }
        SetTimer(Some(hwnd), TIMER_UPDATE, UPDATE_POLL_MS, None);
    }
    if !autostart {
        with_app(App::scan);
    }

    let mut msg = MSG::default();
    // SAFETY: standard message loop; `msg` is a valid out-pointer.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // Drop the app (renderer, workers) before returning.
    APP.with(|cell| cell.borrow_mut().take());
    msg.wParam.0 as i32
}

/// `list`/`get` without a running instance: enumerate, read once, print.
/// Returns the exit code of the last command.
pub fn standalone(config_path: &Path, commands: &[Command]) -> i32 {
    let mut config = Config::load(config_path).unwrap_or_default();
    let skip = config.general.detection_in_progress.clone();
    // SAFETY: this thread has no COM apartment yet; WMI needs one for the built-in display.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    wmi::init_security();
    let found = monitors::enumerate();
    let wmi = wmi_instances(config.general.internal_display, &found);
    let (entries, _) = build_entries(&mut config, found, &skip, &wmi);
    let rows: Vec<Row> = entries
        .into_iter()
        .map(|mut e| {
            e.row.value = match &e.backend {
                Some(Backend::Ddc(h)) => ddc::read_once(h),
                Some(Backend::Wmi(i)) => Wmi::connect(i.clone()).and_then(|w| w.read()),
                None => None,
            };
            // A built-in panel that stays silent is still listed (as "no response").
            e.row.active = e.row.value.is_some() || matches!(e.backend, Some(Backend::Wmi(_)));
            log(&format!("probe id={} value={:?}", e.row.id, e.row.value));
            e.row
        })
        .collect();
    let rows: Vec<&Row> = rows.iter().collect();

    let mut code = 0;
    for cmd in commands {
        let (c, text) = query(cmd, &rows);
        code = c;
        if !text.is_empty() {
            println!("{text}");
        }
    }
    code
}
