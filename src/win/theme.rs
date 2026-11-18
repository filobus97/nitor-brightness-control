//! App and taskbar theme from the registry and DWM.

use crate::color::Rgb;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// `AppsUseLightTheme` (flyout).
    pub apps_light: bool,
    /// `SystemUsesLightTheme` (taskbar).
    pub taskbar_light: bool,
    /// `ColorPrevalence`: taskbar painted with the accent color.
    pub taskbar_accent: bool,
    pub accent: Rgb,
    /// Build ≥ 22000.
    pub is_win11: bool,
}

impl Theme {
    pub fn read() -> Theme {
        todo!()
    }

    /// Approximate taskbar background color used for the contrast safeguard.
    pub fn taskbar_bg(&self) -> Rgb {
        todo!()
    }
}
