//! Windows "Change brightness automatically when lighting changes" detection.

use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerGetActiveScheme, PowerReadACValueIndex, PowerReadDCValueIndex,
    SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_VIDEO_ADAPTIVE_DISPLAY_BRIGHTNESS, GUID_VIDEO_SUBGROUP,
};

/// Whether Windows adaptive brightness is on in the active power scheme (for the current
/// power source). It would fight an automated built-in panel; the UI only shows a hint.
/// `false` when it cannot be read. One cheap powrprof query, called on demand.
pub fn adaptive_brightness_on() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: valid out-pointer.
    let on_ac = unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.ACLineStatus == 1;
    let mut scheme = std::ptr::null_mut();
    // SAFETY: on success `scheme` is a LocalAlloc'ed GUID that we free below; the GUID
    // statics and `index` outlive the calls.
    unsafe {
        if PowerGetActiveScheme(None, &mut scheme) != ERROR_SUCCESS || scheme.is_null() {
            return false;
        }
        let mut index = 0u32;
        let ok = if on_ac {
            PowerReadACValueIndex(
                None,
                Some(scheme),
                Some(&GUID_VIDEO_SUBGROUP),
                Some(&GUID_VIDEO_ADAPTIVE_DISPLAY_BRIGHTNESS),
                &mut index,
            ) == ERROR_SUCCESS
        } else {
            PowerReadDCValueIndex(
                None,
                Some(scheme),
                Some(&GUID_VIDEO_SUBGROUP),
                Some(&GUID_VIDEO_ADAPTIVE_DISPLAY_BRIGHTNESS),
                &mut index,
            ) == ERROR_SUCCESS.0
        };
        let _ = LocalFree(Some(HLOCAL(scheme.cast())));
        ok && index != 0
    }
}
