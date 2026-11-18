//! Parsing hotkey strings such as `Ctrl+Alt+PgUp` into Win32 modifier flags and
//! virtual-key codes. The numeric values match `RegisterHotKey`, but this module
//! has no Windows dependency.

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    /// Combination of the `MOD_*` flags above.
    pub mods: u32,
    /// Win32 virtual-key code.
    pub vk: u32,
}

/// Parse e.g. `Ctrl+Alt+PgUp`, `Win+Shift+F9`, `ctrl + alt + 1`.
/// Modifiers: Ctrl/Control, Alt, Shift, Win. Keys: A–Z, 0–9, F1–F24,
/// PgUp/PageUp, PgDn/PageDown, Home, End, Insert, Delete, Up, Down, Left, Right,
/// Space, Plus, Minus, Num0–Num9, NumPlus, NumMinus. At least one modifier is required.
pub fn parse(s: &str) -> Option<Hotkey> {
    let mut mods = 0;
    let mut vk = None;
    for part in s.split('+') {
        let part = part.trim().to_ascii_lowercase();
        let flag = match part.as_str() {
            "ctrl" | "control" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" => MOD_WIN,
            _ => 0,
        };
        if flag != 0 {
            mods |= flag;
            continue;
        }
        if vk.is_some() {
            return None;
        }
        vk = Some(key_vk(&part)?);
    }
    let vk = vk?;
    (mods != 0).then_some(Hotkey { mods, vk })
}

/// Canonical display form, e.g. `Ctrl+Alt+PgUp`.
pub fn format(hk: Hotkey) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (flag, name) in [
        (MOD_CONTROL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
        (MOD_WIN, "Win"),
    ] {
        if hk.mods & flag != 0 {
            parts.push(name.to_string());
        }
    }
    parts.push(vk_name(hk.vk).unwrap_or_else(|| format!("0x{:02X}", hk.vk)));
    parts.join("+")
}

/// Virtual-key code for a lowercase key name.
fn key_vk(lower: &str) -> Option<u32> {
    let single = {
        let mut chars = lower.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    };
    if let Some(c) = single {
        return match c {
            'a'..='z' => Some(0x41 + (c as u32 - 'a' as u32)),
            '0'..='9' => Some(0x30 + (c as u32 - '0' as u32)),
            _ => None,
        };
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match lower {
        "space" => Some(0x20),
        "pgup" | "pageup" => Some(0x21),
        "pgdn" | "pagedown" => Some(0x22),
        "end" => Some(0x23),
        "home" => Some(0x24),
        "left" => Some(0x25),
        "up" => Some(0x26),
        "right" => Some(0x27),
        "down" => Some(0x28),
        "insert" => Some(0x2D),
        "delete" => Some(0x2E),
        "numplus" => Some(0x6B),
        "numminus" => Some(0x6D),
        "plus" => Some(0xBB),
        "minus" => Some(0xBD),
        _ => {
            if let Some(n) = lower.strip_prefix("num").filter(|n| digits(n)) {
                let n: u32 = n.parse().ok()?;
                (n <= 9).then_some(0x60 + n)
            } else if let Some(n) = lower.strip_prefix('f').filter(|n| digits(n)) {
                let n: u32 = n.parse().ok()?;
                (1..=24).contains(&n).then_some(0x6F + n)
            } else {
                None
            }
        }
    }
}

/// Canonical name for a virtual-key code; inverse of `key_vk`.
fn vk_name(vk: u32) -> Option<String> {
    let name = match vk {
        0x20 => "Space".to_string(),
        0x21 => "PgUp".to_string(),
        0x22 => "PgDn".to_string(),
        0x23 => "End".to_string(),
        0x24 => "Home".to_string(),
        0x25 => "Left".to_string(),
        0x26 => "Up".to_string(),
        0x27 => "Right".to_string(),
        0x28 => "Down".to_string(),
        0x2D => "Insert".to_string(),
        0x2E => "Delete".to_string(),
        0x30..=0x39 => char::from(b'0' + (vk - 0x30) as u8).to_string(),
        0x41..=0x5A => char::from(b'A' + (vk - 0x41) as u8).to_string(),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        0x6B => "NumPlus".to_string(),
        0x6D => "NumMinus".to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0xBB => "Plus".to_string(),
        0xBD => "Minus".to_string(),
        _ => return None,
    };
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_combinations() {
        assert_eq!(
            parse("Ctrl+Alt+PgUp"),
            Some(Hotkey {
                mods: MOD_CONTROL | MOD_ALT,
                vk: 0x21
            })
        );
        assert_eq!(
            parse("Win+Shift+F9"),
            Some(Hotkey {
                mods: MOD_WIN | MOD_SHIFT,
                vk: 0x78
            })
        );
        assert_eq!(
            parse("ctrl + alt + 1"),
            Some(Hotkey {
                mods: MOD_CONTROL | MOD_ALT,
                vk: 0x31
            })
        );
        assert_eq!(
            parse("Control+a"),
            Some(Hotkey {
                mods: MOD_CONTROL,
                vk: 0x41
            })
        );
    }

    #[test]
    fn parses_named_keys() {
        let vk = |s: &str| parse(&format!("Ctrl+{s}")).map(|h| h.vk);
        assert_eq!(vk("PageUp"), Some(0x21));
        assert_eq!(vk("PageDown"), Some(0x22));
        assert_eq!(vk("F1"), Some(0x70));
        assert_eq!(vk("F24"), Some(0x87));
        assert_eq!(vk("F25"), None);
        assert_eq!(vk("F0"), None);
        assert_eq!(vk("Num0"), Some(0x60));
        assert_eq!(vk("Num9"), Some(0x69));
        assert_eq!(vk("NumPlus"), Some(0x6B));
        assert_eq!(vk("NumMinus"), Some(0x6D));
        assert_eq!(vk("Plus"), Some(0xBB));
        assert_eq!(vk("Minus"), Some(0xBD));
        assert_eq!(vk("Space"), Some(0x20));
        assert_eq!(vk("Insert"), Some(0x2D));
        assert_eq!(vk("Delete"), Some(0x2E));
        assert_eq!(vk("Left"), Some(0x25));
        assert_eq!(vk("Down"), Some(0x28));
        assert_eq!(vk("Bogus"), None);
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("PgUp"), None, "modifier required");
        assert_eq!(parse("Ctrl"), None, "key required");
        assert_eq!(parse("Ctrl+"), None);
        assert_eq!(parse("Ctrl+A+B"), None, "only one key");
        assert_eq!(parse("Ctrl+Bogus"), None);
    }

    #[test]
    fn format_canonical_order() {
        assert_eq!(
            format(Hotkey {
                mods: MOD_CONTROL | MOD_ALT,
                vk: 0x21
            }),
            "Ctrl+Alt+PgUp"
        );
        assert_eq!(
            format(Hotkey {
                mods: MOD_SHIFT | MOD_WIN,
                vk: 0x78
            }),
            "Shift+Win+F9"
        );
        assert_eq!(
            format(Hotkey {
                mods: 0x0F,
                vk: 0x41
            }),
            "Ctrl+Alt+Shift+Win+A"
        );
        assert_eq!(
            format(Hotkey {
                mods: MOD_CONTROL,
                vk: 0x6B
            }),
            "Ctrl+NumPlus"
        );
    }

    #[test]
    fn round_trips() {
        let mut vks: Vec<u32> = vec![0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28];
        vks.extend([0x2D, 0x2E, 0x6B, 0x6D, 0xBB, 0xBD]);
        vks.extend(0x30..=0x39);
        vks.extend(0x41..=0x5A);
        vks.extend(0x60..=0x69);
        vks.extend(0x70..=0x87);
        for vk in vks {
            for mods in 1..=0x0Fu32 {
                let hk = Hotkey { mods, vk };
                let text = format(hk);
                assert_eq!(parse(&text), Some(hk), "round trip of {text}");
            }
        }
    }
}
