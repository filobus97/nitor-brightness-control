//! Small helpers shared by the Windows modules.

use std::path::{Path, PathBuf};

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
    exe_path().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

pub fn appdata_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

/// The user's preferred UI language, e.g. `it-IT` (`GetUserDefaultLocaleName` fallback).
pub fn ui_locale() -> String {
    todo!()
}

/// `AttachConsole(ATTACH_PARENT_PROCESS)`; afterwards `println!` reaches the parent console.
pub fn attach_parent_console() -> bool {
    todo!()
}

/// Open a file with its default application (`ShellExecuteW` "open").
pub fn open_with_default_app(path: &Path) {
    let _ = path;
    todo!()
}

/// Panic hook that appends the panic message to `log_path`, then exits the process.
pub fn install_panic_hook(log_path: PathBuf) {
    let _ = log_path;
    todo!()
}
