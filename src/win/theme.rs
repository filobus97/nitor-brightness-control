//! App and taskbar theme from the registry and DWM.

use crate::color::{Rgb, mix};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::{BOOL, PCWSTR, w};

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

const PERSONALIZE: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

fn read_dword(root: HKEY, key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `data`/`size` are valid for the duration of the call; size matches the buffer.
    let err = unsafe {
        RegGetValueW(
            root,
            key,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    (err == ERROR_SUCCESS).then_some(data)
}

fn read_build_number() -> Option<u32> {
    let mut buf = [0u16; 32];
    let mut size = size_of_val(&buf) as u32;
    // SAFETY: `buf`/`size` are valid; size is in bytes as required.
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().parse().ok()
}

fn read_accent() -> Rgb {
    if let Some(v) = read_dword(
        HKEY_CURRENT_USER,
        w!(r"Software\Microsoft\Windows\DWM"),
        w!("AccentColor"),
    ) {
        // 0xAABBGGRR
        return Rgb::new(v as u8, (v >> 8) as u8, (v >> 16) as u8);
    }
    let mut c = 0u32;
    let mut opaque = BOOL(0);
    // SAFETY: both out-pointers are valid locals.
    if unsafe { DwmGetColorizationColor(&mut c, &mut opaque) }.is_ok() {
        // 0xAARRGGBB
        return Rgb::new((c >> 16) as u8, (c >> 8) as u8, c as u8);
    }
    Rgb::new(0x00, 0x78, 0xD4)
}

impl Theme {
    pub fn read() -> Theme {
        let user = |name| read_dword(HKEY_CURRENT_USER, PERSONALIZE, name);
        let is_win11 = read_build_number().is_some_and(|b| b >= 22000);
        Theme {
            apps_light: user(w!("AppsUseLightTheme")).unwrap_or(1) != 0,
            // Absent value: the Windows 10 taskbar defaults to dark.
            taskbar_light: user(w!("SystemUsesLightTheme")).unwrap_or(0) != 0,
            taskbar_accent: user(w!("ColorPrevalence")).unwrap_or(0) != 0,
            accent: read_accent(),
            is_win11,
        }
    }

    /// Approximate taskbar background color used for the contrast safeguard.
    pub fn taskbar_bg(&self) -> Rgb {
        if self.taskbar_accent {
            // Windows paints a darker shade of the accent.
            return mix(self.accent, Rgb::BLACK, 0.4);
        }
        match (self.taskbar_light, self.is_win11) {
            (true, true) => Rgb::new(0xF3, 0xF3, 0xF3),
            (true, false) => Rgb::new(0xEE, 0xEE, 0xEE),
            (false, true) => Rgb::new(0x20, 0x20, 0x20),
            (false, false) => Rgb::new(0x10, 0x10, 0x10),
        }
    }
}
