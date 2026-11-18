//! Tray icon rendering with Direct2D + DirectWrite into an HICON (spec §5).

use crate::color::{Rgb, ensure_contrast, mix, text_on};
use crate::config::TrayStyle;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, FindWindowW, HICON, ICONINFO, SM_CXSMICON,
};
use windows::core::{HSTRING, Interface, w};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconContent {
    /// 0–100.
    Value(u8),
    /// Unresponsive monitor: dimmed "–".
    Dash,
    /// Single-icon mode: sun glyph (Segoe Fluent Icons / MDL2 U+E706).
    Sun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconSpec {
    pub content: IconContent,
    /// The user's color; the renderer applies the contrast safeguard itself.
    pub color: Rgb,
    pub style: TrayStyle,
    pub taskbar_bg: Rgb,
    pub badge: Option<char>,
    pub dimmed: bool,
    /// Pixel size (square), from `tray_icon_size()`.
    pub size: u32,
}

/// HICON destroyed on drop.
pub struct OwnedIcon(pub HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: we own the icon handle; failure (already destroyed) is ignorable.
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

/// Fractions of the icon size; "tune on real hardware" (tested sizes 16/20/24/32).
const DIGIT_HEIGHT: f32 = 0.80; // digit ink height / icon height
const SUN_SIZE: f32 = 0.85;
const BADGE_HEIGHT: f32 = 0.45;
/// Below this size the badge is unreadable and is skipped.
const BADGE_MIN_SIZE: u32 = 20;

/// Which font a piece of text uses.
#[derive(Clone, Copy, PartialEq)]
enum Face {
    Digits,
    Icons,
}

/// Ink bounds relative to the layout origin.
struct Ink {
    left: f32,
    top: f32,
    w: f32,
    h: f32,
}

pub struct IconRenderer {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    /// "Bahnschrift" (condensed) or "Segoe UI" fallback.
    digit_font: (&'static str, DWRITE_FONT_STRETCH),
    /// "Segoe Fluent Icons" (Win11) or "Segoe MDL2 Assets" (Win10).
    icon_font: &'static str,
}

pub(super) fn has_family(dwrite: &IDWriteFactory, name: &str) -> bool {
    let mut coll = None;
    // SAFETY: out-pointers are valid locals.
    unsafe {
        if dwrite.GetSystemFontCollection(&mut coll, false).is_err() {
            return false;
        }
        let Some(coll) = coll else { return false };
        let (mut idx, mut exists) = (0u32, false.into());
        coll.FindFamilyName(&HSTRING::from(name), &mut idx, &mut exists)
            .is_ok()
            && exists.as_bool()
    }
}

pub(super) fn color_f(c: Rgb, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a,
    }
}

impl IconRenderer {
    /// Requires COM initialized on the calling thread.
    pub fn new() -> windows::core::Result<IconRenderer> {
        // SAFETY: plain factory creation calls.
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let digit_font = if has_family(&dwrite, "Bahnschrift") {
                ("Bahnschrift", DWRITE_FONT_STRETCH_CONDENSED)
            } else {
                ("Segoe UI", DWRITE_FONT_STRETCH_NORMAL)
            };
            let icon_font = if has_family(&dwrite, "Segoe Fluent Icons") {
                "Segoe Fluent Icons"
            } else {
                "Segoe MDL2 Assets"
            };
            Ok(IconRenderer {
                d2d,
                dwrite,
                wic,
                digit_font,
                icon_font,
            })
        }
    }

    /// Lays out `text` at `font_size` and measures its ink box.
    fn layout(
        &self,
        text: &str,
        face: Face,
        font_size: f32,
    ) -> windows::core::Result<(IDWriteTextLayout, Ink)> {
        let (family, stretch, weight) = match face {
            Face::Digits => (
                self.digit_font.0,
                self.digit_font.1,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
            ),
            Face::Icons => (
                self.icon_font,
                DWRITE_FONT_STRETCH_NORMAL,
                DWRITE_FONT_WEIGHT_NORMAL,
            ),
        };
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: all pointers are to valid locals; interfaces are owned COM objects.
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                &HSTRING::from(family),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                stretch,
                font_size,
                w!("en-us"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let layout = self
                .dwrite
                .CreateTextLayout(&wide, &format, 4096.0, 4096.0)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            // Shrink the box to the text so overhang metrics are relative to the text extent.
            layout.SetMaxWidth(m.widthIncludingTrailingWhitespace)?;
            layout.SetMaxHeight(m.height)?;
            let o = layout.GetOverhangMetrics()?;
            let ink = Ink {
                left: -o.left,
                top: -o.top,
                w: m.widthIncludingTrailingWhitespace + o.left + o.right,
                h: m.height + o.top + o.bottom,
            };
            Ok((layout, ink))
        }
    }

    /// Font size at which `text`'s ink fits `max_w` x `max_h` (limited by the tighter axis).
    fn fit(&self, text: &str, face: Face, max_w: f32, max_h: f32) -> windows::core::Result<f32> {
        const REF: f32 = 100.0;
        let (_, ink) = self.layout(text, face, REF)?;
        let scale = (max_w / ink.w.max(1.0)).min(max_h / ink.h.max(1.0));
        Ok((REF * scale).max(4.0))
    }

    /// Draws `text` with its ink box placed by `place(ink) -> (x, y)` of the ink's top-left.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        rt: &ID2D1RenderTarget,
        brush: &ID2D1SolidColorBrush,
        text: &str,
        face: Face,
        font_size: f32,
        place: impl Fn(&Ink) -> (f32, f32),
        offset: (f32, f32),
    ) -> windows::core::Result<()> {
        let (layout, ink) = self.layout(text, face, font_size)?;
        let (x, y) = place(&ink);
        // Snap to whole pixels so grid-fitted stems stay crisp.
        let origin = windows_numerics::Vector2 {
            X: (x - ink.left).round() + offset.0,
            Y: (y - ink.top).round() + offset.1,
        };
        // SAFETY: valid layout/brush from the same render target.
        unsafe { rt.DrawTextLayout(origin, &layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE) };
        Ok(())
    }

    pub fn render(&self, spec: &IconSpec) -> windows::core::Result<OwnedIcon> {
        let size = spec.size.max(1);
        let s = size as f32;
        let dimmed = spec.dimmed || spec.content == IconContent::Dash;
        let bg = spec.taskbar_bg;

        // Colors: fill (pill or text), text, and badge.
        let base = if dimmed {
            mix(spec.color, bg, 0.55)
        } else {
            spec.color
        };
        let pill = spec.style == TrayStyle::Pill && spec.content != IconContent::Sun;
        let text_color = if pill {
            text_on(base)
        } else {
            ensure_contrast(base, bg, 3.0)
        };

        // SAFETY: all COM calls on objects we own; raw buffers sized from `size`.
        unsafe {
            let bitmap = self.wic.CreateBitmap(
                size,
                size,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_UNKNOWN,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = self.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
            // ClearType needs an opaque target: use grayscale AA with grid-fitted rendering params.
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            if let Ok(f2) = self.dwrite.cast::<IDWriteFactory2>() {
                if let Ok(params) = f2.CreateCustomRenderingParams(
                    1.0,
                    0.5,
                    1.0,
                    0.0,
                    DWRITE_PIXEL_GEOMETRY_FLAT,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    DWRITE_GRID_FIT_MODE_ENABLED,
                ) {
                    rt.SetTextRenderingParams(&params);
                }
            }

            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));

            let r = (|| -> windows::core::Result<()> {
                if pill {
                    let radius = if size < 24 { 2.0 } else { 3.0 };
                    let b = rt.CreateSolidColorBrush(&color_f(base, 1.0), None)?;
                    rt.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: 0.0,
                                top: 0.0,
                                right: s,
                                bottom: s,
                            },
                            radiusX: radius,
                            radiusY: radius,
                        },
                        &b,
                    );
                }
                let brush = rt.CreateSolidColorBrush(&color_f(text_color, 1.0), None)?;
                let center = |ink: &Ink| ((s - ink.w) / 2.0, (s - ink.h) / 2.0);
                match spec.content {
                    IconContent::Value(v) => {
                        let text = v.min(100).to_string();
                        let fs = self.fit(&text, Face::Digits, s, s * DIGIT_HEIGHT)?;
                        self.draw(&rt, &brush, &text, Face::Digits, fs, center, (0.0, 0.0))?;
                    }
                    IconContent::Dash => {
                        // Same size as two digits so the dash looks like a placeholder number.
                        let fs = self.fit("00", Face::Digits, s, s * DIGIT_HEIGHT)?;
                        self.draw(
                            &rt,
                            &brush,
                            "\u{2013}",
                            Face::Digits,
                            fs,
                            center,
                            (0.0, 0.0),
                        )?;
                    }
                    IconContent::Sun => {
                        let fs = self.fit("\u{E706}", Face::Icons, s * SUN_SIZE, s * SUN_SIZE)?;
                        self.draw(&rt, &brush, "\u{E706}", Face::Icons, fs, center, (0.0, 0.0))?;
                    }
                }
                if let Some(c) = spec.badge.filter(|_| size >= BADGE_MIN_SIZE) {
                    let text = c.to_uppercase().to_string();
                    let fs = self.fit(&text, Face::Digits, s * 0.6, s * BADGE_HEIGHT)?;
                    let corner = |ink: &Ink| (s - ink.w, s - ink.h);
                    if !pill {
                        // 1 px backing in the taskbar color so the badge separates from the digits.
                        let back = rt.CreateSolidColorBrush(&color_f(bg, 1.0), None)?;
                        for (dx, dy) in [
                            (-1.0, 0.0),
                            (1.0, 0.0),
                            (0.0, -1.0),
                            (0.0, 1.0),
                            (-1.0, -1.0),
                            (1.0, 1.0),
                            (-1.0, 1.0),
                            (1.0, -1.0),
                        ] {
                            self.draw(&rt, &back, &text, Face::Digits, fs, corner, (dx, dy))?;
                        }
                    }
                    self.draw(&rt, &brush, &text, Face::Digits, fs, corner, (0.0, 0.0))?;
                }
                Ok(())
            })();
            let end = rt.EndDraw(None, None);
            r?;
            end?;

            // Copy into a top-down 32bpp DIB, un-premultiplying alpha.
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size as i32,
                    biHeight: -(size as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let n = (size * size * 4) as usize;
            // SAFETY: the DIB section owns size*size*4 bytes at `bits`.
            let buf = std::slice::from_raw_parts_mut(bits as *mut u8, n);
            if let Err(e) = bitmap.CopyPixels(std::ptr::null(), size * 4, buf) {
                let _ = DeleteObject(color.into());
                return Err(e);
            }
            for px in buf.chunks_exact_mut(4) {
                let a = px[3] as u32;
                if a != 0 && a != 255 {
                    for c in &mut px[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
            let info = ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask,
                hbmColor: color,
            };
            let icon = CreateIconIndirect(&info);
            let _ = DeleteObject(color.into());
            let _ = DeleteObject(mask.into());
            Ok(OwnedIcon(icon?))
        }
    }
}

/// Small-icon size for the taskbar's DPI (`GetSystemMetricsForDpi(SM_CXSMICON)`).
pub fn tray_icon_size() -> u32 {
    // SAFETY: plain Win32 queries.
    unsafe {
        let tray: HWND = FindWindowW(w!("Shell_TrayWnd"), None).unwrap_or_default();
        let mut dpi = if tray.0.is_null() {
            0
        } else {
            GetDpiForWindow(tray)
        };
        if dpi == 0 {
            dpi = GetDpiForSystem();
        }
        (GetSystemMetricsForDpi(SM_CXSMICON, dpi.max(96)).max(16)) as u32
    }
}
