//! Tray icon rendering with Direct2D + DirectWrite into an HICON (spec §5).

use crate::color::Rgb;
use crate::config::TrayStyle;
use windows::Win32::UI::WindowsAndMessaging::HICON;

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
        todo!()
    }
}

pub struct IconRenderer {
    // implementation-defined (D2D, DWrite, WIC factories)
}

impl IconRenderer {
    /// Requires COM initialized on the calling thread.
    pub fn new() -> windows::core::Result<IconRenderer> {
        todo!()
    }

    pub fn render(&self, spec: &IconSpec) -> windows::core::Result<OwnedIcon> {
        let _ = spec;
        todo!()
    }
}

/// Small-icon size for the taskbar's DPI (`GetSystemMetricsForDpi(SM_CXSMICON)`).
pub fn tray_icon_size() -> u32 {
    todo!()
}
