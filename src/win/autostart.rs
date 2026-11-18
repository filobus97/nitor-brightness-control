//! "Start with Windows" via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Nitor`.

use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, Result};

use super::util::{exe_path, from_wide, wide};

const SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "Nitor";

/// Current Run value, if present.
fn read() -> Option<String> {
    let (key, name) = (wide(SUBKEY), wide(VALUE));
    let mut buf = [0u16; 1024];
    let mut bytes = std::mem::size_of_val(&buf) as u32;
    // SAFETY: `buf` and `bytes` describe the same writable buffer; strings are NUL-terminated.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    (rc == ERROR_SUCCESS).then(|| from_wide(&buf))
}

fn write() -> WIN32_ERROR {
    let (key, name) = (wide(SUBKEY), wide(VALUE));
    let data = wide(&format!("\"{}\"", exe_path().display()));
    // SAFETY: `data` is a NUL-terminated UTF-16 buffer; the size is in bytes incl. the NUL.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    }
}

pub fn is_enabled() -> bool {
    read().is_some()
}

/// Write (quoted current exe path) or delete the Run value.
pub fn set(enabled: bool) -> Result<()> {
    let rc = if enabled {
        write()
    } else {
        let (key, name) = (wide(SUBKEY), wide(VALUE));
        // SAFETY: strings are NUL-terminated and outlive the call.
        let rc = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
            )
        };
        // Deleting a value that does not exist is fine.
        if rc == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
            ERROR_SUCCESS
        } else {
            rc
        }
    };
    rc.ok()
}

/// If the Run value exists but points to a different path, rewrite it to the
/// current exe and return `true`.
pub fn fix_stale() -> bool {
    let Some(cur) = read() else { return false };
    let stored = cur.trim().trim_matches('"');
    let exe = exe_path();
    if stored.eq_ignore_ascii_case(&exe.to_string_lossy()) {
        return false;
    }
    write() == ERROR_SUCCESS
}
