//! "Start with Windows" via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Nitor`.

pub fn is_enabled() -> bool {
    todo!()
}

/// Write (quoted current exe path) or delete the Run value.
pub fn set(enabled: bool) -> windows::core::Result<()> {
    let _ = enabled;
    todo!()
}

/// If the Run value exists but points to a different path, rewrite it to the
/// current exe and return `true`.
pub fn fix_stale() -> bool {
    todo!()
}
