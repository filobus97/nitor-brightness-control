//! UI thread: hidden window, tray icons, menu, wheel, system events (spec §4.1–4.3, §8–10).
//!
//! Layout of this file:
//! - `Row`, target resolution, `query` (list/get) and `build_entries`: shared with the standalone path.
//! - `App`: the single state object, owned by a thread-local; one method per event.
//! - `present`: render + add/modify one tray icon.
//! - window creation, window procedure, context menu, and the `run`/`standalone` entry points.

use super::ddc::{self, DdcEvent, DdcWorker, Notify};
use super::icon::{self, IconContent, IconRenderer, IconSpec, OwnedIcon};
use super::instance::{self, WINDOW_CLASS};
use super::monitors::{self, FoundMonitor, PhysicalHandle};
use super::theme::Theme;
use super::tray::{self, TrayEvent, WM_APP_TRAY};
use super::wheel::{self, WM_APP_WHEEL, WheelEvent};
use super::{autostart, util};
use crate::cli::{self, BrightnessOp, Command, Target};
use crate::color::{Rgb, next_palette_color};
use crate::config::{Config, MonitorSettings};
use crate::i18n::{Key, Lang, tr};
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{POWERBROADCAST_SETTING, RegisterPowerSettingNotification};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::{NIN_SELECT, NINF_KEY};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW,
    DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer, MENU_ITEM_FLAGS,
    MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, PBT_APMRESUMEAUTOMATIC, PBT_POWERSETTINGCHANGE,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow,
    SetTimer, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE,
    WM_CONTEXTMENU, WM_COPYDATA, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_MOUSEMOVE,
    WM_NULL, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{PCWSTR, w};

/// Posted by DDC workers when events are waiting in `App::queue`.
const WM_APP_DDC: u32 = WM_APP + 3;
/// `NIN_SELECT | NINF_KEY`: select via keyboard.
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const TIMER_WHEEL: usize = 1;
const TIMER_RESCAN: usize = 2;
const WHEEL_POLL_MS: u32 = 100;
/// One wheel notch.
const WHEEL_DELTA: i32 = 120;

/// Tray uID of the placeholder icon shown when no monitor is controllable.
const PLACEHOLDER_UID: u32 = 1;
const PLACEHOLDER_COLOR: Rgb = Rgb::new(0x88, 0x87, 0x80);

const ID_RESCAN: u32 = 1;
const ID_SETTINGS: u32 = 2;
const ID_AUTOSTART: u32 = 3;
const ID_EXIT: u32 = 4;

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
}

impl Row {
    fn name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.friendly)
    }
}

/// A discovered monitor plus the handle to probe it with (`None`: not controllable).
struct Entry {
    row: Row,
    handle: Option<PhysicalHandle>,
}

fn settings_of(config: &Config, id: &str) -> MonitorSettings {
    config.monitors.get(id).cloned().unwrap_or_default()
}

/// Turn enumerated monitors into rows in tray order (settings order, then left edge),
/// assigning palette colors to monitors without one (stored into `config`; the flag
/// says whether that changed it). Hidden and `skip`ped monitors lose their handle.
fn build_entries(
    config: &mut Config,
    mut found: Vec<FoundMonitor>,
    skip: &[String],
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
        let usable = !s.hidden && !skip.contains(&f.stable_id);
        let handle = if usable { f.handle.take() } else { None };
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
            },
            handle,
        });
    }
    (entries, changed)
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
        (true, None) => "–".into(),
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
    /// Wheel delta not yet worth a full notch.
    remainder: i32,
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
    balloons: Vec<(&'static str, &'static str)>,
    /// CLI commands to apply after the first probe.
    initial: Vec<Command>,
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
    ) -> App {
        let lang = pick_lang(&config);
        let mut balloons = Vec::new();
        if autostart::fix_stale() {
            balloons.push((tr(lang, Key::AppName), tr(lang, Key::AutostartFixed)));
        }
        config.general.autostart = autostart::is_enabled();
        let safe_skip = std::mem::take(&mut config.general.detection_in_progress);
        if !safe_skip.is_empty() {
            balloons.push((tr(lang, Key::AppName), tr(lang, Key::SafeModeText)));
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
        }
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
        let (entries, changed) = build_entries(&mut self.config, monitors::enumerate(), &skip);
        let probing: Vec<String> = entries
            .iter()
            .filter(|e| e.handle.is_some())
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
            let worker = e.handle.map(|h| self.spawn_worker(key, h, interval));
            self.monitors.push(Monitor {
                row: e.row,
                uid: i as u32 + 1,
                key,
                worker,
                health: Health::Ok,
                probed: false,
                remainder: 0,
                icon: None,
            });
        }
        for w in self.monitors.iter().filter_map(|m| m.worker.as_ref()) {
            w.read();
        }
        if !any {
            self.finish_probe();
        }
    }

    fn spawn_worker(&self, key: u64, handle: PhysicalHandle, interval: Duration) -> DdcWorker {
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
        let worker = DdcWorker::spawn(key, handle, interval, notify);
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
        let m = &mut self.monitors[i];
        if m.worker.is_none() {
            return;
        }
        let first = !m.probed;
        m.probed = true;
        if first && event == DdcEvent::Unresponsive {
            // Not controllable: no icon, release the handle.
            m.worker = None;
            return;
        }
        m.row.active = true;
        match event {
            DdcEvent::Value(p) => {
                m.row.value = Some(p);
                if m.health != Health::ReadOnly {
                    m.health = Health::Ok;
                }
            }
            DdcEvent::Written(_) => m.health = Health::Ok,
            DdcEvent::Unresponsive => m.health = Health::Unresponsive,
            DdcEvent::ReadOnly => m.health = Health::ReadOnly,
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
                tr(self.lang, Key::FirstRunTitle),
                tr(self.lang, Key::FirstRunText),
            ));
        }
        self.save();

        let uid = self
            .monitors
            .iter()
            .find(|m| m.icon.is_some())
            .map(|m| m.uid)
            .or(self.placeholder.is_some().then_some(PLACEHOLDER_UID));
        if let Some(uid) = uid {
            for (title, text) in self.balloons.drain(..) {
                tray::balloon(self.hwnd, uid, title, text);
            }
        }
        for cmd in std::mem::take(&mut self.initial) {
            self.exec(&cmd);
        }
    }

    /// Reload the config from disk (manual edits apply) and scan again.
    fn rescan(&mut self) {
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

    /// Apply `op` to monitor `i`: update the value, redraw immediately, queue the DDC write.
    fn adjust(&mut self, i: usize, op: BrightnessOp) {
        let s = settings_of(&self.config, &self.monitors[i].row.id);
        let m = &mut self.monitors[i];
        let (Some(current), Some(worker)) = (m.row.value, &m.worker) else {
            return;
        };
        let new = op.apply(current, s.min, s.max);
        if new != current {
            m.row.value = Some(new);
            worker.write(new);
            self.refresh(i);
        }
    }

    /// Left click: read every monitor again.
    fn read_all(&self) {
        for m in self.monitors.iter().filter(|m| m.row.active) {
            if let Some(w) = &m.worker {
                w.read();
            }
        }
    }

    // --- menu -------------------------------------------------------------

    fn on_menu(&mut self, cmd: u32) {
        match cmd {
            ID_RESCAN => self.rescan(),
            ID_SETTINGS => {
                if !self.path.exists() {
                    self.save();
                }
                util::open_with_default_app(&self.path);
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
    }

    // --- power ------------------------------------------------------------

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
                let result = self.for_target(target, |app, i| {
                    let m = &mut app.monitors[i];
                    m.row.color = *color;
                    let id = m.row.id.clone();
                    app.config.monitors.entry(id).or_default().color = Some(*color);
                    app.refresh(i);
                });
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
        self.clear();
        self.config.general.detection_in_progress.clear();
        self.save();
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

fn on_tray(hwnd: HWND, ev: TrayEvent) {
    match ev.msg {
        WM_MOUSEMOVE => {
            with_app(|a| a.arm_wheel());
        }
        NIN_SELECT | NIN_KEYSELECT => {
            with_app(|a| a.read_all());
        }
        WM_CONTEXTMENU => {
            let Some(lang) = with_app(|a| a.lang) else {
                return;
            };
            // The menu runs a nested message loop: the app must not be borrowed meanwhile.
            let cmd = show_menu(hwnd, ev.x, ev.y, lang, autostart::is_enabled());
            with_app(|a| a.on_menu(cmd));
        }
        _ => {}
    }
}

/// Show the tray context menu at a screen point and return the chosen command id (0: none).
fn show_menu(hwnd: HWND, x: i32, y: i32, lang: Lang, autostart_on: bool) -> u32 {
    // SAFETY: standard menu calls on a menu we create and destroy here; every text buffer
    // outlives the AppendMenuW call that copies it.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let add = |flags: MENU_ITEM_FLAGS, id: u32, key: Key| {
            let text = util::wide(tr(lang, key));
            let _ = AppendMenuW(menu, MF_STRING | flags, id as usize, PCWSTR(text.as_ptr()));
        };
        add(MENU_ITEM_FLAGS(0), ID_RESCAN, Key::MenuRescan);
        add(MENU_ITEM_FLAGS(0), ID_SETTINGS, Key::MenuSettings);
        let checked = if autostart_on {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        add(checked, ID_AUTOSTART, Key::MenuStartWithWindows);
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
        PBT_APMRESUMEAUTOMATIC => {
            with_app(|a| a.rescan_soon());
        }
        PBT_POWERSETTINGCHANGE if lparam.0 != 0 => {
            let p = lparam.0 as *const POWERBROADCAST_SETTING;
            // SAFETY: for this notification lparam points to a POWERBROADCAST_SETTING
            // followed by `DataLength` bytes of data (a u32 for the display state).
            let state = unsafe {
                ((*p).PowerSetting == GUID_CONSOLE_DISPLAY_STATE && (*p).DataLength >= 4)
                    .then(|| std::ptr::addr_of!((*p).Data).cast::<u32>().read_unaligned())
            };
            if let Some(state) = state {
                with_app(|a| a.on_display_state(state == 0));
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
/// monitors are known. Returns the process exit code.
pub fn run(config_path: PathBuf, initial: Vec<Command>) -> i32 {
    // SAFETY: process-wide setup on the UI thread before any window exists. DPI failure
    // is fine (the manifest sets it too).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let config = Config::load(&config_path).unwrap_or_default();
    let Ok(renderer) = IconRenderer::new() else {
        return 1;
    };
    let Some(hwnd) = create_window() else {
        return 1;
    };
    // SAFETY: valid window handle and GUID; the notification ends with the window.
    unsafe {
        let _ = RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
    }

    let app = App::new(hwnd, config_path, config, renderer, initial);
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    with_app(App::scan);

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
    let (entries, _) = build_entries(&mut config, monitors::enumerate(), &skip);
    let rows: Vec<Row> = entries
        .into_iter()
        .map(|mut e| {
            e.row.value = e.handle.as_ref().and_then(ddc::read_once);
            e.row.active = e.row.value.is_some();
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
