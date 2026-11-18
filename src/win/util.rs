//! Small helpers shared by the Windows modules.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Globalization::{
    GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, PWSTR};

/// Ask Windows to trim this process's working set. A tray app sits idle for days, so this
/// releases the pages touched at startup or by a closed window (they fault back in if
/// needed); it is not a leak workaround.
pub fn trim_working_set() {
    // SAFETY: the pseudo-handle is always valid; (-1, -1) means "empty the working set".
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

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
    shell_open(&path.to_string_lossy());
}

/// Open a web page in the default browser.
pub fn open_url(url: &str) {
    shell_open(url);
}

fn shell_open(target: &str) {
    let file = wide(target);
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

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());
/// The log is emptied when it grows past this.
const LOG_MAX_BYTES: u64 = 256 * 1024;

/// Where `log` writes (`Nitor.log` next to the config). Set once at startup.
pub fn set_log_path(path: PathBuf) {
    let _ = LOG_PATH.set(path);
}

/// Append `<unix secs> <line>` to the log; thread-safe, best effort, no-op before `set_log_path`.
pub fn log(line: &str) {
    let Some(path) = LOG_PATH.get() else { return };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let _guard = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::metadata(path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
        let _ = std::fs::write(path, "");
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{secs} {line}");
    }
}

/// Panic hook that appends the panic message to the log.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| log(&info.to_string())));
}
