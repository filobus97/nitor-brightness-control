//! The Settings window (spec §10): a normal top-level window drawn like Windows 11 Settings.
//! Grouped cards with label-left/control-right rows, Mica backdrop, light/dark theme,
//! Direct2D controls, and native EDIT children for text input.
//!
//! Layout of this file:
//! - public API: `Snapshot` in, `Action` out.
//! - geometry: `State::layout` builds one list of `Node`s in content coordinates; painting
//!   and hit-testing both read it, and so does the placement of the EDIT children.
//! - `State`: the thread-local state and its input methods.
//! - painting, EDIT children, window procedure.
//!
//! Re-entrancy: `open`/`update` run while the app is borrowed, so they never call
//! `on_action`. The window procedure releases the state borrow before it emits an action,
//! because the app's handler calls back into `update`.

use super::icon::{color_f, has_family};
use super::theme::Theme;
use super::util;
use crate::color::{PALETTE, Rgb, text_on};
use crate::config::{Config, MonitorSettings, TrayStyle};
use crate::i18n::{Key, Lang, tr, tr_with};
use crate::update;
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::OnceLock;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DeleteObject, GetMonitorInfoW, HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, OUT_DEFAULT_PRECIS, SetBkColor,
    SetTextColor, ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_ESCAPE, VK_RETURN, VK_TAB,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EN_KILLFOCUS, EN_SETFOCUS, ES_AUTOHSCROLL,
    GetClientRect, GetCursorPos, GetParent, GetWindowTextW, IDC_ARROW, IsIconic, LoadCursorW,
    LoadIconW, RegisterClassExW, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SendMessageW, SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_SETFONT,
    WM_SETTINGCHANGE, WM_SIZE, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU,
};
use windows::core::{HRESULT, HSTRING, PCWSTR, w};
use windows_numerics::{Matrix3x2, Vector2};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// How the app currently sees one monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorState {
    Active,
    Unresponsive,
    ReadOnly,
    NotControllable,
    /// Built-in panel while "Control built-in display" is off.
    BuiltInOff,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorInfo {
    pub id: String,
    pub friendly: String,
    pub color: Rgb,
    pub state: MonitorState,
    /// Built-in (laptop) panel.
    pub internal: bool,
}

/// Everything the window shows: the config plus what only the app knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub config: Config,
    pub lang: Lang,
    /// Known monitors in tray order.
    pub monitors: Vec<MonitorInfo>,
}

/// A change to one monitor's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonitorEdit {
    Color(Rgb),
    Alias(Option<String>),
    Badge(Option<char>),
    Shown(bool),
    /// Brightness limits `(min, max)`.
    Limits(u8, u8),
    MoveUp,
    MoveDown,
}

/// What the user asked for; delivered through the function given to [`init`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Autostart(bool),
    /// `None`: follow Windows.
    Language(Option<Lang>),
    InternalDisplay(bool),
    CheckUpdates(bool),
    CheckNow,
    TrayStyle(TrayStyle),
    WheelStep(u8),
    RefreshOnOpen(bool),
    WriteInterval(u32),
    OpenConfig,
    /// A monitor, by stable ID.
    Monitor(String, MonitorEdit),
}

static ON_ACTION: OnceLock<fn(Action)> = OnceLock::new();

const CLASS: PCWSTR = w!("NitorSettings");

/// Register the window class. Once, on the UI thread.
pub fn init(on_action: fn(Action)) {
    let _ = ON_ACTION.set(on_action);
    // SAFETY: plain class registration with a static class name.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let hinst = HINSTANCE(module.0);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // Resource 1 is the exe icon, if the build embedded one.
            hIcon: LoadIconW(Some(hinst), PCWSTR(std::ptr::without_provenance::<u16>(1)))
                .unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

/// Show the window (or bring the open one to the front).
pub fn open(snap: Snapshot) {
    let existing = with_state(|st| {
        st.set_snapshot(snap.clone());
        st.hwnd
    });
    let Some(hwnd) = existing.or_else(|| create(snap)) else {
        return;
    };
    invalidate(hwnd);
    // SAFETY: plain window calls on our own window; the state borrow is released.
    unsafe {
        let cmd = if IsIconic(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_SHOW
        };
        let _ = ShowWindow(hwnd, cmd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

/// New config/monitor data while open. Never emits an action.
pub fn update(snap: Snapshot) {
    if let Some((hwnd, true)) = with_state(|st| (st.hwnd, st.set_snapshot(snap))) {
        invalidate(hwnd);
    }
}

pub fn is_open() -> bool {
    with_state(|_| ()).is_some()
}

// ---------------------------------------------------------------------------
// Metrics (DIP at 100%)
// ---------------------------------------------------------------------------

/// Client size of the window.
const WIDTH: f32 = 560.0;
const HEIGHT: f32 = 640.0;
const MARGIN: f32 = 24.0;
const PAD: f32 = 16.0;
const TOP: f32 = 8.0;
const HEAD_H: f32 = 36.0;
const ROW_H: f32 = 52.0;
const MON_ROW_H: f32 = 64.0;
const PALETTE_ROW_H: f32 = 48.0;
const SECTION_GAP: f32 = 8.0;
const CARD_GAP: f32 = 8.0;
const BOTTOM: f32 = 16.0;
const FIELD_H: f32 = 32.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 20.0;
const SEG_H: f32 = 32.0;
const BTN: f32 = 32.0;
const SLIDER_H: f32 = 28.0;
const THUMB_R: f32 = 9.0;
const TRACK_H: f32 = 4.0;
const SWATCH_R: f32 = 10.0;
const SWATCH_STEP: f32 = 30.0;
const MON_DOT_R: f32 = 11.0;
const EDIT_H: f32 = 20.0;
const EDIT_PAD: f32 = 8.0;
const RADIUS: f32 = 4.0;
/// Scroll distance of one wheel notch.
const NOTCH: f32 = 56.0;

const INTERVAL_MIN: u32 = 20;
const INTERVAL_MAX: u32 = 500;
const INTERVAL_STEP: u32 = 10;
const WHEEL_STEPS: [u8; 4] = [1, 2, 5, 10];
const LANGS: [Option<Lang>; 3] = [None, Some(Lang::En), Some(Lang::It)];
const STYLES: [TrayStyle; 2] = [TrayStyle::Text, TrayStyle::Pill];

/// `WM_MOUSELEAVE` (not exposed by the bindings).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// An EDIT asks its parent to restore the model text (Esc); `lparam` is the EDIT.
const WM_REVERT: u32 = WM_APP + 1;
const EDIT_SUBCLASS: usize = 1;
/// `D2DERR_RECREATE_TARGET`: the render target must be recreated.
const D2DERR_RECREATE_TARGET: HRESULT = HRESULT(0x8899000Cu32 as i32);

#[derive(Clone, Copy, Default)]
struct Rect {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

impl Rect {
    fn new(l: f32, t: f32, r: f32, b: f32) -> Rect {
        Rect { l, t, r, b }
    }
    fn centered(cx: f32, cy: f32, w: f32, h: f32) -> Rect {
        Rect::new(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
    }
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
    fn cx(&self) -> f32 {
        (self.l + self.r) / 2.0
    }
    fn cy(&self) -> f32 {
        (self.t + self.b) / 2.0
    }
    fn inset(&self, d: f32) -> Rect {
        Rect::new(self.l + d, self.t + d, self.r - d, self.b - d)
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.l,
            top: self.t,
            right: self.r,
            bottom: self.b,
        }
    }
}

// ---------------------------------------------------------------------------
// Controls and layout
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Toggle {
    Autostart,
    Internal,
    Updates,
    RefreshOnOpen,
    /// "Show in tray" of monitor `i`.
    Shown(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Language,
    TrayStyle,
    WheelStep,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slide {
    Interval,
    /// Brightness limits of monitor `i`.
    Range(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Btn {
    CheckNow,
    OpenConfig,
    /// The color dot of monitor `i`: opens its palette.
    Color(usize),
    Up(usize),
    Down(usize),
}

/// Everything clickable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctl {
    Toggle(Toggle),
    /// Option `k` of a segmented choice.
    Choice(Choice, usize),
    Slider(Slide),
    Button(Btn),
    /// Palette swatch `k` of monitor `i`.
    Swatch(usize, usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Alias,
    Badge,
    Hex,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Handle {
    Min,
    Max,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Drag {
    Interval,
    Handle(usize, Handle),
}

enum Node {
    Heading(Key, Rect),
    Card(Rect),
    Separator(Rect),
    Label {
        title: String,
        sub: Option<String>,
        rect: Rect,
        dim: bool,
    },
    /// Background of a segmented choice.
    Group(Rect),
    /// Box behind a native EDIT.
    Field {
        rect: Rect,
        mon: usize,
        kind: Kind,
    },
    /// Right-aligned value text.
    Value(String, Rect),
    Ctl(Ctl, Rect),
}

struct Layout {
    nodes: Vec<Node>,
    /// Content height in device pixels.
    height: f32,
}

impl Layout {
    fn rect_of(&self, ctl: Ctl) -> Option<Rect> {
        self.nodes.iter().find_map(|n| match n {
            Node::Ctl(c, r) if *c == ctl => Some(*r),
            _ => None,
        })
    }

    fn hit(&self, x: f32, y: f32) -> Option<Ctl> {
        self.nodes.iter().find_map(|n| match n {
            Node::Ctl(c, r) if r.contains(x, y) => Some(*c),
            _ => None,
        })
    }

    fn field(&self, mon: usize, kind: Kind) -> Option<Rect> {
        self.nodes.iter().find_map(|n| match n {
            Node::Field {
                rect,
                mon: m,
                kind: k,
            } if *m == mon && *k == kind => Some(*rect),
            _ => None,
        })
    }
}

/// Appends rows to a layout, top to bottom, in device pixels.
struct Builder {
    s: f32,
    y: f32,
    nodes: Vec<Node>,
    /// Index in `nodes` where the open card's background goes, and the card's top.
    card: (usize, f32),
    first_row: bool,
}

impl Builder {
    fn new(s: f32) -> Builder {
        Builder {
            s,
            y: TOP * s,
            nodes: Vec::new(),
            card: (0, 0.0),
            first_row: true,
        }
    }

    fn d(&self, v: f32) -> f32 {
        v * self.s
    }

    fn heading(&mut self, key: Key) {
        let r = Rect::new(
            self.d(MARGIN),
            self.y,
            self.d(WIDTH - MARGIN),
            self.y + self.d(HEAD_H),
        );
        self.nodes.push(Node::Heading(key, r));
        self.y = r.b;
    }

    fn begin_card(&mut self) {
        self.card = (self.nodes.len(), self.y);
        self.first_row = true;
    }

    fn end_card(&mut self, gap: f32) {
        let (at, top) = self.card;
        let r = Rect::new(self.d(MARGIN), top, self.d(WIDTH - MARGIN), self.y);
        self.nodes.insert(at, Node::Card(r));
        self.y += self.d(gap);
    }

    /// The next row of the open card.
    fn row(&mut self, h: f32) -> Rect {
        let top = self.y;
        let (l, r) = (self.d(MARGIN), self.d(WIDTH - MARGIN));
        if !self.first_row {
            let sep = Rect::new(l + self.d(PAD), top, r - self.d(PAD), top + 1.0);
            self.nodes.push(Node::Separator(sep));
        }
        self.first_row = false;
        self.y += self.d(h);
        Rect::new(l, top, r, self.y)
    }

    /// Where controls start, from the right.
    fn edge(&self, row: Rect) -> f32 {
        row.r - self.d(PAD)
    }

    /// A `w`×`h` DIP box ending at `right`, centered vertically in `row`.
    fn slot(&self, row: Rect, right: f32, w: f32, h: f32) -> Rect {
        let (w, h) = (self.d(w), self.d(h));
        Rect::new(right - w, row.cy() - h / 2.0, right, row.cy() + h / 2.0)
    }

    fn ctl(&mut self, c: Ctl, r: Rect) {
        self.nodes.push(Node::Ctl(c, r));
    }

    /// The row's text, from its left padding to `right` (minus a gap).
    fn label(&mut self, row: Rect, left: f32, right: f32, title: String, sub: Option<String>) {
        let rect = Rect::new(left, row.t, right - self.d(12.0), row.b);
        self.nodes.push(Node::Label {
            title,
            sub,
            rect,
            dim: false,
        });
    }

    fn left(&self, row: Rect) -> f32 {
        row.l + self.d(PAD)
    }

    fn toggle_row(&mut self, t: Toggle, title: String) {
        let row = self.row(ROW_H);
        let c = self.slot(row, self.edge(row), SWITCH_W, SWITCH_H);
        self.ctl(Ctl::Toggle(t), c);
        self.label(row, self.left(row), c.l, title, None);
    }

    fn choice_row(&mut self, choice: Choice, n: usize, seg_w: f32, title: String) {
        let row = self.row(ROW_H);
        let g = self.slot(row, self.edge(row), seg_w * n as f32, SEG_H);
        self.nodes.push(Node::Group(g));
        for k in 0..n {
            let l = g.l + self.d(seg_w) * k as f32;
            let r = Rect::new(l, g.t, l + self.d(seg_w), g.b);
            self.ctl(Ctl::Choice(choice, k), r);
        }
        self.label(row, self.left(row), g.l, title, None);
    }

    fn button_row(&mut self, b: Btn, w: f32, title: String) {
        let row = self.row(ROW_H);
        let c = self.slot(row, self.edge(row), w, BTN);
        self.ctl(Ctl::Button(b), c);
        self.label(row, self.left(row), c.l, title, None);
    }
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

struct Pal {
    /// Window background when there is no Mica.
    page: Rgb,
    /// Card fill and its opacity (translucent over Mica).
    card: (Rgb, f32),
    text: Rgb,
    /// Fill of text fields (also the EDIT background: opaque).
    field: Rgb,
    thumb: Rgb,
}

fn pal(light: bool, mica: bool) -> Pal {
    if light {
        Pal {
            page: Rgb::new(0xF3, 0xF3, 0xF3),
            card: (Rgb::WHITE, if mica { 0.7 } else { 1.0 }),
            text: Rgb::new(0x1B, 0x1B, 0x1B),
            field: Rgb::WHITE,
            thumb: Rgb::WHITE,
        }
    } else {
        Pal {
            page: Rgb::new(0x20, 0x20, 0x20),
            card: if mica {
                (Rgb::WHITE, 0.05)
            } else {
                (Rgb::new(0x2B, 0x2B, 0x2B), 1.0)
            },
            text: Rgb::WHITE,
            field: Rgb::new(0x34, 0x34, 0x34),
            thumb: Rgb::new(0x45, 0x45, 0x45),
        }
    }
}

fn colorref(c: Rgb) -> COLORREF {
    COLORREF(u32::from(c.r) | u32::from(c.g) << 8 | u32::from(c.b) << 16)
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct Fonts {
    body: IDWriteTextFormat,
    semi: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    value: IDWriteTextFormat,
    mid: IDWriteTextFormat,
    glyph: IDWriteTextFormat,
}

struct Target {
    rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
}

/// A native EDIT child, placed over the `Field` node of its monitor.
struct Edit {
    hwnd: HWND,
    mon: String,
    kind: Kind,
}

struct State {
    hwnd: HWND,
    hinst: HINSTANCE,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    text_font: &'static str,
    icon_font: &'static str,
    /// Mica backdrop accepted by DWM.
    mica: bool,
    fonts: Option<(u32, Fonts)>,
    target: Option<Target>,
    /// GDI font of the EDITs, with the DPI it was made for.
    gdi_font: Option<(u32, HFONT)>,
    /// Background brush of the EDITs (the field color).
    edit_brush: HBRUSH,
    edits: Vec<Edit>,

    snap: Snapshot,
    title_lang: Lang,
    theme: Theme,
    pal: Pal,
    dpi: u32,
    /// Scroll offset in device pixels.
    scroll: f32,
    hover: Option<Ctl>,
    drag: Option<Drag>,
    tracking_leave: bool,
    /// Monitor (by ID) whose palette is open.
    picker: Option<String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Run `f` on the state, unless it does not exist or is borrowed (re-entrant message).
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut s| s.as_mut().map(f))
    })
}

fn emit(action: Option<Action>) {
    if let (Some(a), Some(f)) = (action, ON_ACTION.get()) {
        f(a);
    }
}

fn invalidate(hwnd: HWND) {
    // SAFETY: plain repaint request for our own window.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// `x` (0..=1) of a slider track, from a client coordinate.
fn frac_at(r: Rect, s: f32, x: f32) -> f32 {
    let (x0, x1) = (r.l + THUMB_R * s, r.r - THUMB_R * s);
    ((x - x0) / (x1 - x0)).clamp(0.0, 1.0)
}

fn x_at(r: Rect, s: f32, frac: f32) -> f32 {
    let (x0, x1) = (r.l + THUMB_R * s, r.r - THUMB_R * s);
    x0 + (x1 - x0) * frac
}

impl State {
    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn lang(&self) -> Lang {
        self.snap.lang
    }

    fn mon_settings(&self, i: usize) -> MonitorSettings {
        self.snap
            .monitors
            .get(i)
            .and_then(|m| self.snap.config.monitors.get(&m.id))
            .cloned()
            .unwrap_or_default()
    }

    fn mon_name(&self, i: usize) -> String {
        let m = &self.snap.monitors[i];
        self.mon_settings(i)
            .alias
            .unwrap_or_else(|| m.friendly.clone())
    }

    // --- geometry ---------------------------------------------------------

    fn layout(&self) -> Layout {
        let (s, lang) = (self.scale(), self.lang());
        let g = &self.snap.config.general;
        let t = |k: Key| tr(lang, k).to_string();
        let mut b = Builder::new(s);

        b.heading(Key::SecGeneral);
        b.begin_card();
        b.toggle_row(Toggle::Autostart, t(Key::MenuStartWithWindows));
        b.choice_row(Choice::Language, 3, 84.0, t(Key::LblLanguage));
        if self.snap.monitors.iter().any(|m| m.internal) {
            b.toggle_row(Toggle::Internal, t(Key::MenuInternalDisplay));
        }
        b.toggle_row(Toggle::Updates, t(Key::LblCheckUpdates));
        let version = tr_with(lang, Key::LblVersion, update::current_version());
        b.button_row(Btn::CheckNow, 120.0, version);
        b.end_card(SECTION_GAP);

        b.heading(Key::SecTray);
        b.begin_card();
        b.choice_row(Choice::TrayStyle, 2, 96.0, t(Key::LblTrayStyle));
        b.choice_row(Choice::WheelStep, 4, 52.0, t(Key::LblWheelStep));
        b.toggle_row(Toggle::RefreshOnOpen, t(Key::LblRefreshOnOpen));
        b.end_card(SECTION_GAP);

        b.heading(Key::SecMonitors);
        if self.snap.monitors.is_empty() {
            b.begin_card();
            let row = b.row(ROW_H);
            b.label(row, b.left(row), row.r, t(Key::NoMonitors), None);
            b.end_card(SECTION_GAP);
        }
        for i in 0..self.snap.monitors.len() {
            self.monitor_card(&mut b, i);
        }

        b.heading(Key::SecAdvanced);
        b.begin_card();
        let row = b.row(ROW_H);
        let value = Rect::new(b.edge(row) - b.d(64.0), row.t, b.edge(row), row.b);
        let slider = b.slot(row, value.l - b.d(8.0), 200.0, SLIDER_H);
        b.nodes
            .push(Node::Value(format!("{} ms", g.write_interval_ms), value));
        b.ctl(Ctl::Slider(Slide::Interval), slider);
        b.label(row, b.left(row), slider.l, t(Key::LblWriteInterval), None);
        b.button_row(Btn::OpenConfig, 210.0, t(Key::LblConfigFile));
        b.end_card(0.0);

        Layout {
            height: b.y + b.d(BOTTOM),
            nodes: b.nodes,
        }
    }

    /// One monitor's card: header, optional palette, name, badge, tray toggle and limits.
    fn monitor_card(&self, b: &mut Builder, i: usize) {
        let (lang, m) = (self.lang(), &self.snap.monitors[i]);
        let ms = self.mon_settings(i);
        let t = |k: Key| tr(lang, k).to_string();
        b.begin_card();

        let row = b.row(MON_ROW_H);
        let down = b.slot(row, b.edge(row), BTN, BTN);
        let up = b.slot(row, down.l - b.d(4.0), BTN, BTN);
        let dot = Rect::new(
            b.left(row),
            row.cy() - b.d(BTN) / 2.0,
            b.left(row) + b.d(BTN),
            row.cy() + b.d(BTN) / 2.0,
        );
        b.ctl(Ctl::Button(Btn::Color(i)), dot);
        b.ctl(Ctl::Button(Btn::Up(i)), up);
        b.ctl(Ctl::Button(Btn::Down(i)), down);
        let state = match m.state {
            MonitorState::Active => None,
            MonitorState::Unresponsive => Some(Key::Unresponsive),
            MonitorState::ReadOnly => Some(Key::ReadOnly),
            MonitorState::NotControllable => Some(Key::StateNotControllable),
            MonitorState::BuiltInOff => Some(Key::StateBuiltInOff),
            MonitorState::Hidden => Some(Key::StateHidden),
        };
        let renamed = ms.alias.is_some().then(|| m.friendly.clone());
        let sub: Vec<String> = renamed.into_iter().chain(state.map(t)).collect();
        let dim = matches!(
            m.state,
            MonitorState::NotControllable | MonitorState::BuiltInOff | MonitorState::Hidden
        );
        let sub = (!sub.is_empty()).then(|| sub.join(" · "));
        b.label(row, dot.r + b.d(12.0), up.l, self.mon_name(i), sub);
        if let Some(Node::Label { dim: d, .. }) = b.nodes.last_mut() {
            *d = dim;
        }

        if self.picker.as_deref() == Some(m.id.as_str()) {
            let row = b.row(PALETTE_ROW_H);
            for k in 0..PALETTE.len() {
                let cx = b.left(row) + b.d(14.0 + SWATCH_STEP * k as f32);
                let r = Rect::centered(cx, row.cy(), b.d(SWATCH_STEP), b.d(BTN));
                b.ctl(Ctl::Swatch(i, k), r);
            }
            let rect = b.slot(row, b.edge(row), 96.0, FIELD_H);
            b.nodes.push(Node::Field {
                rect,
                mon: i,
                kind: Kind::Hex,
            });
        }

        let row = b.row(ROW_H);
        let rect = b.slot(row, b.edge(row), 220.0, FIELD_H);
        b.nodes.push(Node::Field {
            rect,
            mon: i,
            kind: Kind::Alias,
        });
        let hint = tr_with(lang, Key::HintAlias, &m.friendly);
        b.label(row, b.left(row), rect.l, t(Key::LblName), Some(hint));

        let row = b.row(ROW_H);
        let rect = b.slot(row, b.edge(row), 56.0, FIELD_H);
        b.nodes.push(Node::Field {
            rect,
            mon: i,
            kind: Kind::Badge,
        });
        b.label(row, b.left(row), rect.l, t(Key::LblBadge), None);

        b.toggle_row(Toggle::Shown(i), t(Key::LblShowInTray));

        let row = b.row(ROW_H);
        let value = Rect::new(b.edge(row) - b.d(72.0), row.t, b.edge(row), row.b);
        let slider = b.slot(row, value.l - b.d(8.0), 200.0, SLIDER_H);
        b.nodes
            .push(Node::Value(format!("{} – {}", ms.min, ms.max), value));
        b.ctl(Ctl::Slider(Slide::Range(i)), slider);
        b.label(row, b.left(row), slider.l, t(Key::LblLimits), None);

        b.end_card(CARD_GAP);
    }

    fn client_height(&self) -> f32 {
        let mut rc = RECT::default();
        // SAFETY: valid window and out-pointer.
        let _ = unsafe { GetClientRect(self.hwnd, &mut rc) };
        (rc.bottom - rc.top) as f32
    }

    fn max_scroll(&self, layout: &Layout) -> f32 {
        (layout.height - self.client_height()).max(0.0)
    }

    fn enabled(&self, ctl: Ctl) -> bool {
        match ctl {
            Ctl::Button(Btn::Up(i)) => i > 0,
            Ctl::Button(Btn::Down(i)) => i + 1 < self.snap.monitors.len(),
            _ => true,
        }
    }

    fn hit(&self, layout: &Layout, x: f32, y: f32) -> Option<Ctl> {
        layout.hit(x, y + self.scroll).filter(|&c| self.enabled(c))
    }

    // --- snapshot and sync -----------------------------------------------

    /// Take new data; returns whether anything changed.
    fn set_snapshot(&mut self, snap: Snapshot) -> bool {
        if snap == self.snap {
            return false;
        }
        if let Some(id) = &self.picker {
            if !snap.monitors.iter().any(|m| &m.id == id) {
                self.picker = None;
            }
        }
        self.snap = snap;
        self.sync_all();
        true
    }

    /// Bring everything that depends on the layout in line: title, scroll range, EDITs.
    fn sync_all(&mut self) {
        if self.title_lang != self.lang() {
            self.title_lang = self.lang();
            let title = util::wide(tr(self.lang(), Key::SettingsTitle));
            // SAFETY: our own window; `title` is NUL-terminated and outlives the call.
            unsafe {
                let _ = SetWindowTextW(self.hwnd, PCWSTR(title.as_ptr()));
            }
        }
        self.ensure_gdi_font();
        let layout = self.layout();
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(&layout));
        self.sync_edits(&layout);
    }

    fn set_dpi(&mut self, dpi: u32) {
        self.scroll *= dpi as f32 / self.dpi as f32;
        self.dpi = dpi;
    }

    fn on_theme_change(&mut self) {
        self.theme = Theme::read();
        self.pal = pal(self.theme.apps_light, self.mica);
        set_dark(self.hwnd, !self.theme.apps_light);
        // SAFETY: a new brush replaces the old one, which is deleted after the EDITs
        // were asked to repaint with the new one.
        unsafe {
            let old = std::mem::replace(
                &mut self.edit_brush,
                CreateSolidBrush(colorref(self.pal.field)),
            );
            for e in &self.edits {
                invalidate(e.hwnd);
            }
            let _ = DeleteObject(HGDIOBJ(old.0));
        }
    }

    fn scroll_by(&mut self, delta: i32) -> bool {
        let layout = self.layout();
        let max = self.max_scroll(&layout);
        let to = (self.scroll - delta as f32 / 120.0 * NOTCH * self.scale()).clamp(0.0, max);
        let moved = to != self.scroll;
        self.scroll = to;
        if moved {
            self.sync_edits(&layout);
        }
        moved
    }

    // --- input ------------------------------------------------------------

    fn mouse_move(&mut self, x: f32, y: f32) -> Option<Action> {
        let layout = self.layout();
        self.hover = self.hit(&layout, x, y);
        self.drag_to(&layout, x)
    }

    fn toggle_on(&self, t: Toggle) -> bool {
        let g = &self.snap.config.general;
        match t {
            Toggle::Autostart => g.autostart,
            Toggle::Internal => g.internal_display,
            Toggle::Updates => g.check_updates,
            Toggle::RefreshOnOpen => g.refresh_on_open,
            Toggle::Shown(i) => !self.mon_settings(i).hidden,
        }
    }

    fn choice_selected(&self, c: Choice, k: usize) -> bool {
        let g = &self.snap.config.general;
        match c {
            Choice::Language => Lang::from_setting(&g.language) == LANGS[k],
            Choice::TrayStyle => g.tray_style == STYLES[k],
            Choice::WheelStep => g.wheel_step == WHEEL_STEPS[k],
        }
    }

    fn choice_label(&self, c: Choice, k: usize) -> String {
        let lang = self.lang();
        match c {
            Choice::Language => match LANGS[k] {
                None => tr(lang, Key::LangAuto).to_string(),
                Some(Lang::En) => "English".to_string(),
                Some(Lang::It) => "Italiano".to_string(),
            },
            Choice::TrayStyle => match STYLES[k] {
                TrayStyle::Text => tr(lang, Key::StyleNumber).to_string(),
                TrayStyle::Pill => tr(lang, Key::StylePill).to_string(),
            },
            Choice::WheelStep => WHEEL_STEPS[k].to_string(),
        }
    }

    /// Slider position as `(low, high)` fractions; a single-thumb slider has low 0.
    fn slide_fracs(&self, s: Slide) -> (f32, f32) {
        match s {
            Slide::Interval => {
                let v = self
                    .snap
                    .config
                    .general
                    .write_interval_ms
                    .clamp(INTERVAL_MIN, INTERVAL_MAX);
                (
                    0.0,
                    (v - INTERVAL_MIN) as f32 / (INTERVAL_MAX - INTERVAL_MIN) as f32,
                )
            }
            Slide::Range(i) => {
                let ms = self.mon_settings(i);
                (f32::from(ms.min) / 100.0, f32::from(ms.max) / 100.0)
            }
        }
    }

    fn mouse_down(&mut self, x: f32, y: f32) -> Option<Action> {
        let layout = self.layout();
        let ctl = self.hit(&layout, x, y)?;
        match ctl {
            Ctl::Toggle(t) => {
                let on = !self.toggle_on(t);
                Some(match t {
                    Toggle::Autostart => Action::Autostart(on),
                    Toggle::Internal => Action::InternalDisplay(on),
                    Toggle::Updates => Action::CheckUpdates(on),
                    Toggle::RefreshOnOpen => Action::RefreshOnOpen(on),
                    Toggle::Shown(i) => self.monitor_action(i, MonitorEdit::Shown(on))?,
                })
            }
            Ctl::Choice(c, k) => Some(match c {
                Choice::Language => Action::Language(LANGS[k]),
                Choice::TrayStyle => Action::TrayStyle(STYLES[k]),
                Choice::WheelStep => Action::WheelStep(WHEEL_STEPS[k]),
            }),
            Ctl::Slider(Slide::Interval) => {
                self.drag = Some(Drag::Interval);
                self.drag_to(&layout, x)
            }
            Ctl::Slider(Slide::Range(i)) => {
                // Grab the nearer handle; on a tie the one that can move toward the click.
                let (lo, hi) = self.slide_fracs(Slide::Range(i));
                let r = layout.rect_of(ctl)?;
                let s = self.scale();
                let (xl, xh) = (x_at(r, s, lo), x_at(r, s, hi));
                let (dl, dh) = ((x - xl).abs(), (x - xh).abs());
                let min = dl < dh || (dl == dh && x < xl);
                let which = if min { Handle::Min } else { Handle::Max };
                self.drag = Some(Drag::Handle(i, which));
                self.drag_to(&layout, x)
            }
            Ctl::Button(b) => match b {
                Btn::CheckNow => Some(Action::CheckNow),
                Btn::OpenConfig => Some(Action::OpenConfig),
                Btn::Color(i) => {
                    let id = self.snap.monitors.get(i)?.id.clone();
                    self.picker = if self.picker.as_ref() == Some(&id) {
                        None
                    } else {
                        Some(id)
                    };
                    self.sync_all();
                    None
                }
                Btn::Up(i) => self.monitor_action(i, MonitorEdit::MoveUp),
                Btn::Down(i) => self.monitor_action(i, MonitorEdit::MoveDown),
            },
            Ctl::Swatch(i, k) => self.monitor_action(i, MonitorEdit::Color(PALETTE[k])),
        }
    }

    fn monitor_action(&self, i: usize, edit: MonitorEdit) -> Option<Action> {
        Some(Action::Monitor(self.snap.monitors.get(i)?.id.clone(), edit))
    }

    fn drag_to(&mut self, layout: &Layout, x: f32) -> Option<Action> {
        let s = self.scale();
        match self.drag? {
            Drag::Interval => {
                let r = layout.rect_of(Ctl::Slider(Slide::Interval))?;
                let steps = (INTERVAL_MAX - INTERVAL_MIN) / INTERVAL_STEP;
                let k = (frac_at(r, s, x) * steps as f32).round() as u32;
                let ms = INTERVAL_MIN + k * INTERVAL_STEP;
                (ms != self.snap.config.general.write_interval_ms)
                    .then_some(Action::WriteInterval(ms))
            }
            Drag::Handle(i, which) => {
                let r = layout.rect_of(Ctl::Slider(Slide::Range(i)))?;
                let v = (frac_at(r, s, x) * 100.0).round() as u8;
                let ms = self.mon_settings(i);
                let (lo, hi) = match which {
                    Handle::Min => (v.min(ms.max), ms.max),
                    Handle::Max => (ms.min, v.max(ms.min)),
                };
                if (lo, hi) == (ms.min, ms.max) {
                    return None;
                }
                self.monitor_action(i, MonitorEdit::Limits(lo, hi))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// EDIT children
// ---------------------------------------------------------------------------

fn get_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: valid window and writable buffer.
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn set_text(hwnd: HWND, text: &str) {
    let wide = util::wide(text);
    // SAFETY: NUL-terminated buffer that outlives the call.
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
    }
}

impl State {
    /// The text an EDIT shows when it has no pending input.
    fn edit_text(&self, mon: &str, kind: Kind) -> String {
        let Some(i) = self.snap.monitors.iter().position(|m| m.id == mon) else {
            return String::new();
        };
        match kind {
            Kind::Alias => self.mon_settings(i).alias.unwrap_or_default(),
            Kind::Badge => self
                .mon_settings(i)
                .badge
                .map(String::from)
                .unwrap_or_default(),
            Kind::Hex => self.snap.monitors[i].color.to_hex(),
        }
    }

    fn ensure_gdi_font(&mut self) {
        if self.gdi_font.is_some_and(|(dpi, _)| dpi == self.dpi) {
            return;
        }
        let height = -(14.0 * self.scale()).round() as i32;
        let face = HSTRING::from(self.text_font);
        // SAFETY: plain GDI font creation; the face name outlives the call.
        let font = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                &face,
            )
        };
        for e in &self.edits {
            // SAFETY: valid EDIT; the font is kept alive until after this loop.
            unsafe {
                SendMessageW(
                    e.hwnd,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
        if let Some((_, old)) = self.gdi_font.replace((self.dpi, font)) {
            // SAFETY: the EDITs no longer use the old font.
            unsafe {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
        }
    }

    fn make_edit(&self, mon: &str, kind: Kind) -> Option<Edit> {
        // SAFETY: plain child window creation; the subclass procedure is a static function.
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                0,
                0,
                Some(self.hwnd),
                None,
                Some(self.hinst),
                None,
            )
            .ok()?;
            let limit = match kind {
                Kind::Alias => 32,
                Kind::Badge => 2,
                Kind::Hex => 7,
            };
            SendMessageW(
                hwnd,
                windows::Win32::UI::Controls::EM_SETLIMITTEXT,
                Some(WPARAM(limit)),
                None,
            );
            if let Some((_, font)) = self.gdi_font {
                SendMessageW(
                    hwnd,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
            let _ = SetWindowSubclass(hwnd, Some(edit_proc), EDIT_SUBCLASS, 0);
            Some(Edit {
                hwnd,
                mon: mon.to_string(),
                kind,
            })
        }
    }

    /// One EDIT per monitor and field; each sits over its `Field` node (hidden without one).
    fn sync_edits(&mut self, layout: &Layout) {
        let ids: Vec<String> = self.snap.monitors.iter().map(|m| m.id.clone()).collect();
        self.edits.retain(|e| {
            let keep = ids.contains(&e.mon);
            if !keep {
                // SAFETY: our own child window.
                unsafe {
                    let _ = DestroyWindow(e.hwnd);
                }
            }
            keep
        });
        for id in &ids {
            for kind in [Kind::Alias, Kind::Badge, Kind::Hex] {
                if !self.edits.iter().any(|e| &e.mon == id && e.kind == kind) {
                    if let Some(e) = self.make_edit(id, kind) {
                        self.edits.push(e);
                    }
                }
            }
        }
        let s = self.scale();
        // SAFETY: plain focus query.
        let focused = unsafe { GetFocus() };
        for e in &self.edits {
            let field = ids
                .iter()
                .position(|id| id == &e.mon)
                .and_then(|i| layout.field(i, e.kind));
            // SAFETY: our own child windows.
            unsafe {
                let Some(f) = field else {
                    let _ = ShowWindow(e.hwnd, SW_HIDE);
                    continue;
                };
                let r = Rect::new(
                    f.l + EDIT_PAD * s,
                    f.cy() - EDIT_H * s / 2.0 - self.scroll,
                    f.r - EDIT_PAD * s,
                    f.cy() + EDIT_H * s / 2.0 - self.scroll,
                );
                let _ = SetWindowPos(
                    e.hwnd,
                    None,
                    r.l.round() as i32,
                    r.t.round() as i32,
                    (r.r - r.l).round() as i32,
                    (r.b - r.t).round() as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            // Never overwrite what the user is typing.
            let text = self.edit_text(&e.mon, e.kind);
            if e.hwnd != focused && get_text(e.hwnd) != text {
                set_text(e.hwnd, &text);
            }
        }
    }

    /// Put the model text back into `edit` (Esc, or input that changes nothing).
    fn restore_text(&self, edit: HWND) {
        if let Some(e) = self.edits.iter().find(|e| e.hwnd == edit) {
            set_text(edit, &self.edit_text(&e.mon, e.kind));
        }
    }

    /// The EDIT lost focus: turn its text into an action. Unusable input is reverted.
    fn commit(&self, edit: HWND) -> Option<Action> {
        let e = self.edits.iter().find(|e| e.hwnd == edit)?;
        let i = self.snap.monitors.iter().position(|m| m.id == e.mon)?;
        let text = get_text(edit);
        let text = text.trim();
        let ms = self.mon_settings(i);
        let change = match e.kind {
            Kind::Alias => {
                let alias = Some(text.to_string()).filter(|t| !t.is_empty());
                (alias != ms.alias).then_some(MonitorEdit::Alias(alias))
            }
            Kind::Badge => {
                let badge = text.chars().next();
                (badge != ms.badge).then_some(MonitorEdit::Badge(badge))
            }
            Kind::Hex => Rgb::from_hex(text)
                .filter(|&c| c != self.snap.monitors[i].color)
                .map(MonitorEdit::Color),
        };
        if change.is_none() {
            self.restore_text(edit);
        }
        self.monitor_action(i, change?)
    }
}

/// Subclass of the EDITs: Enter, Tab and Esc hand the focus back to the window (which
/// commits the text); Esc restores the old text first.
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let key = wparam.0 as u16;
    if (msg == WM_KEYDOWN || msg == WM_CHAR)
        && (key == VK_RETURN.0 || key == VK_TAB.0 || key == VK_ESCAPE.0)
    {
        if msg == WM_KEYDOWN {
            // SAFETY: the parent of a live child; the state borrow is not held here.
            unsafe {
                if let Ok(parent) = GetParent(hwnd) {
                    if key == VK_ESCAPE.0 {
                        SendMessageW(parent, WM_REVERT, None, Some(LPARAM(hwnd.0 as isize)));
                    }
                    let _ = SetFocus(Some(parent));
                }
            }
        }
        // Swallow the key (and its WM_CHAR) so the EDIT does not beep.
        return LRESULT(0);
    }
    if msg == WM_NCDESTROY {
        // SAFETY: removes the subclass installed in `make_edit`.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), EDIT_SUBCLASS);
        }
    }
    // SAFETY: default processing with the arguments we were given.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

fn ellipse(cx: f32, cy: f32, r: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: Vector2::new(cx, cy),
        radiusX: r,
        radiusY: r,
    }
}

fn rounded(r: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: r.d2d(),
        radiusX: radius,
        radiusY: radius,
    }
}

/// Drawing helpers bound to one frame.
struct Canvas<'a> {
    rt: &'a ID2D1HwndRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    fonts: &'a Fonts,
    pal: &'a Pal,
    s: f32,
}

impl Canvas<'_> {
    /// Select `color` at `alpha` and return the brush.
    fn pen(&self, color: Rgb, alpha: f32) -> &ID2D1SolidColorBrush {
        // SAFETY: the brush belongs to this render target.
        unsafe { self.brush.SetColor(&color_f(color, alpha)) };
        self.brush
    }

    fn text(&self, s: &str, font: &IDWriteTextFormat, r: Rect, color: Rgb, alpha: f32) {
        let utf16: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: valid format/brush from the same factory/target; `utf16` outlives the call.
        unsafe {
            self.rt.DrawText(
                &utf16,
                font,
                &r.d2d(),
                self.pen(color, alpha),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn circle(&self, cx: f32, cy: f32, r: f32, color: Rgb, alpha: f32) {
        // SAFETY: drawing on our own render target between BeginDraw/EndDraw.
        unsafe {
            self.rt
                .FillEllipse(&ellipse(cx, cy, r), self.pen(color, alpha))
        };
    }

    fn ring(&self, cx: f32, cy: f32, r: f32, width: f32, color: Rgb, alpha: f32) {
        // SAFETY: as above.
        unsafe {
            self.rt
                .DrawEllipse(&ellipse(cx, cy, r), self.pen(color, alpha), width, None)
        };
    }

    fn fill(&self, r: Rect, radius: f32, color: Rgb, alpha: f32) {
        // SAFETY: as above.
        unsafe {
            self.rt
                .FillRoundedRectangle(&rounded(r, radius), self.pen(color, alpha))
        };
    }

    /// 1 px outline just inside `r`.
    fn frame(&self, r: Rect, radius: f32, color: Rgb, alpha: f32) {
        let px = self.s.max(1.0);
        // SAFETY: as above.
        unsafe {
            self.rt.DrawRoundedRectangle(
                &rounded(r.inset(px / 2.0), radius),
                self.pen(color, alpha),
                px,
                None,
            )
        };
    }

    fn pill(&self, r: Rect, color: Rgb, alpha: f32) {
        self.fill(r, (r.b - r.t) / 2.0, color, alpha);
    }

    fn switch(&self, r: Rect, on: bool, accent: Rgb) {
        let s = self.s;
        if on {
            self.pill(r, accent, 1.0);
            self.circle(r.r - 10.0 * s, r.cy(), 6.0 * s, text_on(accent), 1.0);
        } else {
            self.frame(r, (r.b - r.t) / 2.0, self.pal.text, 0.6);
            self.circle(r.l + 10.0 * s, r.cy(), 5.0 * s, self.pal.text, 0.8);
        }
    }

    /// Track, fill between `lo` and `hi` (fractions), thumbs.
    fn slider(&self, r: Rect, (lo, hi): (f32, f32), two: bool, color: Rgb, look: (bool, bool)) {
        let (hot, pressed) = look;
        let (s, pal) = (self.s, self.pal);
        let cy = r.cy();
        let track =
            |a: f32, b: f32| Rect::new(a, cy - TRACK_H * s / 2.0, b, cy + TRACK_H * s / 2.0);
        self.pill(track(r.l, r.r), pal.text, 0.45);
        let (xl, xh) = (x_at(r, s, lo), x_at(r, s, hi));
        self.pill(track(if two { xl } else { r.l }, xh), color, 1.0);
        let thumbs: &[f32] = if two { &[xl, xh] } else { &[xh] };
        for &x in thumbs {
            self.circle(x, cy, THUMB_R * s, Rgb::BLACK, 0.25);
            self.circle(x, cy, (THUMB_R - 0.5) * s, pal.thumb, 1.0);
            let dot = match (pressed, hot) {
                (true, _) => 4.0,
                (_, true) => 6.0,
                _ => 5.0,
            };
            self.circle(x, cy, dot * s, color, 1.0);
        }
    }
}

impl State {
    fn ensure_fonts(&mut self) -> Option<()> {
        if self.fonts.as_ref().is_some_and(|(dpi, _)| *dpi == self.dpi) {
            return Some(());
        }
        let s = self.scale();
        let make = |family: &str, size: f32, weight, align| -> Option<IDWriteTextFormat> {
            // SAFETY: factory call with valid arguments; the text format is ours.
            unsafe {
                let f = self
                    .dwrite
                    .CreateTextFormat(
                        &HSTRING::from(family),
                        None,
                        weight,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size * s,
                        w!("en-us"),
                    )
                    .ok()?;
                f.SetTextAlignment(align).ok()?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)
                    .ok()?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP).ok()?;
                Some(f)
            }
        };
        let (t, i) = (self.text_font, self.icon_font);
        let (reg, semi) = (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD);
        let (lead, trail, mid) = (
            DWRITE_TEXT_ALIGNMENT_LEADING,
            DWRITE_TEXT_ALIGNMENT_TRAILING,
            DWRITE_TEXT_ALIGNMENT_CENTER,
        );
        let fonts = Fonts {
            body: make(t, 14.0, reg, lead)?,
            semi: make(t, 14.0, semi, lead)?,
            caption: make(t, 12.0, reg, lead)?,
            value: make(t, 14.0, reg, trail)?,
            mid: make(t, 14.0, reg, mid)?,
            glyph: make(i, 16.0, reg, mid)?,
        };
        self.fonts = Some((self.dpi, fonts));
        Some(())
    }

    fn ensure_target(&mut self) -> Option<()> {
        let mut rc = RECT::default();
        // SAFETY: valid window and out-pointer.
        unsafe { GetClientRect(self.hwnd, &mut rc).ok()? };
        let size = D2D_SIZE_U {
            width: (rc.right - rc.left).max(1) as u32,
            height: (rc.bottom - rc.top).max(1) as u32,
        };
        if let Some(t) = &self.target {
            // SAFETY: valid render target.
            unsafe {
                if t.rt.GetPixelSize() != size {
                    t.rt.Resize(&size).ok()?;
                }
            }
            return Some(());
        }
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_UNKNOWN,
                alphaMode: if self.mica {
                    D2D1_ALPHA_MODE_PREMULTIPLIED
                } else {
                    D2D1_ALPHA_MODE_IGNORE
                },
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.hwnd,
            pixelSize: size,
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        // SAFETY: valid factory and property structs.
        unsafe {
            let rt = self.d2d.CreateHwndRenderTarget(&props, &hwnd_props).ok()?;
            let brush = rt
                .CreateSolidColorBrush(&color_f(Rgb::WHITE, 1.0), None)
                .ok()?;
            self.target = Some(Target { rt, brush });
        }
        Some(())
    }

    fn paint(&mut self) {
        if self.ensure_fonts().is_none() || self.ensure_target().is_none() {
            return;
        }
        let (Some((_, fonts)), Some(target)) = (&self.fonts, &self.target) else {
            return;
        };
        let cv = Canvas {
            rt: &target.rt,
            brush: &target.brush,
            fonts,
            pal: &self.pal,
            s: self.scale(),
        };
        let layout = self.layout();
        let client = Rect::new(0.0, 0.0, WIDTH * cv.s, self.client_height());

        // SAFETY: BeginDraw/EndDraw bracket all drawing on this render target.
        unsafe {
            cv.rt.BeginDraw();
            cv.rt.Clear(Some(&color_f(Rgb::BLACK, 0.0)));
            if !self.mica {
                cv.rt
                    .FillRectangle(&client.d2d(), cv.pen(self.pal.page, 1.0));
            }
            cv.rt
                .SetTransform(&Matrix3x2::translation(0.0, -self.scroll));
        }
        for node in &layout.nodes {
            self.draw_node(&cv, node);
        }
        // SAFETY: as above.
        unsafe { cv.rt.SetTransform(&Matrix3x2::identity()) };
        let max = self.max_scroll(&layout);
        if max > 0.0 {
            // Thin scroll indicator.
            let (view, s) = (client.b, cv.s);
            let len = (view * view / layout.height).max(24.0 * s);
            let top = self.scroll / max * (view - len);
            let bar = Rect::new(client.r - 6.0 * s, top, client.r - 3.0 * s, top + len);
            cv.pill(bar, self.pal.text, 0.35);
        }

        // SAFETY: closes the BeginDraw above.
        let end = unsafe { cv.rt.EndDraw(None, None) };
        if matches!(end, Err(e) if e.code() == D2DERR_RECREATE_TARGET) {
            self.target = None;
        }
    }

    fn draw_node(&self, cv: &Canvas, node: &Node) {
        let (pal, fonts, s) = (cv.pal, cv.fonts, cv.s);
        match node {
            Node::Heading(key, r) => {
                cv.text(tr(self.lang(), *key), &fonts.semi, *r, pal.text, 1.0);
            }
            Node::Card(r) => {
                cv.fill(*r, RADIUS * 2.0 * s, pal.card.0, pal.card.1);
                cv.frame(*r, RADIUS * 2.0 * s, Rgb::BLACK, 0.12);
            }
            Node::Separator(r) => cv.fill(*r, 0.0, pal.text, 0.08),
            Node::Label {
                title,
                sub,
                rect,
                dim,
            } => {
                let a = if *dim { 0.5 } else { 1.0 };
                match sub {
                    Some(sub) => {
                        let cy = rect.cy();
                        let top = Rect::new(rect.l, cy - 19.0 * s, rect.r, cy + 1.0 * s);
                        let bottom = Rect::new(rect.l, cy + 1.0 * s, rect.r, cy + 19.0 * s);
                        cv.text(title, &fonts.body, top, pal.text, a);
                        cv.text(sub, &fonts.caption, bottom, pal.text, 0.65 * a);
                    }
                    None => cv.text(title, &fonts.body, *rect, pal.text, a),
                }
            }
            Node::Group(r) => {
                cv.fill(*r, RADIUS * s, pal.text, 0.06);
                cv.frame(*r, RADIUS * s, pal.text, 0.1);
            }
            Node::Field { rect, mon, kind } => {
                cv.fill(*rect, RADIUS * s, pal.field, 1.0);
                cv.frame(*rect, RADIUS * s, pal.text, 0.18);
                let focused = self.snap.monitors.get(*mon).is_some_and(|m| {
                    self.edits
                        .iter()
                        // SAFETY: plain focus query.
                        .any(|e| {
                            e.mon == m.id && e.kind == *kind && e.hwnd == unsafe { GetFocus() }
                        })
                });
                if focused {
                    let line =
                        Rect::new(rect.l + 2.0 * s, rect.b - 2.0 * s, rect.r - 2.0 * s, rect.b);
                    cv.fill(line, s, self.theme.accent, 1.0);
                }
            }
            Node::Value(text, r) => cv.text(text, &fonts.value, *r, pal.text, 0.85),
            Node::Ctl(ctl, r) => self.draw_ctl(cv, *ctl, *r),
        }
    }

    fn draw_ctl(&self, cv: &Canvas, ctl: Ctl, r: Rect) {
        let (pal, fonts, s) = (cv.pal, cv.fonts, cv.s);
        let accent = self.theme.accent;
        let hot = self.hover == Some(ctl);
        match ctl {
            Ctl::Toggle(t) => cv.switch(r, self.toggle_on(t), accent),
            Ctl::Choice(c, k) => {
                let label = self.choice_label(c, k);
                if self.choice_selected(c, k) {
                    cv.fill(r.inset(2.0 * s), (RADIUS - 1.0) * s, accent, 1.0);
                    cv.text(&label, &fonts.mid, r, text_on(accent), 1.0);
                } else {
                    if hot {
                        cv.fill(r.inset(2.0 * s), (RADIUS - 1.0) * s, pal.text, 0.08);
                    }
                    cv.text(&label, &fonts.mid, r, pal.text, 1.0);
                }
            }
            Ctl::Slider(slide) => {
                let (color, two) = match slide {
                    Slide::Interval => (accent, false),
                    Slide::Range(i) => (self.snap.monitors[i].color, true),
                };
                let pressed = match (slide, self.drag) {
                    (Slide::Interval, Some(Drag::Interval)) => true,
                    (Slide::Range(i), Some(Drag::Handle(j, _))) => i == j,
                    _ => false,
                };
                cv.slider(r, self.slide_fracs(slide), two, color, (hot, pressed));
            }
            Ctl::Button(Btn::Color(i)) => {
                let m = &self.snap.monitors[i];
                let a = if matches!(m.state, MonitorState::Active | MonitorState::ReadOnly) {
                    1.0
                } else {
                    0.5
                };
                cv.circle(r.cx(), r.cy(), MON_DOT_R * s, m.color, a);
                if hot || self.picker.as_ref() == Some(&m.id) {
                    cv.ring(
                        r.cx(),
                        r.cy(),
                        (MON_DOT_R + 3.0) * s,
                        1.5 * s,
                        pal.text,
                        0.8,
                    );
                }
            }
            Ctl::Button(b @ (Btn::Up(_) | Btn::Down(_))) => {
                let a = if self.enabled(ctl) { 1.0 } else { 0.3 };
                if hot {
                    cv.fill(r, RADIUS * s, pal.text, 0.08);
                }
                let glyph = if matches!(b, Btn::Up(_)) {
                    "\u{E70E}"
                } else {
                    "\u{E70D}"
                };
                cv.text(glyph, &fonts.glyph, r, pal.text, 0.9 * a);
            }
            Ctl::Button(b) => {
                let key = if b == Btn::CheckNow {
                    Key::BtnCheckNow
                } else {
                    Key::BtnOpenConfig
                };
                cv.fill(r, RADIUS * s, pal.text, if hot { 0.12 } else { 0.06 });
                cv.frame(r, RADIUS * s, pal.text, 0.12);
                cv.text(tr(self.lang(), key), &fonts.mid, r, pal.text, 1.0);
            }
            Ctl::Swatch(i, k) => {
                let current = self.snap.monitors[i].color == PALETTE[k];
                cv.circle(r.cx(), r.cy(), SWATCH_R * s, PALETTE[k], 1.0);
                if current {
                    cv.ring(r.cx(), r.cy(), (SWATCH_R + 3.0) * s, 1.5 * s, pal.text, 1.0);
                } else if hot {
                    cv.ring(r.cx(), r.cy(), (SWATCH_R + 3.0) * s, 1.0 * s, pal.text, 0.5);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

fn set_dark(hwnd: HWND, dark: bool) {
    let value = i32::from(dark);
    // SAFETY: valid window handle; the attribute value outlives the call.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const i32 as *const _,
            size_of::<i32>() as u32,
        );
    }
}

/// Create the window on the monitor under the cursor, centered, and its state.
fn create(snap: Snapshot) -> Option<HWND> {
    ON_ACTION.get()?;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    // SAFETY: plain Win32/COM creation; every pointer passed outlives its call.
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let mon = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dpi, mut dpi_y) = (96, 96);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
        let work = info.rcWork;

        // Outer size for a client of `ch` pixels height; shrink the client to fit the screen.
        let s = dpi as f32 / 96.0;
        let cw = (WIDTH * s).round() as i32;
        let mut ch = (HEIGHT * s).round() as i32;
        let outer = |ch: i32| {
            let mut rc = RECT {
                left: 0,
                top: 0,
                right: cw,
                bottom: ch,
            };
            let _ = AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE(0), dpi);
            (rc.right - rc.left, rc.bottom - rc.top)
        };
        let (mut w, mut h) = outer(ch);
        let avail = work.bottom - work.top;
        if h > avail {
            ch -= h - avail;
            (w, h) = outer(ch);
        }
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (work.bottom - work.top - h) / 2;

        let hinst = HINSTANCE(GetModuleHandleW(None).ok()?.0);
        let title = util::wide(tr(snap.lang, Key::SettingsTitle));
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            PCWSTR(title.as_ptr()),
            style,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hinst),
            None,
        )
        .ok()?;

        // Windows 11: Mica behind the whole window. If DWM rejects any of it, paint opaque.
        let theme = Theme::read();
        set_dark(hwnd, !theme.apps_light);
        let backdrop = DWMSBT_MAINWINDOW.0;
        let mica = theme.is_win11
            && DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop as *const i32 as *const _,
                size_of::<i32>() as u32,
            )
            .is_ok()
            && DwmExtendFrameIntoClientArea(
                hwnd,
                &MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                },
            )
            .is_ok();

        let state = (|| {
            let d2d: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
            let text_font = if has_family(&dwrite, "Segoe UI Variable Text") {
                "Segoe UI Variable Text"
            } else {
                "Segoe UI"
            };
            let icon_font = if has_family(&dwrite, "Segoe Fluent Icons") {
                "Segoe Fluent Icons"
            } else {
                "Segoe MDL2 Assets"
            };
            let pal = pal(theme.apps_light, mica);
            Some(State {
                hwnd,
                hinst,
                d2d,
                dwrite,
                text_font,
                icon_font,
                mica,
                fonts: None,
                target: None,
                gdi_font: None,
                edit_brush: CreateSolidBrush(colorref(pal.field)),
                edits: Vec::new(),
                title_lang: snap.lang,
                snap,
                theme,
                pal,
                dpi,
                scroll: 0.0,
                hover: None,
                drag: None,
                tracking_leave: false,
                picker: None,
            })
        })();
        let Some(state) = state else {
            let _ = DestroyWindow(hwnd);
            return None;
        };
        STATE.with(|cell| *cell.borrow_mut() = Some(state));
        with_state(State::sync_all);
        Some(hwnd)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // SAFETY: the window (and with it every EDIT) is gone; the objects are ours.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.edit_brush.0));
            if let Some((_, font)) = self.gdi_font {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
    }
}

fn point(lparam: LPARAM) -> (f32, f32) {
    (
        f32::from(lparam.0 as u16 as i16),
        f32::from((lparam.0 >> 16) as u16 as i16),
    )
}

/// Whether a `WM_SETTINGCHANGE` lparam is the "ImmersiveColorSet" string.
fn is_color_change(lparam: LPARAM) -> bool {
    // SAFETY: for WM_SETTINGCHANGE a non-zero lparam is a NUL-terminated wide string.
    lparam.0 != 0
        && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }
            .is_ok_and(|s| s == "ImmersiveColorSet")
}

/// Ask for one `WM_MOUSELEAVE` while the cursor is over the window.
fn track_leave(hwnd: HWND) {
    if with_state(|st| std::mem::replace(&mut st.tracking_leave, true)) != Some(false) {
        return;
    }
    let mut tme = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    // SAFETY: `tme` is valid for the call.
    unsafe {
        let _ = TrackMouseEvent(&mut tme);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            with_state(State::paint);
            // SAFETY: our own window.
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_SIZE => {
            with_state(State::sync_all);
            invalidate(hwnd);
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            with_state(|st| st.set_dpi(dpi));
            // SAFETY: for WM_DPICHANGED, lparam points to the suggested window rectangle.
            unsafe {
                let r = *(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            with_state(State::sync_all);
            invalidate(hwnd);
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            let action = with_state(|st| st.mouse_move(x, y)).flatten();
            track_leave(hwnd);
            emit(action);
            invalidate(hwnd);
        }
        WM_MOUSELEAVE => {
            with_state(|st| {
                st.hover = None;
                st.tracking_leave = false;
            });
            invalidate(hwnd);
        }
        WM_LBUTTONDOWN => {
            // Taking the focus commits a pending EDIT (and may change the layout) first.
            // SAFETY: our own window.
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
            let (x, y) = point(lparam);
            let (action, dragging) =
                with_state(|st| (st.mouse_down(x, y), st.drag.is_some())).unwrap_or_default();
            if dragging {
                // SAFETY: our own window.
                unsafe { SetCapture(hwnd) };
            }
            emit(action);
            invalidate(hwnd);
        }
        WM_LBUTTONUP => {
            with_state(|st| st.drag = None);
            // SAFETY: releases a capture we may hold.
            unsafe {
                let _ = ReleaseCapture();
            }
            invalidate(hwnd);
        }
        WM_CAPTURECHANGED => {
            with_state(|st| st.drag = None);
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam.0 >> 16) as u16 as i16);
            if with_state(|st| st.scroll_by(delta)) == Some(true) {
                invalidate(hwnd);
            }
        }
        WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
            close(hwnd);
        }
        WM_CLOSE => close(hwnd),
        WM_COMMAND => {
            let edit = HWND(lparam.0 as *mut c_void);
            match ((wparam.0 >> 16) & 0xFFFF) as u32 {
                EN_SETFOCUS => invalidate(hwnd),
                EN_KILLFOCUS => {
                    let action = with_state(|st| st.commit(edit)).flatten();
                    invalidate(hwnd);
                    emit(action);
                }
                _ => {}
            }
        }
        WM_REVERT => {
            let edit = HWND(lparam.0 as *mut c_void);
            with_state(|st| st.restore_text(edit));
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wparam.0 as *mut c_void);
            // SAFETY: `hdc` is the EDIT's device context for the duration of this message.
            let brush = with_state(|st| unsafe {
                SetTextColor(hdc, colorref(st.pal.text));
                SetBkColor(hdc, colorref(st.pal.field));
                st.edit_brush
            });
            if let Some(brush) = brush {
                return LRESULT(brush.0 as isize);
            }
            // SAFETY: default processing with the arguments we were given.
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_SETTINGCHANGE if is_color_change(lparam) => {
            with_state(State::on_theme_change);
            invalidate(hwnd);
        }
        WM_DESTROY => {
            // Dropping the state frees the Direct2D target and the GDI objects.
            STATE.with(|cell| cell.try_borrow_mut().ok().and_then(|mut s| s.take()));
        }
        // SAFETY: default processing with the arguments we were given.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// Close the window; taking the focus first commits a pending EDIT.
fn close(hwnd: HWND) {
    // SAFETY: our own window; no state borrow is held.
    unsafe {
        let _ = SetFocus(Some(hwnd));
        let _ = DestroyWindow(hwnd);
    }
}
