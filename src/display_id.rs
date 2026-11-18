//! Matching Windows display identities across APIs (platform-independent, unit-tested).
//!
//! DisplayConfig reports `\\?\DISPLAY#BOE0867#4&2c3a5f5&0&UID8388688#{guid}`, WMI reports
//! `DISPLAY\BOE0867\4&2c3a5f5&0&UID8388688_0`. Both reduce to the same key.

/// `MODEL|INSTANCE` in upper case from a monitor device path or a WMI `InstanceName`;
/// the `_N` suffix WMI appends to the instance is dropped. `None` if it is not a display id.
pub fn display_key(id: &str) -> Option<String> {
    let mut it = id.trim_start_matches(r"\\?\").split(['#', '\\']);
    let (class, model, inst) = (it.next()?, it.next()?, it.next()?);
    if !class.eq_ignore_ascii_case("DISPLAY") || model.is_empty() || inst.is_empty() {
        return None;
    }
    let inst = match inst.rsplit_once('_') {
        Some((head, n))
            if !head.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => inst,
    };
    Some(format!("{model}|{inst}").to_uppercase())
}

/// Escape a string for a single-quoted WQL literal (backslash and quote).
pub fn wql_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('\'', r"\'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wmi_name_and_device_path_match() {
        let wmi = r"DISPLAY\BOE0867\4&2c3a5f5&0&UID8388688_0";
        let path =
            r"\\?\DISPLAY#BOE0867#4&2c3a5f5&0&UID8388688#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
        assert_eq!(
            display_key(wmi),
            Some("BOE0867|4&2C3A5F5&0&UID8388688".into())
        );
        assert_eq!(display_key(wmi), display_key(path));
        assert_eq!(display_key(&path.to_lowercase()), display_key(path));
    }

    #[test]
    fn different_panels_differ() {
        let a = display_key(r"DISPLAY\BOE0867\4&2c3a5f5&0&UID1_0");
        let b = display_key(r"DISPLAY\BOE0867\4&2c3a5f5&0&UID2_0");
        assert_ne!(a, b);
    }

    #[test]
    fn keeps_non_numeric_underscore_suffix() {
        assert_eq!(
            display_key(r"DISPLAY\AAA0001\X_Y"),
            Some("AAA0001|X_Y".into())
        );
    }

    #[test]
    fn rejects_other_ids() {
        assert_eq!(display_key(r"\\?\USB#VID_1#2#{g}"), None);
        assert_eq!(display_key("DISPLAY"), None);
        assert_eq!(display_key(r"DISPLAY\\x"), None);
    }

    #[test]
    fn escapes_wql() {
        assert_eq!(wql_escape(r"DISPLAY\A'B"), r"DISPLAY\\A\'B");
    }
}
