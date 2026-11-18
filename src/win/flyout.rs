//! The flyout opened by a left click on a tray icon (spec §4.4, §5): one slider per monitor,
//! an inline color picker, calibrated "Link all", Rescan and Settings.
//!
//! Layout of this file:
//! - geometry: `Layout` is computed in one place from the state; hit-testing and painting both read it.
//! - `State`: the single thread-local state (rows, interaction, graphics) and its input methods.
//! - painting with Direct2D/DirectWrite.
//! - window creation, window procedure and the public API.
//!
//! Re-entrancy: `show`/`update`/`hide` run while the app is borrowed, so they never call
//! `on_action`. The window procedure releases the state borrow before it emits an action,
//! because the app's handler calls back into `update`/`hide`.

use super::icon::{color_f, has_family};
use super::theme::Theme;
use super::util;
#[cfg(feature = "plus")]
use crate::auto::color::{MAX_K, kelvin_step, step_kelvin, tint, ui_steps};
use crate::color::{PALETTE, Rgb};
#[cfg(feature = "plus")]
use crate::i18n::PlusKey;
use crate::i18n::{Key, Lang, tr};
use crate::link::link_master;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWMWINDOWATTRIBUTE, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    ScreenToClient, ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT, VK_SHIFT, VK_TAB,
    VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, HWND_TOPMOST, IDC_ARROW,
    LoadCursorW, RegisterClassExW, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetForegroundWindow, SetWindowPos, WA_INACTIVE, WM_ACTIVATE, WM_CAPTURECHANGED, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{HRESULT, HSTRING, PCWSTR, w};
use windows_numerics::Vector2;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// One monitor as the flyout shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlyoutRow {
    pub uid: u32,
    pub name: String,
    pub color: Rgb,
    pub value: Option<u8>,
    /// `false`: unresponsive (dimmed, slider disabled).
    pub enabled: bool,
    /// Calibrated range for "Link all" (0..=100).
    pub link_min: u8,
    pub link_max: u8,
}

/// Plus: the color temperature slider as the flyout shows it.
#[cfg(feature = "plus")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorRow {
    /// What the displays show now (kelvin).
    pub kelvin: u32,
    /// The warm end of the slider (kelvin).
    pub min_k: u32,
}

/// What the user asked for; delivered through the function given to [`init`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    SetBrightness(u32, u8),
    /// The master slider moved (0..=100).
    SetAll(u8),
    SetColor(u32, Rgb),
    SetLinked(bool),
    /// A monitor's calibrated range changed: `(uid, min, max)`.
    SetLinkRange(u32, u8, u8),
    /// Plus: the color temperature slider is being dragged (kelvin): show it, nothing more.
    #[cfg(feature = "plus")]
    PreviewKelvin(u32),
    /// Plus: the color temperature was chosen (kelvin): drag released, wheel or key.
    #[cfg(feature = "plus")]
    SetKelvin(u32),
    Rescan,
    Settings,
}

static ON_ACTION: OnceLock<fn(Action)> = OnceLock::new();

/// Register the window class. Once, on the UI thread. The window itself, and every
/// Direct2D/DirectWrite object behind it, only exists while the flyout is open.
pub fn init(on_action: fn(Action)) {
    let _ = ON_ACTION.set(on_action);
    // SAFETY: plain class registration; the class name is copied by the call.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let class = class_name();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(module.0),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

fn class_name() -> Vec<u16> {
    util::wide("NitorFlyout")
}

/// Open the flyout near `anchor` (screen coordinates of the clicked tray icon).
/// `color` (Plus): the color temperature slider; `None` hides it.
pub fn show(
    anchor: POINT,
    rows: Vec<FlyoutRow>,
    linked: bool,
    lang: Lang,
    #[cfg(feature = "plus")] color: Option<ColorRow>,
) {
    // A click on the tray icon first deactivates (closes) the flyout, then arrives here: ignore it.
    if is_visible() || CLOSED_AT.get().is_some_and(|t| t.elapsed() < REOPEN_GUARD) {
        return;
    }
    let Some(mut st) = State::create() else {
        return;
    };
    st.open(
        anchor,
        rows,
        linked,
        lang,
        #[cfg(feature = "plus")]
        color,
    );
    let hwnd = st.hwnd;
    STATE.with(|s| *s.borrow_mut() = Some(st));
    let (x, y, w, h) = sync_geometry().unwrap_or_default();
    // SAFETY: plain window calls on our own window; the state borrow is released.
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

/// New values/colors while open. Does not disturb a drag in progress.
pub fn update(rows: Vec<FlyoutRow>, #[cfg(feature = "plus")] color: Option<ColorRow>) {
    let changed = with_state(|st| {
        st.merge(
            rows,
            #[cfg(feature = "plus")]
            color,
        )
    });
    if changed == Some(true) {
        refresh_window();
    }
}

/// The system theme changed: re-read it and restyle (also while open).
pub fn retheme() {
    let open = with_state(|st| {
        st.theme = Theme::read();
        st.apply_theme();
    });
    if open.is_some() {
        refresh_window();
    }
}

pub fn hide() {
    close();
}

/// The flyout exists exactly while it is open (a borrowed state counts as open).
pub fn is_visible() -> bool {
    STATE.with(|s| !matches!(s.try_borrow(), Ok(s) if s.is_none()))
}

// ---------------------------------------------------------------------------
// Metrics (DIP at 100%) and geometry
// ---------------------------------------------------------------------------

const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
const MARGIN: f32 = 12.0;
const HEADER_TOP: f32 = 12.0;
const HEADER_H: f32 = 28.0;
const CAPTION_H: f32 = 24.0;
const PICKER_H: f32 = 28.0;
const SLIDER_H: f32 = 48.0;
const FOOTER_H: f32 = 52.0;
/// The color temperature slider under its caption.
#[cfg(feature = "plus")]
const COLOR_H: f32 = 40.0;
const DOT: f32 = 8.0;
const GLYPH_W: f32 = 24.0;
const VALUE_W: f32 = 40.0;
const THUMB_R: f32 = 10.0;
const TRACK_H: f32 = 4.0;
/// Track center below the slider row's top; the range handles sit under the thumb.
const TRACK_Y: f32 = 20.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 20.0;
const BUTTON: f32 = 32.0;
const SWATCH_R: f32 = 9.0;
const SWATCH_STEP: f32 = 28.0;
const WHEEL_STEP: i32 = 5;
/// Ignore a re-open this soon after an automatic close (the tray click that closed it).
/// `WM_MOUSELEAVE` (not exposed by the bindings).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// `D2DERR_RECREATE_TARGET`: the render target must be recreated.
const D2DERR_RECREATE_TARGET: HRESULT = HRESULT(0x8899000Cu32 as i32);
const REOPEN_GUARD: Duration = Duration::from_millis(250);

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
    fn cy(&self) -> f32 {
        (self.t + self.b) / 2.0
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

/// A slider row. `x0..x1` is the range of the thumb center; the track extends a thumb radius beyond.
#[derive(Clone, Copy, Default)]
struct SliderGeom {
    glyph: Rect,
    value: Rect,
    x0: f32,
    x1: f32,
    cy: f32,
    /// Everything that starts a drag (the track and thumb).
    grab: Rect,
    scale: f32,
    /// The largest value (the right end): 100, or the color slider's steps.
    max: u8,
}

impl SliderGeom {
    fn x_of(&self, v: u8) -> f32 {
        self.x0 + (self.x1 - self.x0) * f32::from(v.min(self.max)) / f32::from(self.max)
    }
    fn value_at(&self, x: f32) -> u8 {
        (((x - self.x0) / (self.x1 - self.x0)) * f32::from(self.max))
            .round()
            .clamp(0.0, f32::from(self.max)) as u8
    }
    /// Hit area of a calibration handle for `v`, below the thumb.
    fn handle_rect(&self, v: u8) -> Rect {
        let s = self.scale;
        Rect::centered(self.x_of(v), self.cy + 17.0 * s, 16.0 * s, 14.0 * s)
    }
}

/// Caption row (+ optional swatch row) and slider of one monitor, or of the master.
#[derive(Clone, Default)]
struct Block {
    dot: Rect,
    name: Rect,
    swatches: Vec<Rect>,
    slider: SliderGeom,
}

#[derive(Default)]
struct Layout {
    width: f32,
    height: f32,
    scale: f32,
    header: Rect,
    master: Option<Block>,
    monitors: Vec<Block>,
    /// Plus: caption and slider of the color temperature.
    #[cfg(feature = "plus")]
    color: Option<(Rect, SliderGeom)>,
    footer: Rect,
    link: Rect,
    switch: Rect,
    rescan: Rect,
    settings: Rect,
    /// Where the hovered button's label goes.
    hint: Rect,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slider {
    Master,
    Row(usize),
    /// Plus: color temperature, in steps of 50 K.
    #[cfg(feature = "plus")]
    Color,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Handle {
    Min,
    Max,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Slider(Slider),
    Handle(usize, Handle),
    Dot(usize),
    Swatch(usize, usize),
    Link,
    Rescan,
    Settings,
}

impl Layout {
    fn slider(&self, id: Slider) -> Option<&SliderGeom> {
        match id {
            Slider::Master => self.master.as_ref().map(|b| &b.slider),
            Slider::Row(i) => self.monitors.get(i).map(|b| &b.slider),
            #[cfg(feature = "plus")]
            Slider::Color => self.color.as_ref().map(|c| &c.1),
        }
    }

    fn hit(&self, rows: &[FlyoutRow], linked: bool, x: f32, y: f32) -> Hit {
        for (i, b) in self.monitors.iter().enumerate() {
            let row = &rows[i];
            if let Some(k) = b.swatches.iter().position(|r| r.contains(x, y)) {
                return Hit::Swatch(i, k);
            }
            if b.dot.contains(x, y) {
                return Hit::Dot(i);
            }
            if !row.enabled {
                continue;
            }
            if linked {
                for (h, v) in [(Handle::Min, row.link_min), (Handle::Max, row.link_max)] {
                    if b.slider.handle_rect(v).contains(x, y) {
                        return Hit::Handle(i, h);
                    }
                }
            }
            if b.slider.grab.contains(x, y) {
                return Hit::Slider(Slider::Row(i));
            }
        }
        if self
            .master
            .as_ref()
            .is_some_and(|m| m.slider.grab.contains(x, y))
        {
            return Hit::Slider(Slider::Master);
        }
        #[cfg(feature = "plus")]
        if self.color.as_ref().is_some_and(|c| c.1.grab.contains(x, y)) {
            return Hit::Slider(Slider::Color);
        }
        if self.link.contains(x, y) {
            Hit::Link
        } else if self.rescan.contains(x, y) {
            Hit::Rescan
        } else if self.settings.contains(x, y) {
            Hit::Settings
        } else {
            Hit::None
        }
    }
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

struct Colors {
    /// Background tint: translucent over the acrylic, opaque otherwise.
    bg: (Rgb, f32),
    /// Black overlay of the footer band.
    footer: f32,
    text: Rgb,
    thumb: Rgb,
}

fn colors(light: bool, acrylic: bool) -> Colors {
    let (bg, text, thumb, footer) = if light {
        (
            Rgb::new(0xF9, 0xF9, 0xF9),
            Rgb::new(0x1B, 0x1B, 0x1B),
            Rgb::WHITE,
            0.05,
        )
    } else {
        (
            Rgb::new(0x2C, 0x2C, 0x2C),
            Rgb::WHITE,
            Rgb::new(0x45, 0x45, 0x45),
            0.20,
        )
    };
    Colors {
        bg: (bg, if acrylic { 0.6 } else { 1.0 }),
        footer,
        text,
        thumb,
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    Slider(Slider),
    Handle(usize, Handle),
}

/// Where the flyout sits: a corner of a monitor's work area.
#[derive(Clone, Copy, Default)]
struct Placement {
    work: RECT,
    right: bool,
    bottom: bool,
}

struct Fonts {
    header: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    hint: IDWriteTextFormat,
    value: IDWriteTextFormat,
    glyph: IDWriteTextFormat,
}

struct Target {
    rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
}

struct State {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    text_font: &'static str,
    icon_font: &'static str,
    /// Win11 backdrop + corners were accepted by DWM.
    acrylic: bool,
    fonts: Option<(u32, Fonts)>,
    target: Option<Target>,

    rows: Vec<FlyoutRow>,
    linked: bool,
    lang: Lang,
    theme: Theme,
    dpi: u32,
    place: Placement,
    /// Window rectangle last applied (x, y, w, h).
    applied: (i32, i32, i32, i32),

    master: u8,
    /// The master was moved by the user: keep it instead of deriving it from the rows.
    master_touched: bool,
    /// Plus: color temperature slider position (`None`: no slider).
    #[cfg(feature = "plus")]
    color: Option<u8>,
    /// Plus: the warm end of the color slider (kelvin).
    #[cfg(feature = "plus")]
    color_min: u32,
    /// Plus: the color slider moved during the current drag (a final value is due).
    #[cfg(feature = "plus")]
    color_moved: bool,
    picker: Option<usize>,
    focus: Option<Slider>,
    focus_visible: bool,
    hover: Hit,
    tracking_leave: bool,
    drag: Option<Drag>,
    wheel_rem: i32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    /// When the flyout last closed; outside `State`, which is destroyed on close.
    static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
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

impl State {
    /// The sliders the keyboard can reach, in Tab order.
    fn focusable(&self) -> Vec<Slider> {
        let rows = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.enabled && r.value.is_some())
            .map(|(i, _)| Slider::Row(i));
        let order = self
            .linked
            .then_some(Slider::Master)
            .into_iter()
            .chain(rows);
        #[cfg(feature = "plus")]
        let order = order.chain(self.color.map(|_| Slider::Color));
        order.collect()
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    // --- geometry ---------------------------------------------------------

    fn layout(&self) -> Layout {
        let s = self.scale();
        let width = (WIDTH * s).round();
        let slider_at = |top: f32| {
            let cy = top + TRACK_Y * s;
            let x0 = (PAD + GLYPH_W + 12.0 + THUMB_R) * s;
            SliderGeom {
                glyph: Rect::new(PAD * s, cy - 12.0 * s, (PAD + GLYPH_W) * s, cy + 12.0 * s),
                value: Rect::new(
                    width - (PAD + VALUE_W) * s,
                    cy - 12.0 * s,
                    width - PAD * s,
                    cy + 12.0 * s,
                ),
                x0,
                x1: width - (PAD + VALUE_W + 12.0 + THUMB_R) * s,
                cy,
                grab: Rect::new(
                    x0 - (THUMB_R + 2.0) * s,
                    cy - 14.0 * s,
                    width - (PAD + VALUE_W + 12.0 - 2.0) * s,
                    cy + 11.0 * s,
                ),
                scale: s,
                max: 100,
            }
        };
        let block = |top: f32, swatches: bool| {
            let mid = top + CAPTION_H * s / 2.0;
            let mut b = Block {
                dot: Rect::centered((PAD + DOT / 2.0) * s, mid, 24.0 * s, 24.0 * s),
                name: Rect::new(
                    (PAD + DOT + 8.0) * s,
                    top,
                    width - PAD * s,
                    top + CAPTION_H * s,
                ),
                ..Block::default()
            };
            let mut slider_top = top + CAPTION_H * s;
            if swatches {
                let cy = slider_top + PICKER_H * s / 2.0;
                b.swatches = (0..PALETTE.len())
                    .map(|k| {
                        let cx = (PAD + DOT / 2.0 + k as f32 * SWATCH_STEP) * s;
                        Rect::centered(cx, cy, 24.0 * s, 24.0 * s)
                    })
                    .collect();
                slider_top += PICKER_H * s;
            }
            b.slider = slider_at(slider_top);
            (b, slider_top + SLIDER_H * s)
        };

        let mut y = (HEADER_TOP + HEADER_H) * s;
        let header = Rect::new(PAD * s, HEADER_TOP * s, width - PAD * s, y);
        let master = self.linked.then(|| {
            let (b, bottom) = block(y, false);
            y = bottom;
            b
        });
        let monitors = (0..self.rows.len())
            .map(|i| {
                let (b, bottom) = block(y, self.picker == Some(i));
                y = bottom;
                b
            })
            .collect();
        #[cfg(feature = "plus")]
        let color = self.color.map(|_| {
            let caption = Rect::new(PAD * s, y, width - PAD * s, y + CAPTION_H * s);
            let slider = SliderGeom {
                max: ui_steps(self.color_min),
                ..slider_at(y + CAPTION_H * s)
            };
            y += (CAPTION_H + COLOR_H) * s;
            (caption, slider)
        });
        let footer = Rect::new(0.0, y, width, y + FOOTER_H * s);
        let cy = footer.cy();
        let button = |offset: f32| {
            Rect::centered(
                (WIDTH - PAD - BUTTON / 2.0 - offset) * s,
                cy,
                BUTTON * s,
                BUTTON * s,
            )
        };
        let settings = button(0.0);
        let rescan = button(BUTTON + 4.0);
        Layout {
            width,
            height: footer.b.ceil(),
            scale: s,
            header,
            master,
            monitors,
            #[cfg(feature = "plus")]
            color,
            footer,
            link: Rect::new(
                PAD * s,
                cy - 16.0 * s,
                (PAD + SWITCH_W + 8.0 + 84.0) * s,
                cy + 16.0 * s,
            ),
            switch: Rect::new(
                PAD * s,
                cy - SWITCH_H * s / 2.0,
                (PAD + SWITCH_W) * s,
                cy + SWITCH_H * s / 2.0,
            ),
            rescan,
            settings,
            hint: Rect::new(
                (PAD + SWITCH_W + 8.0 + 92.0) * s,
                cy - 12.0 * s,
                rescan.l - 8.0 * s,
                cy + 12.0 * s,
            ),
        }
    }

    fn window_size(&self) -> (i32, i32) {
        let l = self.layout();
        (l.width as i32, l.height as i32)
    }

    /// Window origin for a given size: the pinned corner of the work area, inset by the margin.
    fn origin(&self, w: i32, h: i32) -> (i32, i32) {
        let (work, m) = (self.place.work, (MARGIN * self.scale()).round() as i32);
        let x = if self.place.right {
            work.right - w - m
        } else {
            work.left + m
        };
        let y = if self.place.bottom {
            work.bottom - h - m
        } else {
            work.top + m
        };
        (
            x.clamp(work.left, (work.right - w).max(work.left)),
            y.clamp(work.top, (work.bottom - h).max(work.top)),
        )
    }

    // --- opening and updating --------------------------------------------

    fn open(
        &mut self,
        anchor: POINT,
        rows: Vec<FlyoutRow>,
        linked: bool,
        lang: Lang,
        #[cfg(feature = "plus")] color: Option<ColorRow>,
    ) {
        self.rows = rows;
        self.linked = linked;
        self.lang = lang;
        self.picker = None;
        self.hover = Hit::None;
        self.drag = None;
        self.wheel_rem = 0;
        self.master_touched = false;
        #[cfg(feature = "plus")]
        {
            self.set_color(color);
            self.color_moved = false;
        }
        self.recompute_master();
        self.focus = self.focusable().first().copied();
        self.focus_visible = false;

        // SAFETY: plain monitor queries with valid out-pointers.
        let (dpi, work, center) = unsafe {
            let mon = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(mon, &mut info);
            let (mut dx, mut dy) = (96, 96);
            let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            (dx, info.rcWork, info.rcWork)
        };
        self.dpi = dpi;
        self.place = Placement {
            work,
            right: anchor.x > (center.left + center.right) / 2,
            bottom: anchor.y > (center.top + center.bottom) / 2,
        };
        self.apply_theme();
    }

    /// Dark/light frame, then the backdrop: both decide the acrylic's base color, so the
    /// dark flag goes first and the backdrop is re-applied after it. Before the window is shown.
    fn apply_theme(&self) {
        set_dwm_int(
            self.hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            i32::from(!self.theme.taskbar_light),
        );
        if self.acrylic {
            set_dwm_int(
                self.hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                DWMSBT_TRANSIENTWINDOW.0,
            );
        }
    }

    /// Take the color slider's position and range.
    #[cfg(feature = "plus")]
    fn set_color(&mut self, color: Option<ColorRow>) {
        if let Some(c) = color {
            self.color_min = c.min_k;
        }
        self.color = color.map(|c| kelvin_step(c.kelvin, c.min_k));
    }

    /// Take new rows; returns whether anything visible changed. Values and link ranges of
    /// the control being dragged are kept, the user's hand wins over a late read.
    fn merge(
        &mut self,
        mut rows: Vec<FlyoutRow>,
        #[cfg(feature = "plus")] color: Option<ColorRow>,
    ) -> bool {
        #[cfg(feature = "plus")]
        let color_changed = {
            let before = (self.color, self.color_min);
            if self.drag != Some(Drag::Slider(Slider::Color)) {
                self.set_color(color);
            }
            before != (self.color, self.color_min)
        };
        #[cfg(not(feature = "plus"))]
        let color_changed = false;
        let same_monitors = rows.len() == self.rows.len()
            && rows.iter().zip(&self.rows).all(|(a, b)| a.uid == b.uid);
        if same_monitors {
            match self.drag {
                Some(Drag::Slider(Slider::Row(i))) => rows[i].value = self.rows[i].value,
                Some(Drag::Handle(i, _)) => {
                    rows[i].link_min = self.rows[i].link_min;
                    rows[i].link_max = self.rows[i].link_max;
                }
                _ => {}
            }
        } else {
            self.picker = None;
            self.drag = None;
            self.focus = None;
        }
        if rows == self.rows {
            return color_changed;
        }
        self.rows = rows;
        if !self.master_touched {
            self.recompute_master();
        }
        true
    }

    /// Master position = average over the responsive monitors of where their value sits in their range.
    fn recompute_master(&mut self) {
        let positions: Vec<u32> = self
            .rows
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|r| {
                r.value
                    .map(|v| u32::from(link_master(v, r.link_min, r.link_max)))
            })
            .collect();
        if !positions.is_empty() {
            let n = positions.len() as u32;
            self.master = ((positions.iter().sum::<u32>() + n / 2) / n) as u8;
        }
    }

    // --- input ------------------------------------------------------------

    fn hit(&self, x: i32, y: i32) -> Hit {
        self.layout()
            .hit(&self.rows, self.linked, x as f32, y as f32)
    }

    fn value_of(&self, id: Slider) -> Option<u8> {
        match id {
            Slider::Master => Some(self.master),
            Slider::Row(i) => self.rows.get(i).and_then(|r| r.value),
            #[cfg(feature = "plus")]
            Slider::Color => self.color,
        }
    }

    /// The largest value of slider `id`.
    fn max_of(&self, id: Slider) -> u8 {
        match id {
            #[cfg(feature = "plus")]
            Slider::Color => ui_steps(self.color_min),
            _ => 100,
        }
    }

    fn set_value(&mut self, id: Slider, v: u8) -> Option<Action> {
        let v = v.min(self.max_of(id));
        if self.value_of(id) == Some(v) {
            return None;
        }
        match id {
            Slider::Master => {
                self.master = v;
                self.master_touched = true;
                Some(Action::SetAll(v))
            }
            Slider::Row(i) => {
                self.master_touched = false;
                let row = self.rows.get_mut(i)?;
                row.value = Some(v);
                Some(Action::SetBrightness(row.uid, v))
            }
            #[cfg(feature = "plus")]
            Slider::Color => {
                self.color = Some(v);
                let kelvin = step_kelvin(v, self.color_min);
                // While dragging only a preview goes out; the release sends the value.
                if self.drag == Some(Drag::Slider(Slider::Color)) {
                    self.color_moved = true;
                    Some(Action::PreviewKelvin(kelvin))
                } else {
                    Some(Action::SetKelvin(kelvin))
                }
            }
        }
    }

    /// Move one calibration handle, keeping min <= max.
    fn set_range(&mut self, i: usize, which: Handle, v: u8) -> Option<Action> {
        let row = self.rows.get_mut(i)?;
        let (lo, hi) = match which {
            Handle::Min => (v.min(row.link_max), row.link_max),
            Handle::Max => (row.link_min, v.max(row.link_min)),
        };
        if (lo, hi) == (row.link_min, row.link_max) {
            return None;
        }
        (row.link_min, row.link_max) = (lo, hi);
        // Calibrating: hold the master still instead of re-deriving it from the rows.
        self.master_touched = true;
        Some(Action::SetLinkRange(row.uid, lo, hi))
    }

    /// The mouse button went up: end the drag; a moved color slider gets its final value.
    fn release(&mut self) -> Option<Action> {
        self.drag = None;
        #[cfg(feature = "plus")]
        if std::mem::take(&mut self.color_moved) {
            return self
                .color
                .map(|s| Action::SetKelvin(step_kelvin(s, self.color_min)));
        }
        None
    }

    fn drag_to(&mut self, x: i32) -> Option<Action> {
        let layout = self.layout();
        match self.drag? {
            Drag::Slider(id) => {
                let v = layout.slider(id)?.value_at(x as f32);
                self.set_value(id, v)
            }
            Drag::Handle(i, which) => {
                let v = layout.slider(Slider::Row(i))?.value_at(x as f32);
                self.set_range(i, which, v)
            }
        }
    }

    fn mouse_down(&mut self, x: i32, y: i32) -> Option<Action> {
        self.focus_visible = false;
        match self.hit(x, y) {
            Hit::Slider(id) => {
                self.focus = Some(id);
                self.drag = Some(Drag::Slider(id));
                self.drag_to(x)
            }
            Hit::Handle(i, which) => {
                self.drag = Some(Drag::Handle(i, which));
                None
            }
            Hit::Dot(i) => {
                self.picker = if self.picker == Some(i) {
                    None
                } else {
                    Some(i)
                };
                None
            }
            Hit::Swatch(i, k) => {
                self.picker = None;
                let row = self.rows.get_mut(i)?;
                row.color = PALETTE[k];
                Some(Action::SetColor(row.uid, PALETTE[k]))
            }
            Hit::Link => {
                self.linked = !self.linked;
                self.master_touched = false;
                self.recompute_master();
                if !self.linked && self.focus == Some(Slider::Master) {
                    self.focus = None;
                }
                Some(Action::SetLinked(self.linked))
            }
            Hit::Rescan => Some(Action::Rescan),
            Hit::Settings => Some(Action::Settings),
            Hit::None => None,
        }
    }

    fn wheel(&mut self, x: i32, y: i32, delta: i32, fine: bool) -> Option<Action> {
        let id = match self.hit(x, y) {
            Hit::Slider(id) => id,
            _ => self.focus?,
        };
        self.wheel_rem += delta;
        let notches = self.wheel_rem / 120;
        self.wheel_rem -= notches * 120;
        // A color notch is 100 K, or 50 K with Shift: two steps or one.
        let step = match (id, fine) {
            (_, true) => 1,
            #[cfg(feature = "plus")]
            (Slider::Color, false) => 2,
            _ => WHEEL_STEP,
        };
        self.nudge(id, notches * step)
    }

    fn nudge(&mut self, id: Slider, delta: i32) -> Option<Action> {
        let v = i32::from(self.value_of(id)?) + delta;
        self.set_value(id, v.clamp(0, i32::from(self.max_of(id))) as u8)
    }

    /// Keyboard: the resulting action, and whether to close the flyout.
    fn key(&mut self, vk: u16, shift: bool) -> (Option<Action>, bool) {
        self.focus_visible = true;
        if vk == VK_ESCAPE.0 {
            return (None, true);
        }
        let order = self.focusable();
        if vk == VK_TAB.0 {
            if !order.is_empty() {
                let pos = order.iter().position(|&s| Some(s) == self.focus);
                let n = order.len();
                let next = match (pos, shift) {
                    (None, _) => 0,
                    (Some(p), false) => (p + 1) % n,
                    (Some(p), true) => (p + n - 1) % n,
                };
                self.focus = Some(order[next]);
            }
            return (None, false);
        }
        let Some(id) = self.focus.filter(|f| order.contains(f)) else {
            return (None, false);
        };
        let action = match vk {
            v if v == VK_LEFT.0 || v == VK_DOWN.0 => self.nudge(id, -1),
            v if v == VK_RIGHT.0 || v == VK_UP.0 => self.nudge(id, 1),
            v if v == VK_PRIOR.0 => self.nudge(id, 10),
            v if v == VK_NEXT.0 => self.nudge(id, -10),
            v if v == VK_HOME.0 => self.set_value(id, 0),
            v if v == VK_END.0 => self.set_value(id, self.max_of(id)),
            _ => None,
        };
        (action, false)
    }
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
    colors: &'a Colors,
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

    fn pill(&self, r: Rect, color: Rgb, alpha: f32) {
        let radius = (r.b - r.t) / 2.0;
        // SAFETY: as above.
        unsafe {
            self.rt
                .FillRoundedRectangle(&rounded(r, radius), self.pen(color, alpha))
        };
    }

    fn caption(&self, b: &Block, name: &str, color: Rgb, alpha: f32) {
        let c = self.colors;
        self.circle(
            b.dot.l + (b.dot.r - b.dot.l) / 2.0,
            b.dot.cy(),
            DOT / 2.0 * self.s,
            color,
            alpha,
        );
        self.text(name, &self.fonts.caption, b.name, c.text, 0.75 * alpha);
    }

    /// Sun glyph, track, fill, thumb, value, optional focus ring and calibration handles.
    #[allow(clippy::too_many_arguments)]
    fn slider(
        &self,
        g: &SliderGeom,
        value: Option<u8>,
        color: Rgb,
        enabled: bool,
        look: (bool, bool, bool),
        range: Option<(u8, u8)>,
    ) {
        let (hot, pressed, focused) = look;
        let (c, s) = (self.colors, self.s);
        let a = if enabled { 1.0 } else { 0.4 };
        self.text("\u{E706}", &self.fonts.glyph, g.glyph, c.text, 0.85 * a);

        let (left, right) = (g.x0 - THUMB_R * s, g.x1 + THUMB_R * s);
        let track = |r: f32| Rect::new(left, g.cy - TRACK_H * s / 2.0, r, g.cy + TRACK_H * s / 2.0);
        self.pill(track(right), c.text, 0.45 * a);
        let shown = value.filter(|_| enabled);
        let tx = g.x_of(shown.unwrap_or(0));
        if shown.is_some() {
            self.pill(track(tx), color, 1.0);
        }
        // Thumb: outer disc with a hairline edge, inner dot in the monitor's color.
        self.circle(tx, g.cy, THUMB_R * s, Rgb::BLACK, 0.25 * a);
        self.circle(tx, g.cy, (THUMB_R - 0.5) * s, c.thumb, a);
        let dot = match (pressed, hot) {
            (true, _) => 5.0,
            (_, true) => 7.0,
            _ => 6.0,
        };
        self.circle(tx, g.cy, dot * s, color, a);
        if focused {
            self.ring(tx, g.cy, (THUMB_R + 3.0) * s, 2.0 * s, c.text, 1.0);
        }
        let label = shown.map_or("–".to_string(), |v| v.to_string());
        self.text(&label, &self.fonts.value, g.value, c.text, a);

        if let Some((lo, hi)) = range {
            let (hl, hh) = (g.handle_rect(lo), g.handle_rect(hi));
            let bar = Rect::new(g.x_of(lo), hl.cy() - s, g.x_of(hi), hl.cy() + s);
            self.pill(bar, c.text, 0.35 * a);
            for h in [hl, hh] {
                let r = Rect::centered((h.l + h.r) / 2.0, h.cy(), 6.0 * s, 10.0 * s);
                self.pill(r, c.text, 0.8 * a);
            }
        }
    }
}

#[cfg(feature = "plus")]
impl Canvas<'_> {
    /// A small warm sun: a disc with eight rays, centered in `r`.
    fn sun(&self, r: Rect) {
        let s = self.s;
        let amber = Rgb::new(0xFF, 0xB1, 0x4A);
        let (cx, cy) = ((r.l + r.r) / 2.0, r.cy());
        self.circle(cx, cy, 4.0 * s, amber, 1.0);
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let (sin, cos) = a.sin_cos();
            // SAFETY: drawing on our own render target between BeginDraw/EndDraw.
            unsafe {
                self.rt.DrawLine(
                    Vector2::new(cx + 6.5 * s * cos, cy + 6.5 * s * sin),
                    Vector2::new(cx + 9.0 * s * cos, cy + 9.0 * s * sin),
                    self.pen(amber, 1.0),
                    1.5 * s,
                    None,
                );
            }
        }
    }

    /// Sun, amber-to-white gradient track, thumb in the shown color, value, focus ring.
    fn color_slider(&self, g: &SliderGeom, step: u8, min_k: u32, look: (bool, bool, bool)) {
        let (hot, pressed, focused) = look;
        let (c, s) = (self.colors, self.s);
        self.sun(g.glyph);

        let (left, right) = (g.x0 - THUMB_R * s, g.x1 + THUMB_R * s);
        let track = Rect::new(
            left,
            g.cy - TRACK_H * s / 2.0,
            right,
            g.cy + TRACK_H * s / 2.0,
        );
        let stops: [D2D1_GRADIENT_STOP; 5] = std::array::from_fn(|i| {
            let [r, gr, b] = tint(min_k + (MAX_K - min_k) * i as u32 / 4);
            D2D1_GRADIENT_STOP {
                position: i as f32 / 4.0,
                color: color_f(Rgb::new(r, gr, b), 1.0),
            }
        });
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: Vector2::new(left, 0.0),
            endPoint: Vector2::new(right, 0.0),
        };
        // SAFETY: valid render target; the gradient objects live until the end of the block.
        let gradient = unsafe {
            self.rt
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .and_then(|coll| self.rt.CreateLinearGradientBrush(&props, None, &coll))
        };
        let radius = TRACK_H * s / 2.0;
        if let Ok(brush) = gradient {
            // SAFETY: as above.
            unsafe {
                self.rt
                    .FillRoundedRectangle(&rounded(track, radius), &brush)
            };
        }
        // A hairline keeps the white end visible on a light background.
        // SAFETY: as above.
        unsafe {
            self.rt
                .DrawRoundedRectangle(&rounded(track, radius), self.pen(c.text, 0.25), s, None)
        };

        let tx = g.x_of(step);
        let kelvin = step_kelvin(step, min_k);
        let [r, gr, b] = tint(kelvin);
        self.circle(tx, g.cy, THUMB_R * s, Rgb::BLACK, 0.25);
        self.circle(tx, g.cy, (THUMB_R - 0.5) * s, c.thumb, 1.0);
        let dot = match (pressed, hot) {
            (true, _) => 5.0,
            (_, true) => 7.0,
            _ => 6.0,
        };
        self.circle(tx, g.cy, dot * s, Rgb::new(r, gr, b), 1.0);
        self.ring(tx, g.cy, dot * s, 0.5 * s, c.text, 0.4);
        if focused {
            self.ring(tx, g.cy, (THUMB_R + 3.0) * s, 2.0 * s, c.text, 1.0);
        }
        let label = format!("{kelvin} K");
        self.text(&label, &self.fonts.hint, g.value, c.text, 1.0);
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
            header: make(t, 14.0, semi, lead)?,
            caption: make(t, 12.0, reg, lead)?,
            hint: make(t, 12.0, reg, trail)?,
            value: make(t, 14.0, reg, trail)?,
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
                alphaMode: if self.acrylic {
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
        let layout = self.layout();
        let colors = colors(self.theme.taskbar_light, self.acrylic);
        let (Some((_, fonts)), Some(target)) = (&self.fonts, &self.target) else {
            return;
        };
        let cv = Canvas {
            rt: &target.rt,
            brush: &target.brush,
            fonts,
            colors: &colors,
            s: layout.scale,
        };
        let s = layout.scale;
        let accent = self.theme.accent;
        let full = Rect::new(0.0, 0.0, layout.width, layout.height);

        // SAFETY: BeginDraw/EndDraw bracket all drawing on this render target.
        unsafe {
            cv.rt.BeginDraw();
            cv.rt.Clear(Some(&color_f(Rgb::BLACK, 0.0)));
            let (bg, alpha) = colors.bg;
            cv.rt.FillRectangle(&full.d2d(), cv.pen(bg, alpha));
            cv.rt
                .FillRectangle(&layout.footer.d2d(), cv.pen(Rgb::BLACK, colors.footer));
        }

        cv.text(
            tr(self.lang, Key::Brightness),
            &fonts.header,
            layout.header,
            colors.text,
            1.0,
        );
        let hot = |id: Slider| self.hover == Hit::Slider(id);
        let pressed = |id: Slider| self.drag == Some(Drag::Slider(id));
        let focused = |id: Slider| self.focus_visible && self.focus == Some(id);

        if let Some(m) = &layout.master {
            let id = Slider::Master;
            cv.caption(m, tr(self.lang, Key::LinkAll), accent, 1.0);
            let look = (hot(id), pressed(id), focused(id));
            cv.slider(&m.slider, Some(self.master), accent, true, look, None);
        }
        for (i, (b, row)) in layout.monitors.iter().zip(&self.rows).enumerate() {
            let id = Slider::Row(i);
            let a = if row.enabled { 1.0 } else { 0.5 };
            cv.caption(b, &row.name, row.color, a);
            for (swatch, color) in b.swatches.iter().zip(PALETTE) {
                let cx = (swatch.l + swatch.r) / 2.0;
                cv.circle(cx, swatch.cy(), SWATCH_R * s, color, 1.0);
                if color == row.color {
                    cv.ring(
                        cx,
                        swatch.cy(),
                        (SWATCH_R + 3.0) * s,
                        1.5 * s,
                        colors.text,
                        1.0,
                    );
                }
            }
            let look = (hot(id), pressed(id), focused(id));
            let range = self.linked.then_some((row.link_min, row.link_max));
            cv.slider(&b.slider, row.value, row.color, row.enabled, look, range);
        }

        #[cfg(feature = "plus")]
        if let (Some((caption, g)), Some(step)) = (&layout.color, self.color) {
            let id = Slider::Color;
            cv.text(
                tr(self.lang, Key::Plus(PlusKey::FlyoutColor)),
                &fonts.caption,
                *caption,
                colors.text,
                0.75,
            );
            let look = (hot(id), pressed(id), focused(id));
            cv.color_slider(g, step, self.color_min, look);
        }

        // Footer: "Link all" switch + label, then the hovered button's label and the buttons.
        let sw = layout.switch;
        let (knob_r, knob_x) = if self.linked {
            cv.pill(sw, accent, 1.0);
            (7.0 * s, sw.r - 10.0 * s)
        } else {
            cv.pill(sw, colors.text, 0.55);
            let inner = Rect::new(sw.l + s, sw.t + s, sw.r - s, sw.b - s);
            cv.pill(inner, colors.bg.0, 1.0);
            (5.0 * s, sw.l + 10.0 * s)
        };
        let knob = if self.linked { Rgb::WHITE } else { colors.text };
        cv.circle(
            knob_x,
            sw.cy(),
            knob_r,
            knob,
            if self.linked { 1.0 } else { 0.8 },
        );
        let label = Rect::new(sw.r + 8.0 * s, layout.link.t, layout.link.r, layout.link.b);
        cv.text(
            tr(self.lang, Key::LinkAll),
            &fonts.caption,
            label,
            colors.text,
            1.0,
        );
        for (r, hit, glyph) in [
            (layout.rescan, Hit::Rescan, "\u{E72C}"),
            (layout.settings, Hit::Settings, "\u{E713}"),
        ] {
            if self.hover == hit {
                // SAFETY: as above.
                unsafe {
                    cv.rt
                        .FillRoundedRectangle(&rounded(r, 4.0 * s), cv.pen(colors.text, 0.08));
                }
                let key = if hit == Hit::Rescan {
                    Key::MenuRescan
                } else {
                    Key::MenuSettings
                };
                cv.text(
                    tr(self.lang, key),
                    &fonts.hint,
                    layout.hint,
                    colors.text,
                    0.75,
                );
            }
            cv.text(glyph, &fonts.glyph, r, colors.text, 0.9);
        }
        if !self.theme.is_win11 {
            // Windows 10: DWM draws no border for us.
            let edge = Rect::new(0.5, 0.5, layout.width - 0.5, layout.height - 0.5);
            // SAFETY: as above.
            unsafe {
                cv.rt
                    .DrawRectangle(&edge.d2d(), cv.pen(colors.text, 0.2), 1.0, None)
            };
        }

        // SAFETY: closes the BeginDraw above.
        let end = unsafe { cv.rt.EndDraw(None, None) };
        if matches!(end, Err(e) if e.code() == D2DERR_RECREATE_TARGET) {
            self.target = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// Set an integer DWM window attribute; whether DWM accepted it.
fn set_dwm_int(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: i32) -> bool {
    // SAFETY: `value` outlives the call and its size is passed.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            attr,
            &value as *const i32 as *const _,
            size_of::<i32>() as u32,
        )
        .is_ok()
    }
}

impl State {
    fn create() -> Option<State> {
        // SAFETY: plain Win32/COM creation; `class` outlives both calls that read it.
        unsafe {
            let d2d: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
            let hinst = HINSTANCE(GetModuleHandleW(None).ok()?.0);
            let class = class_name();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
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
            .ok()?;

            // Windows 11: rounded corners and a transient acrylic backdrop that shows through
            // the extended frame. If DWM rejects any of it, paint an opaque background instead.
            let theme = Theme::read();
            set_dwm_int(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                i32::from(!theme.taskbar_light),
            );
            let acrylic = theme.is_win11
                && set_dwm_int(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND.0)
                && set_dwm_int(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW.0)
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
            Some(State {
                hwnd,
                d2d,
                dwrite,
                text_font,
                icon_font,
                acrylic,
                fonts: None,
                target: None,
                rows: Vec::new(),
                linked: false,
                lang: Lang::default(),
                theme,
                dpi: 96,
                place: Placement::default(),
                applied: (0, 0, 0, 0),
                master: 0,
                master_touched: false,
                #[cfg(feature = "plus")]
                color: None,
                #[cfg(feature = "plus")]
                color_min: MAX_K,
                #[cfg(feature = "plus")]
                color_moved: false,
                picker: None,
                focus: None,
                focus_visible: false,
                hover: Hit::None,
                tracking_leave: false,
                drag: None,
                wheel_rem: 0,
            })
        }
    }
}

/// The master slider's position, for re-applying it after a calibration change.
pub fn master() -> u8 {
    with_state(|st| st.master).unwrap_or(0)
}

/// Close (idempotent): destroy the window and release all Direct2D/DirectWrite objects,
/// and remember when, so the tray click that caused it doesn't reopen.
fn close() {
    let state = STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.take()));
    if let Some(st) = state {
        CLOSED_AT.set(Some(Instant::now()));
        // The state is already out of `STATE`, so the messages this sends find nothing.
        drop(st);
        util::trim_working_set();
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // Release the render target before its window goes away.
        self.target = None;
        self.fonts = None;
        // SAFETY: our own window, destroyed once on the thread that created it.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Recompute the window rectangle (the height changes with the picker and the master row)
/// and return it; `None` before `init` or while the state is borrowed.
fn sync_geometry() -> Option<(i32, i32, i32, i32)> {
    with_state(|st| {
        let (w, h) = st.window_size();
        let (x, y) = st.origin(w, h);
        st.applied = (x, y, w, h);
        st.applied
    })
}

/// Apply a changed size to the window and repaint.
fn refresh_window() {
    let Some((hwnd, old)) = with_state(|st| (st.hwnd, st.applied)) else {
        return;
    };
    if let Some(new @ (x, y, w, h)) = sync_geometry() {
        if new != old {
            // SAFETY: our own window; no state borrow is held.
            unsafe {
                let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }
    // SAFETY: our own window.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn point(lparam: LPARAM) -> (i32, i32) {
    (
        i32::from(lparam.0 as u16 as i16),
        i32::from((lparam.0 >> 16) as u16 as i16),
    )
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
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                // A drag cut short still delivers its final value.
                let action = with_state(State::release).flatten();
                close();
                emit(action);
            }
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            if with_state(|st| std::mem::replace(&mut st.dpi, dpi) != dpi) == Some(true) {
                refresh_window();
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            let action = with_state(|st| {
                st.hover = st.hit(x, y);
                st.drag_to(x)
            })
            .flatten();
            track_leave(hwnd);
            emit(action);
            refresh_window();
        }
        WM_MOUSELEAVE => {
            with_state(|st| {
                st.hover = Hit::None;
                st.tracking_leave = false;
            });
            refresh_window();
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            let (action, dragging) =
                with_state(|st| (st.mouse_down(x, y), st.drag.is_some())).unwrap_or_default();
            if dragging {
                // SAFETY: our own window.
                unsafe { SetCapture(hwnd) };
            }
            emit(action);
            refresh_window();
        }
        WM_LBUTTONUP => {
            let action = with_state(State::release).flatten();
            // SAFETY: releases a capture we may hold.
            unsafe {
                let _ = ReleaseCapture();
            }
            emit(action);
            refresh_window();
        }
        WM_CAPTURECHANGED => {
            let action = with_state(State::release).flatten();
            emit(action);
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam.0 >> 16) as u16 as i16);
            let fine = wparam.0 & 0x4 != 0; // MK_SHIFT
            let (sx, sy) = point(lparam);
            let mut pt = POINT { x: sx, y: sy };
            // SAFETY: valid window and point.
            unsafe {
                let _ = ScreenToClient(hwnd, &mut pt);
            }
            let action = with_state(|st| st.wheel(pt.x, pt.y, delta, fine)).flatten();
            emit(action);
            refresh_window();
        }
        WM_KEYDOWN => {
            // SAFETY: plain key state query.
            let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
            let (action, close_it) =
                with_state(|st| st.key(wparam.0 as u16, shift)).unwrap_or_default();
            if close_it {
                close();
            }
            emit(action);
            refresh_window();
        }
        // SAFETY: default processing with the arguments we were given.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
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
