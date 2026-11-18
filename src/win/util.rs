//! Small helpers shared by the Windows modules.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Globalization::{
    GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, PWSTR};

/// UTF-16, NUL-terminated.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 up to the first NUL.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("Nitor.exe"))
}

pub fn exe_dir() -> PathBuf {
    exe_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn appdata_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

/// The user's preferred UI language, e.g. `it-IT` (`GetUserDefaultLocaleName` fallback).
pub fn ui_locale() -> String {
    let mut buf = [0u16; 512];
    let mut count = 0u32;
    let mut len = buf.len() as u32;
    // SAFETY: `buf`/`len` describe the same writable buffer.
    let ok = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    };
    // The buffer is a double-NUL-terminated list; `from_wide` yields the first entry.
    let first = if ok.is_ok() && count > 0 {
        from_wide(&buf)
    } else {
        String::new()
    };
    if !first.is_empty() {
        return first;
    }
    let mut name = [0u16; 85];
    // SAFETY: writable buffer of the stated length.
    let n = unsafe { GetUserDefaultLocaleName(&mut name) };
    let s = if n > 0 {
        from_wide(&name)
    } else {
        String::new()
    };
    if s.is_empty() { "en-US".to_owned() } else { s }
}

/// `AttachConsole(ATTACH_PARENT_PROCESS)`; afterwards `println!` reaches the parent console.
pub fn attach_parent_console() -> bool {
    // SAFETY: no pointers involved.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok()
}

/// Open a file with its default application (`ShellExecuteW` "open").
pub fn open_with_default_app(path: &Path) {
    let file = wide(&path.to_string_lossy());
    let verb = wide("open");
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Panic hook that appends the panic message to `log_path`, then exits the process.
pub fn install_panic_hook(log_path: PathBuf) {
    std::panic::set_hook(Box::new(move |info| {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = writeln!(f, "{secs} {info}");
        }
    }));
}
