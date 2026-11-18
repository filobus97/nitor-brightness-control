//! Nitor Plus: the offline map popup. An equirectangular world map from the embedded
//! Natural Earth data (`assets/world-110m.bin`): click to place a marker, mouse wheel to
//! zoom (1x-8x) around the cursor, drag to pan, then confirm. Themed like Settings and
//! built on its drawing helpers (a child module of `settings`). No network involved.

use super::*;
use crate::auto::map::{View, decode_rings};
use crate::color::mix;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

const WORLD: &[u8] = include_bytes!("../../../assets/world-110m.bin");
const MAP_CLASS: PCWSTR = w!("NitorMapPicker");
/// Client size in DIP: the map (2:1) above a bar with the coordinates and the buttons.
const MAP_W: f32 = 720.0;
const MAP_H: f32 = 360.0;
const BAR_H: f32 = 64.0;
/// Pointer travel (DIP) that turns a click into a drag.
const SLOP: f32 = 4.0;
/// Zoom factor of one wheel notch.
const NOTCH_ZOOM: f64 = 1.25;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Press {
    Use,
    Cancel,
}

struct DragState {
    from: (f32, f32),
    last: (f32, f32),
    moved: bool,
}

struct Map {
    hwnd: HWND,
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    text_font: &'static str,
    icon_font: &'static str,
    fonts: Option<(u32, Fonts)>,
    target: Option<Target>,
    /// All land rings as one even-odd path in `(lon, -lat)` degrees.
    land: Option<ID2D1PathGeometry>,
    pal: Pal,
    accent: Rgb,
    lang: Lang,
    dpi: u32,
    view: View,
    /// `(lon, lat)`, like the view.
    marker: Option<(f64, f64)>,
    drag: Option<DragState>,
    hover: Option<Press>,
    pick: fn(f64, f64),
}

thread_local! {
    static MAP: RefCell<Option<Map>> = const { RefCell::new(None) };
}

fn with_map<R>(f: impl FnOnce(&mut Map) -> R) -> Option<R> {
    MAP.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut s| s.as_mut().map(f))
    })
}

/// Open the popup over `owner` (or bring the open one to the front). `initial` is the
/// current location `(lat, lon)`, if any; `pick` receives the confirmed point.
pub(super) fn open(
    owner: HWND,
    lang: Lang,
    dpi: u32,
    initial: Option<(f64, f64)>,
    pick: fn(f64, f64),
) {
    if let Some(hwnd) = with_map(|m| m.hwnd) {
        // SAFETY: our own window.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    if let Some(hwnd) = create(owner, lang, dpi, initial, pick) {
        // SAFETY: our own window.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetFocus(Some(hwnd));
        }
        invalidate(hwnd);
    }
}

/// The land as a single path geometry, in `(lon, -lat)` so a plain scale maps it.
fn land_geometry(d2d: &ID2D1Factory) -> Option<ID2D1PathGeometry> {
    let rings = decode_rings(WORLD)?;
    // SAFETY: factory and sink calls with valid arguments; the sink is closed before use.
    unsafe {
        let geometry = d2d.CreatePathGeometry().ok()?;
        let sink = geometry.Open().ok()?;
        sink.SetFillMode(D2D1_FILL_MODE_ALTERNATE);
        for ring in &rings {
            let pts: Vec<Vector2> = ring.iter().map(|&(x, y)| Vector2::new(x, -y)).collect();
            sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&pts[1..]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        sink.Close().ok()?;
        Some(geometry)
    }
}

fn create(
    owner: HWND,
    lang: Lang,
    dpi: u32,
    initial: Option<(f64, f64)>,
    pick: fn(f64, f64),
) -> Option<HWND> {
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    // SAFETY: plain Win32/COM creation; every pointer passed outlives its call.
    unsafe {
        let hinst = HINSTANCE(GetModuleHandleW(None).ok()?.0);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(map_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: MAP_CLASS,
            ..Default::default()
        };
        // Fails harmlessly when the class exists from an earlier popup.
        RegisterClassExW(&wc);

        let s = dpi as f32 / 96.0;
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: (MAP_W * s).round() as i32,
            bottom: ((MAP_H + BAR_H) * s).round() as i32,
        };
        let _ = AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE(0), dpi);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        // Centered over the Settings window.
        let mut at = RECT::default();
        let _ = GetWindowRect(owner, &mut at);
        let x = at.left + (at.right - at.left - w) / 2;
        let y = at.top + (at.bottom - at.top - h) / 2;

        let title = util::wide(tr(lang, Key::Plus(PlusKey::MapTitle)));
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            MAP_CLASS,
            PCWSTR(title.as_ptr()),
            style,
            x,
            y,
            w,
            h,
            Some(owner),
            None,
            Some(hinst),
            None,
        )
        .ok()?;

        let theme = Theme::read();
        set_dark(hwnd, !theme.apps_light);
        let state = (|| {
            let d2d: ID2D1Factory =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
            let text_font = if has_family(&dwrite, "Segoe UI Variable Text") {
                "Segoe UI Variable Text"
            } else {
                "Segoe UI"
            };
            Some(Map {
                hwnd,
                land: land_geometry(&d2d),
                d2d,
                dwrite,
                text_font,
                icon_font: "Segoe UI",
                fonts: None,
                target: None,
                pal: pal(theme.apps_light, false),
                accent: theme.accent,
                lang,
                dpi,
                view: View::default(),
                marker: initial.map(|(lat, lon)| (lon, lat)),
                drag: None,
                hover: None,
                pick,
            })
        })();
        let Some(state) = state else {
            let _ = DestroyWindow(hwnd);
            return None;
        };
        MAP.with(|cell| *cell.borrow_mut() = Some(state));
        Some(hwnd)
    }
}

impl Map {
    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// The map viewport in device pixels.
    fn size(&self) -> (f64, f64) {
        let s = f64::from(self.scale());
        (f64::from(MAP_W) * s, f64::from(MAP_H) * s)
    }

    fn label(&self, p: Press) -> &'static str {
        tr(
            self.lang,
            Key::Plus(match p {
                Press::Use => PlusKey::MapUse,
                Press::Cancel => PlusKey::MapCancel,
            }),
        )
    }

    /// The bar's buttons, right to left from the corner.
    fn buttons(&self) -> [(Press, Rect); 2] {
        let s = self.scale();
        let h = BTN * s;
        let top = (MAP_H + BAR_H / 2.0) * s - h / 2.0;
        let mut right = (MAP_W - PAD) * s;
        [Press::Use, Press::Cancel].map(|p| {
            let w = (self.label(p).chars().count() as f32 * 7.5 + 28.0) * s;
            let r = Rect::new(right - w, top, right, top + h);
            right = r.l - 8.0 * s;
            (p, r)
        })
    }

    fn button_at(&self, x: f32, y: f32) -> Option<Press> {
        self.buttons()
            .into_iter()
            .find(|(p, r)| r.contains(x, y) && (*p == Press::Cancel || self.marker.is_some()))
            .map(|(p, _)| p)
    }

    fn in_map(&self, x: f32, y: f32) -> bool {
        let s = self.scale();
        x >= 0.0 && y >= 0.0 && x < MAP_W * s && y < MAP_H * s
    }

    // --- input ------------------------------------------------------------

    fn mouse_down(&mut self, x: f32, y: f32) -> Option<Press> {
        if self.in_map(x, y) {
            self.drag = Some(DragState {
                from: (x, y),
                last: (x, y),
                moved: false,
            });
            return None;
        }
        self.button_at(x, y)
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        self.hover = self.button_at(x, y);
        let (w, h) = self.size();
        let slop = SLOP * self.scale();
        let Some(d) = &mut self.drag else { return };
        d.moved |= (x - d.from.0).abs() > slop || (y - d.from.1).abs() > slop;
        if d.moved {
            let (dx, dy) = (f64::from(x - d.last.0), f64::from(y - d.last.1));
            self.view = self.view.pan(dx, dy, w, h);
        }
        d.last = (x, y);
    }

    /// A click (no drag) on the map moves the marker.
    fn mouse_up(&mut self, x: f32, y: f32) {
        let (w, h) = self.size();
        if let Some(d) = self.drag.take() {
            if !d.moved {
                self.marker = Some(self.view.to_geo(f64::from(x), f64::from(y), w, h));
            }
        }
    }

    fn wheel(&mut self, x: f32, y: f32, delta: i32) {
        let (w, h) = self.size();
        let factor = NOTCH_ZOOM.powf(f64::from(delta) / 120.0);
        self.view = self.view.zoom_at(f64::from(x), f64::from(y), w, h, factor);
    }

    // --- painting ---------------------------------------------------------

    fn paint(&mut self) {
        let fonts_ok = self.fonts.as_ref().is_some_and(|(dpi, _)| *dpi == self.dpi);
        if !fonts_ok {
            let Some(f) = make_fonts(&self.dwrite, self.text_font, self.icon_font, self.dpi) else {
                return;
            };
            self.fonts = Some((self.dpi, f));
        }
        if sync_target(&mut self.target, &self.d2d, self.hwnd, false).is_none() {
            return;
        }
        let (Some((_, fonts)), Some(target)) = (&self.fonts, &self.target) else {
            return;
        };
        let s = self.scale();
        let cv = Canvas {
            rt: &target.rt,
            brush: &target.brush,
            fonts,
            pal: &self.pal,
            s,
        };
        let (w, h) = self.size();
        let map = Rect::new(0.0, 0.0, w as f32, h as f32);
        let ocean = mix(self.pal.page, self.accent, 0.1);
        let land = mix(ocean, self.pal.text, 0.2);
        let px = s.max(1.0);

        // SAFETY: BeginDraw/EndDraw bracket all drawing on this render target; the
        // geometry belongs to the same factory.
        unsafe {
            cv.rt.BeginDraw();
            cv.rt.Clear(Some(&color_f(self.pal.page, 1.0)));
            cv.rt.FillRectangle(&map.d2d(), cv.pen(ocean, 1.0));
            cv.rt
                .PushAxisAlignedClip(&map.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
            if let Some(geometry) = &self.land {
                let sc = self.view.scale(w, h) as f32;
                cv.rt.SetTransform(&Matrix3x2 {
                    M11: sc,
                    M12: 0.0,
                    M21: 0.0,
                    M22: sc,
                    M31: (w / 2.0 - self.view.lon * f64::from(sc)) as f32,
                    M32: (h / 2.0 + self.view.lat * f64::from(sc)) as f32,
                });
                cv.rt.FillGeometry(geometry, cv.pen(land, 1.0), None);
                cv.rt.SetTransform(&Matrix3x2::identity());
            }
            // Graticule every 30 degrees.
            let line = |a: (f64, f64), b: (f64, f64)| {
                cv.rt.DrawLine(
                    Vector2::new(a.0 as f32, a.1 as f32),
                    Vector2::new(b.0 as f32, b.1 as f32),
                    cv.pen(self.pal.text, 0.14),
                    px,
                    None,
                );
            };
            for lon in (-180..=180).step_by(30) {
                let (x, _) = self.view.to_screen(f64::from(lon), 0.0, w, h);
                line((x, 0.0), (x, h));
            }
            for lat in (-90..=90).step_by(30) {
                let (_, y) = self.view.to_screen(0.0, f64::from(lat), w, h);
                line((0.0, y), (w, y));
            }
            if let Some((lon, lat)) = self.marker {
                let (x, y) = self.view.to_screen(lon, lat, w, h);
                self.draw_marker(&cv, x as f32, y as f32);
            }
            cv.rt.PopAxisAlignedClip();
        }
        self.draw_bar(&cv);

        // SAFETY: closes the BeginDraw above.
        let end = unsafe { cv.rt.EndDraw(None, None) };
        if matches!(end, Err(e) if e.code() == D2DERR_RECREATE_TARGET) {
            self.target = None;
        }
    }

    /// A crosshair ring with a halo so it reads on land, ocean and both themes.
    fn draw_marker(&self, cv: &Canvas, x: f32, y: f32) {
        let s = cv.s;
        let (r, arm) = (7.0 * s, 14.0 * s);
        for (color, width) in [(self.pal.page, 5.0 * s), (self.accent, 2.0 * s)] {
            let brush = cv.pen(color, 1.0);
            // SAFETY: drawing between BeginDraw/EndDraw on our own render target.
            unsafe {
                cv.rt.DrawEllipse(&ellipse(x, y, r), brush, width, None);
                for (a, b) in [
                    ((x - arm, y), (x - r, y)),
                    ((x + r, y), (x + arm, y)),
                    ((x, y - arm), (x, y - r)),
                    ((x, y + r), (x, y + arm)),
                ] {
                    cv.rt.DrawLine(
                        Vector2::new(a.0, a.1),
                        Vector2::new(b.0, b.1),
                        brush,
                        width,
                        None,
                    );
                }
            }
        }
    }

    fn draw_bar(&self, cv: &Canvas) {
        let (s, pal) = (cv.s, cv.pal);
        let top = MAP_H * s;
        let buttons = self.buttons();
        let text_w = buttons[1].1.l - 2.0 * PAD * s;
        let coords = self.marker.map_or("–".to_string(), |(lon, lat)| {
            format!("{lat:.2}, {lon:.2}")
        });
        let line = |y: f32| Rect::new(PAD * s, top + y * s, text_w, top + (y + 24.0) * s);
        cv.text(&coords, &cv.fonts.semi, line(8.0), pal.text, 1.0);
        let hint = tr(self.lang, Key::Plus(PlusKey::MapHint));
        cv.text(hint, &cv.fonts.caption, line(32.0), pal.text, 0.65);
        for (p, r) in buttons {
            let hot = self.hover == Some(p);
            match p {
                Press::Cancel => {
                    cv.fill(r, RADIUS * s, pal.text, if hot { 0.12 } else { 0.06 });
                    cv.frame(r, RADIUS * s, pal.text, 0.12);
                    cv.text(self.label(p), &cv.fonts.mid, r, pal.text, 1.0);
                }
                Press::Use => {
                    let on = self.marker.is_some();
                    let alpha = match (on, hot) {
                        (false, _) => 0.35,
                        (true, true) => 0.85,
                        _ => 1.0,
                    };
                    cv.fill(r, RADIUS * s, self.accent, alpha);
                    let text = text_on(self.accent);
                    cv.text(self.label(p), &cv.fonts.mid, r, text, alpha.max(0.6));
                }
            }
        }
    }
}

/// Confirm the marker (if any) and close; the state borrow is released before `pick` runs.
fn confirm(hwnd: HWND) {
    if let Some(Some((pick, (lon, lat)))) = with_map(|m| m.marker.map(|p| (m.pick, p))) {
        pick(lat, lon);
        close(hwnd);
    }
}

/// Client coordinates of a `WM_MOUSEWHEEL` (it carries screen coordinates).
fn wheel_point(hwnd: HWND, lparam: LPARAM) -> (f32, f32) {
    let mut p = POINT {
        x: i32::from(lparam.0 as u16 as i16),
        y: i32::from((lparam.0 >> 16) as u16 as i16),
    };
    // SAFETY: valid window and in/out point.
    unsafe {
        let _ = ScreenToClient(hwnd, &mut p);
    }
    (p.x as f32, p.y as f32)
}

unsafe extern "system" fn map_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            with_map(Map::paint);
            // SAFETY: our own window.
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            with_map(|m| m.dpi = dpi);
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
            invalidate(hwnd);
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point(lparam);
            let press = with_map(|m| m.mouse_down(x, y)).flatten();
            match press {
                Some(Press::Use) => confirm(hwnd),
                Some(Press::Cancel) => close(hwnd),
                None => {
                    // SAFETY: our own window; keeps the drag when the pointer leaves it.
                    unsafe { SetCapture(hwnd) };
                }
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = point(lparam);
            with_map(|m| m.mouse_move(x, y));
            invalidate(hwnd);
        }
        WM_LBUTTONUP => {
            let (x, y) = point(lparam);
            with_map(|m| m.mouse_up(x, y));
            // SAFETY: releases a capture we may hold.
            unsafe {
                let _ = ReleaseCapture();
            }
            invalidate(hwnd);
        }
        WM_CAPTURECHANGED => {
            with_map(|m| m.drag = None);
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam.0 >> 16) as u16 as i16);
            let (x, y) = wheel_point(hwnd, lparam);
            with_map(|m| m.wheel(x, y, delta));
            invalidate(hwnd);
        }
        WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => close(hwnd),
        WM_KEYDOWN if wparam.0 as u16 == VK_RETURN.0 => confirm(hwnd),
        WM_CLOSE => close(hwnd),
        WM_DESTROY => {
            MAP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut s| s.take()));
        }
        // SAFETY: default processing with the arguments we were given.
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

fn close(hwnd: HWND) {
    // SAFETY: our own window; no state borrow is held.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}
