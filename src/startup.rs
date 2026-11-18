//! Start-with-Windows logic that needs no Win32: the `--autostart` flag, how the Run
//! entry is judged, and the retry schedule for monitors that are not ready at logon.

/// Added to the Run entry, so the app knows it was started by Windows at logon.
pub const AUTOSTART_FLAG: &str = "--autostart";

/// Delay of the first scan after a start at logon, in milliseconds.
pub const AUTOSTART_DELAY_MS: u32 = 3_000;

/// Delays before the automatic re-scans when monitors did not answer, in seconds.
const RETRY_DELAYS_SECS: [u32; 4] = [5, 15, 30, 60];

/// The delay in milliseconds before re-scan number `step` (0-based); `None` when exhausted.
pub fn retry_delay_ms(step: usize) -> Option<u32> {
    RETRY_DELAYS_SECS.get(step).map(|s| s * 1000)
}

/// Remove `flag` from `args`; whether it was there.
pub fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != flag);
    args.len() != before
}

/// The Run value for `exe`: quoted path plus the autostart flag.
pub fn run_value(exe: &str) -> String {
    format!("\"{exe}\" {AUTOSTART_FLAG}")
}

/// What a stored Run value needs.
#[derive(Debug, PartialEq, Eq)]
pub enum RunEntry {
    /// Current exe, with the flag.
    Current,
    /// Current exe, written by an older version (no flag).
    NeedsFlag,
    /// Points elsewhere.
    Stale,
}

pub fn classify_run(stored: &str, exe: &str) -> RunEntry {
    let stored = stored.trim();
    let (path, flagged) = match stored.strip_suffix(AUTOSTART_FLAG) {
        Some(rest) => (rest.trim_end(), true),
        None => (stored, false),
    };
    if !path.trim_matches('"').eq_ignore_ascii_case(exe) {
        RunEntry::Stale
    } else if flagged {
        RunEntry::Current
    } else {
        RunEntry::NeedsFlag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_schedule_backs_off_then_stops() {
        let all: Vec<_> = (0..6).map(retry_delay_ms).collect();
        assert_eq!(
            all,
            [
                Some(5_000),
                Some(15_000),
                Some(30_000),
                Some(60_000),
                None,
                None
            ]
        );
    }

    #[test]
    fn take_flag_removes_every_copy() {
        let mut args = vec!["--autostart".to_string(), "list".into()];
        assert!(take_flag(&mut args, AUTOSTART_FLAG));
        assert_eq!(args, ["list"]);
        assert!(!take_flag(&mut args, AUTOSTART_FLAG));
    }

    #[test]
    fn run_entries_are_classified() {
        let exe = r"C:\Program Files\Nitor\Nitor.exe";
        assert_eq!(classify_run(&run_value(exe), exe), RunEntry::Current);
        assert_eq!(
            classify_run(&run_value(&exe.to_uppercase()), exe),
            RunEntry::Current
        );
        assert_eq!(
            classify_run(&format!("\"{exe}\""), exe),
            RunEntry::NeedsFlag
        );
        assert_eq!(classify_run(exe, exe), RunEntry::NeedsFlag);
        assert_eq!(
            classify_run(r#""D:\Old\Nitor.exe" --autostart"#, exe),
            RunEntry::Stale
        );
        assert_eq!(classify_run(r#""D:\Old\Nitor.exe""#, exe), RunEntry::Stale);
    }
}
