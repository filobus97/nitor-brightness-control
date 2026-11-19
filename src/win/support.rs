//! The support reminder: a small popup near the tray, styled like the flyout, that says
//! once in a long while that supporting the project is an option (rules in `crate::support`).
//!
//! It never takes focus (`WS_EX_NOACTIVATE`), exists only while shown (the window and every
//! Direct2D/DirectWrite object are created on demand and destroyed on close) and closes by
//! itself after a minute without interaction. Like the flyout, `show` runs while the app is
//! borrowed, so the outcome is only reported from the window procedure, after the state
//! borrow is released.

use super::flyout::{Rect, Target, colors, create_window, make_target, rounded, set_dwm_int};
use super::icon::{color_f, has_family};
use super::theme::Theme;
use super::util;
use crate::color::Rgb;
use crate::i18n::{Key, Lang, tr};
use crate::support::KOFI_URL;
use std::cell::RefCell;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    ValidateRect,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyWindow, GetClientRect, GetSystemMetrics, HWND_TOPMOST, IDC_ARROW,
    KillTimer, LoadCursorW, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetTimer, SetWindowPos, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};
use windows::core::{HSTRING, PCWSTR, w};
use windows_numerics::Vector2;

/// How the popup was closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// "Support on Ko-fi" (the page was opened).
    Kofi,
    NotNow,
    /// A minute without interaction.
    Timeout,
}

/// Reason and whether "Don't remind me again" was ticked; delivered through [`init`].
type OnClose = fn(Reason, bool);

static ON_CLOSE: OnceLock<OnClose> = OnceLock::new();

const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
const MARGIN: f32 = 12.0;
const BOX: f32 = 16.0;
/// Interaction-free time after which the popup closes itself.
const AUTO_CLOSE_MS: u32 = 60_000;
const TIMER_CLOSE: usize = 1;
/// `WM_MOUSELEAVE` (not exposed by the bindings).
const WM_MOUSELEAVE: u32 = 0x02A3;
/// `MA_NOACTIVATE`: a click does not activate the window.
const MA_NOACTIVATE: isize = 3;
/// `D2DERR_RECREATE_TARGET`: the render target must be recreated.
const D2DERR_RECREATE_TARGET: windows::core::HRESULT = windows::core::HRESULT(0x8899000Cu32 as i32);

/// Register the window class. Once, on the UI thread.
pub fn init(on_close: OnClose) {
    let _ = ON_CLOSE.set(on_close);
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
    util::wide("NitorSupport")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    None,
    Check,
    Kofi,
    NotNow,
}

#[derive(Default)]
struct Layout {
    width: f32,
    height: f32,
    scale: f32,
    title: Rect,
    body: Rect,
    check: Rect,
    check_box: Rect,
    check_label: Rect,
    kofi: Rect,
    not_now: Rect,
}

impl Layout {
    fn new(s: f32) -> Layout {
        let (w, pad) = (WIDTH * s, PAD * s);
        let check = Rect::new(pad, 102.0 * s, w - pad, 128.0 * s);
        let kofi = Rect::new(w - pad - 140.0 * s, 140.0 * s, w - pad, 172.0 * s);
        Layout {
            width: w,
            height: 188.0 * s,
            scale: s,
            title: Rect::new(pad, 14.0 * s, w - pad, 38.0 * s),
            body: Rect::new(pad, 40.0 * s, w - pad, 96.0 * s),
            check,
            check_box: Rect::new(
                check.l,
                check.cy() - BOX * s / 2.0,
                check.l + BOX * s,
                check.cy() + BOX * s / 2.0,
            ),
            check_label: Rect::new(check.l + (BOX + 8.0) * s, check.t, check.r, check.b),
            kofi,
            not_now: Rect::new(
                kofi.l - 8.0 * s - 96.0 * s,
                kofi.t,
                kofi.l - 8.0 * s,
                kofi.b,
            ),
        }
    }

    fn hit(&self, x: f32, y: f32) -> Hit {
        if self.kofi.contains(x, y) {
            Hit::Kofi
        } else if self.not_now.contains(x, y) {
            Hit::NotNow
        } else if self.check.contains(x, y) {
            Hit::Check
        } else {
            Hit::None
        }
    }
}

struct Fonts {
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    label: IDWriteTextFormat,
    button: IDWriteTextFormat,
}

struct State {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    text_font: &'static str,
    /// Win11 backdrop + corners were accepted by DWM.
    acrylic: bool,
    theme: Theme,
    lang: Lang,
    dpi: u32,
    /// The work area of the monitor it sits on, and the corner it is pinned to.
    work: RECT,
    right: bool,
    bottom: bool,
    fonts: Option<(u32, Fonts)>,
    target: Option<Target>,
    hover: Hit,
    tracking_leave: bool,
    /// "Don't remind me again".
    dont_remind: bool,
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

/// The popup exists exactly while it is shown.
pub fn is_open() -> bool {
    STATE.with(|s| !matches!(s.try_borrow(), Ok(s) if s.is_none()))
}

/// Show the popup near `anchor` (screen coordinates of a tray icon; `None`: the bottom right
/// of the primary screen). Without focus. Whether it was created.
pub fn show(anchor: Option<POINT>, lang: Lang) -> bool {
    if is_open() {
        return false;
    }
    let Some(mut st) = State::create() else {
        return false;
    };
    st.lang = lang;
    // SAFETY: plain system queries with valid out-pointers.
    let anchor = anchor.unwrap_or_else(|| unsafe {
        POINT {
            x: GetSystemMetrics(SM_CXSCREEN) - 1,
            y: GetSystemMetrics(SM_CYSCREEN) - 1,
        }
    });
    // SAFETY: as above.
    unsafe {
        let mon = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        st.dpi = dx;
        st.work = info.rcWork;
    }
    st.right = anchor.x > (st.work.left + st.work.right) / 2;
    st.bottom = anchor.y > (st.work.top + st.work.bottom) / 2;
    let hwnd = st.hwnd;
    let (x, y, w, h) = st.geometry();
    STATE.with(|s| *s.borrow_mut() = Some(st));
    // SAFETY: plain window calls on our own window; the state borrow is released.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            w,
            h,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        SetTimer(Some(hwnd), TIMER_CLOSE, AUTO_CLOSE_MS, None);
    }
    true
}

/// Close without reporting (shutdown); whether it was open.
pub fn close() -> bool {
    STATE
        .with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.take()))
        .map(|st| {
            drop(st);
            util::trim_working_set();
        })
        .is_some()
}

/// The system theme changed: restyle (also while open).
pub fn retheme() {
    let open = with_state(|st| {
        st.theme = Theme::read();
        set_dwm_int(
            st.hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            i32::from(!st.theme.taskbar_light),
        );
        if st.acrylic {
            set_dwm_int(st.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW.0);
        }
        st.hwnd
    });
    if let Some(hwnd) = open {
        // SAFETY: our own window.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }
}

/// Close for `reason` and report it. The state is taken out first, so the callback may
/// call back into this module.
fn finish(reason: Reason) {
    let state = STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.take()));
    let Some(st) = state else { return };
    let dont_remind = st.dont_remind;
    drop(st);
    util::trim_working_set();
    if reason == Reason::Kofi {
        util::open_url(KOFI_URL);
    }
    if let Some(f) = ON_CLOSE.get() {
        f(reason, dont_remind);
    }
}

impl State {
    fn create() -> Option<State> {
        // SAFETY: plain COM factory creation.
        let (d2d, dwrite) = unsafe {
            let d2d: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
            (d2d, dwrite)
        };
        let (hwnd, theme, acrylic) = create_window(
            &class_name(),
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        )?;
        let text_font = if has_family(&dwrite, "Segoe UI Variable Text") {
            "Segoe UI Variable Text"
        } else {
            "Segoe UI"
        };
        Some(State {
            hwnd,
            d2d,
            dwrite,
            text_font,
            acrylic,
            theme,
            lang: Lang::default(),
            dpi: 96,
            work: RECT::default(),
            right: true,
            bottom: true,
            fonts: None,
            target: None,
            hover: Hit::None,
            tracking_leave: false,
            dont_remind: false,
        })
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// Window rectangle: pinned to its corner of the work area, inset by the margin.
    fn geometry(&self) -> (i32, i32, i32, i32) {
        let s = self.scale();
        let l = Layout::new(s);
        let (w, h) = (l.width as i32, l.height as i32);
        let m = (MARGIN * s).round() as i32;
        let x = if self.right {
            self.work.right - w - m
        } else {
            self.work.left + m
        };
        let y = if self.bottom {
            self.work.bottom - h - m
        } else {
            self.work.top + m
        };
        (
            x.clamp(self.work.left, (self.work.right - w).max(self.work.left)),
            y.clamp(self.work.top, (self.work.bottom - h).max(self.work.top)),
            w,
            h,
        )
    }

    fn ensure_fonts(&mut self) -> Option<()> {
        if self.fonts.as_ref().is_some_and(|(dpi, _)| *dpi == self.dpi) {
            return Some(());
        }
        let s = self.scale();
        let make = |size: f32, weight, align, para, wrap| -> Option<IDWriteTextFormat> {
            // SAFETY: factory call with valid arguments; the text format is ours.
            unsafe {
                let f = self
                    .dwrite
                    .CreateTextFormat(
                        &HSTRING::from(self.text_font),
                        None,
                        weight,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size * s,
                        w!("en-us"),
                    )
                    .ok()?;
                f.SetTextAlignment(align).ok()?;
                f.SetParagraphAlignment(para).ok()?;
                f.SetWordWrapping(wrap).ok()?;
                Some(f)
            }
        };
        let (reg, semi) = (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD);
        let (lead, mid) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_CENTER);
        let (near, center) = (
            DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
        );
        let (wrap, no_wrap) = (DWRITE_WORD_WRAPPING_WRAP, DWRITE_WORD_WRAPPING_NO_WRAP);
        let fonts = Fonts {
            title: make(14.0, semi, lead, center, no_wrap)?,
            body: make(12.0, reg, lead, near, wrap)?,
            label: make(12.0, reg, lead, center, no_wrap)?,
            button: make(12.0, reg, mid, center, no_wrap)?,
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
        self.target = Some(make_target(&self.d2d, self.hwnd, self.acrylic, size)?);
        Some(())
    }

    fn paint(&mut self) {
        if self.ensure_fonts().is_none() || self.ensure_target().is_none() {
            return;
        }
        let l = Layout::new(self.scale());
        let s = l.scale;
        let colors = colors(self.theme.taskbar_light, self.acrylic);
        let accent = self.theme.accent;
        let (Some((_, fonts)), Some(target)) = (&self.fonts, &self.target) else {
            return;
        };
        let (rt, brush) = (&target.rt, &target.brush);
        let pen = |color: Rgb, alpha: f32| {
            // SAFETY: the brush belongs to this render target.
            unsafe { brush.SetColor(&color_f(color, alpha)) };
            brush
        };
        let text = |t: &str, font: &IDWriteTextFormat, r: Rect, color: Rgb, alpha: f32| {
            let utf16: Vec<u16> = t.encode_utf16().collect();
            // SAFETY: valid format/brush from the same factory/target; `utf16` outlives the call.
            unsafe {
                rt.DrawText(
                    &utf16,
                    font,
                    &r.d2d(),
                    pen(color, alpha),
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        };
        let full = Rect::new(0.0, 0.0, l.width, l.height);

        // SAFETY: BeginDraw/EndDraw bracket all drawing on this render target.
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&color_f(Rgb::BLACK, 0.0)));
            let (bg, alpha) = colors.bg;
            rt.FillRectangle(&full.d2d(), pen(bg, alpha));
        }
        text(
            tr(self.lang, Key::SupportTitle),
            &fonts.title,
            l.title,
            colors.text,
            1.0,
        );
        text(
            tr(self.lang, Key::SupportText),
            &fonts.body,
            l.body,
            colors.text,
            0.85,
        );

        // Checkbox.
        let b = l.check_box;
        // SAFETY: as above.
        unsafe {
            if self.dont_remind {
                rt.FillRoundedRectangle(&rounded(b, 3.0 * s), pen(accent, 1.0));
                let (x, y) = (b.l, b.t);
                for (p0, p1) in [((3.5, 8.5), (6.5, 11.5)), ((6.5, 11.5), (12.5, 4.5))] {
                    rt.DrawLine(
                        Vector2::new(x + p0.0 * s, y + p0.1 * s),
                        Vector2::new(x + p1.0 * s, y + p1.1 * s),
                        pen(Rgb::WHITE, 1.0),
                        1.6 * s,
                        None,
                    );
                }
            } else {
                let edge = Rect::new(b.l + 0.5, b.t + 0.5, b.r - 0.5, b.b - 0.5);
                rt.DrawRoundedRectangle(&rounded(edge, 3.0 * s), pen(colors.text, 0.55), 1.0, None);
            }
        }
        text(
            tr(self.lang, Key::LblDontRemind),
            &fonts.label,
            l.check_label,
            colors.text,
            0.9,
        );

        // Buttons.
        let hot = |h: Hit| self.hover == h;
        // SAFETY: as above.
        unsafe {
            rt.FillRoundedRectangle(
                &rounded(l.kofi, 4.0 * s),
                pen(accent, if hot(Hit::Kofi) { 0.85 } else { 1.0 }),
            );
            rt.FillRoundedRectangle(
                &rounded(l.not_now, 4.0 * s),
                pen(colors.text, if hot(Hit::NotNow) { 0.16 } else { 0.08 }),
            );
        }
        text(
            tr(self.lang, Key::BtnSupport),
            &fonts.button,
            l.kofi,
            Rgb::WHITE,
            1.0,
        );
        text(
            tr(self.lang, Key::BtnNotNow),
            &fonts.button,
            l.not_now,
            colors.text,
            1.0,
        );
        if !self.theme.is_win11 {
            // Windows 10: DWM draws no border for us.
            let edge = Rect::new(0.5, 0.5, l.width - 0.5, l.height - 0.5);
            // SAFETY: as above.
            unsafe { rt.DrawRectangle(&edge.d2d(), pen(colors.text, 0.2), 1.0, None) };
        }

        // SAFETY: closes the BeginDraw above.
        let end = unsafe { rt.EndDraw(None, None) };
        if matches!(end, Err(e) if e.code() == D2DERR_RECREATE_TARGET) {
            self.target = None;
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // Release the render target before its window goes away.
        self.target = None;
        self.fonts = None;
        // SAFETY: our own window, destroyed once on the thread that created it; this also
        // discards its auto-close timer.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_CLOSE);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn point(lparam: LPARAM) -> (f32, f32) {
    (
        f32::from(lparam.0 as u16 as i16),
        f32::from((lparam.0 >> 16) as u16 as i16),
    )
}

/// Interaction restarts the auto-close countdown.
fn restart_timer(hwnd: HWND) {
    // SAFETY: our own window; setting a timer with the same id replaces it.
    unsafe {
        SetTimer(Some(hwnd), TIMER_CLOSE, AUTO_CLOSE_MS, None);
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
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE),
        WM_TIMER if wparam.0 == TIMER_CLOSE => finish(Reason::Timeout),
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let geo = with_state(|st| {
                (std::mem::replace(&mut st.dpi, dpi) != dpi).then(|| st.geometry())
            })
            .flatten();
            if let Some((x, y, w, h)) = geo {
                // SAFETY: our own window; no state borrow is held.
                unsafe {
                    let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            let changed = with_state(|st| {
                let h = Layout::new(st.scale()).hit(x, y);
                let changed = std::mem::replace(&mut st.hover, h) != h;
                if !std::mem::replace(&mut st.tracking_leave, true) {
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
                changed
            });
            if changed == Some(true) {
                restart_timer(hwnd);
                // SAFETY: our own window.
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
        }
        WM_MOUSELEAVE => {
            with_state(|st| {
                st.hover = Hit::None;
                st.tracking_leave = false;
            });
            // SAFETY: our own window.
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            let hit = with_state(|st| Layout::new(st.scale()).hit(x, y)).unwrap_or(Hit::None);
            match hit {
                Hit::Kofi => finish(Reason::Kofi),
                Hit::NotNow => finish(Reason::NotNow),
                Hit::Check => {
                    with_state(|st| st.dont_remind = !st.dont_remind);
                    restart_timer(hwnd);
                    // SAFETY: our own window.
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                Hit::None => {}
            }
        }
        // SAFETY: default processing with the arguments we were given.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}
