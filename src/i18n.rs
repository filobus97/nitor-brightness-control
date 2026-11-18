//! UI strings. Add a language by adding a `Lang` variant and one arm per key in `tr`.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    It,
}

impl Lang {
    /// From a BCP-47 locale name such as `it-IT` or `en-US`; unknown → English.
    pub fn from_locale(name: &str) -> Lang {
        let language = name.trim().split(['-', '_']).next().unwrap_or("");
        if language.eq_ignore_ascii_case("it") {
            Lang::It
        } else {
            Lang::En
        }
    }

    /// From the config value: `auto` (or empty) → `None`, `en`/`it` → that language.
    pub fn from_setting(value: &str) -> Option<Lang> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => None,
            "en" => Some(Lang::En),
            "it" => Some(Lang::It),
            _ => None,
        }
    }
}

/// Every user-visible string. Keep this list in sync with `tr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    AppName,
    MenuRescan,
    MenuSettings,
    MenuStartWithWindows,
    MenuInternalDisplay,
    MenuExit,
    MenuCheckUpdates,
    /// Menu item to install a found update; `{}` is the version.
    MenuUpdateTo,
    /// Update balloons; `{}` is the version (or the error for `UpdateFailed`).
    UpdateAvailable,
    UpToDate,
    UpdateFailed,
    Updated,
    /// Tooltip suffix for a monitor that doesn't answer.
    Unresponsive,
    /// Tooltip suffix for a monitor that rejects writes.
    ReadOnly,
    /// Explanation appended to the read-only tooltip.
    ReadOnlyHint,
    /// Tray tooltip when no controllable monitor was found.
    NoMonitors,
    /// First-run balloon title and text about pinning icons.
    FirstRunTitle,
    FirstRunText,
    /// Balloon when the autostart entry pointed to an old location and was fixed.
    AutostartFixed,
    /// Balloon after a crash during detection; monitors were skipped.
    SafeModeText,
    /// Balloon when a hotkey couldn't be registered; `{}` is replaced by the hotkey.
    HotkeyConflict,
    /// Flyout header.
    Brightness,
    LinkAll,
}

/// All keys, for checks that every key is translated.
#[cfg(test)]
const ALL_KEYS: &[Key] = &[
    Key::AppName,
    Key::MenuRescan,
    Key::MenuSettings,
    Key::MenuStartWithWindows,
    Key::MenuInternalDisplay,
    Key::MenuExit,
    Key::MenuCheckUpdates,
    Key::MenuUpdateTo,
    Key::UpdateAvailable,
    Key::UpToDate,
    Key::UpdateFailed,
    Key::Updated,
    Key::Unresponsive,
    Key::ReadOnly,
    Key::ReadOnlyHint,
    Key::NoMonitors,
    Key::FirstRunTitle,
    Key::FirstRunText,
    Key::AutostartFixed,
    Key::SafeModeText,
    Key::HotkeyConflict,
    Key::Brightness,
    Key::LinkAll,
];

/// `tr` for a template: `{}` is replaced by `arg`.
pub fn tr_with(lang: Lang, key: Key, arg: &str) -> String {
    tr(lang, key).replace("{}", arg)
}

pub fn tr(lang: Lang, key: Key) -> &'static str {
    match lang {
        Lang::En => match key {
            Key::AppName => "Nitor",
            Key::MenuRescan => "Rescan monitors",
            Key::MenuSettings => "Settings",
            Key::MenuStartWithWindows => "Start with Windows",
            Key::MenuInternalDisplay => "Control built-in display",
            Key::MenuExit => "Exit",
            Key::MenuCheckUpdates => "Check for updates",
            Key::MenuUpdateTo => "Update to {}",
            Key::UpdateAvailable => "Nitor {} is available",
            Key::UpToDate => "You're up to date ({})",
            Key::UpdateFailed => "Update failed: {}",
            Key::Updated => "Updated to {}",
            Key::Unresponsive => "not responding",
            Key::ReadOnly => "read-only",
            Key::ReadOnlyHint => {
                "A monitor preset (game mode, HDR or driver color settings) is locking brightness. Change the preset on the monitor to control it."
            }
            Key::NoMonitors => "No controllable monitor found",
            Key::FirstRunTitle => "Nitor is in the tray",
            Key::FirstRunText => {
                "Windows may hide icons in the overflow area (^). Drag the Nitor icon onto the taskbar to keep it visible."
            }
            Key::AutostartFixed => {
                "Nitor was moved, so the \"Start with Windows\" entry now points to the new location."
            }
            Key::SafeModeText => {
                "Nitor closed unexpectedly while detecting monitors last time, so those monitors were skipped this time. Use Rescan to try again."
            }
            Key::HotkeyConflict => "Couldn't register hotkey {}: it's used by another app",
            Key::Brightness => "Brightness",
            Key::LinkAll => "Link all",
        },
        Lang::It => match key {
            Key::AppName => "Nitor",
            Key::MenuRescan => "Rileva monitor",
            Key::MenuSettings => "Impostazioni",
            Key::MenuStartWithWindows => "Avvia con Windows",
            Key::MenuInternalDisplay => "Controlla schermo integrato",
            Key::MenuExit => "Esci",
            Key::MenuCheckUpdates => "Controlla aggiornamenti",
            Key::MenuUpdateTo => "Aggiorna a {}",
            Key::UpdateAvailable => "È disponibile Nitor {}",
            Key::UpToDate => "Nitor è aggiornato ({})",
            Key::UpdateFailed => "Aggiornamento non riuscito: {}",
            Key::Updated => "Aggiornato a {}",
            Key::Unresponsive => "non risponde",
            Key::ReadOnly => "sola lettura",
            Key::ReadOnlyHint => {
                "Un preset del monitor (modalità gioco, HDR o impostazioni colore del driver) blocca la luminosità. Cambia il preset sul monitor per controllarla."
            }
            Key::NoMonitors => "Nessun monitor controllabile trovato",
            Key::FirstRunTitle => "Nitor è nell'area di notifica",
            Key::FirstRunText => {
                "Windows può nascondere le icone nell'area di overflow (^). Trascina l'icona di Nitor sulla barra delle applicazioni per tenerla visibile."
            }
            Key::AutostartFixed => {
                "Nitor è stato spostato, quindi la voce \"Avvia con Windows\" ora punta alla nuova posizione."
            }
            Key::SafeModeText => {
                "Nitor si è chiuso in modo imprevisto durante il rilevamento dei monitor l'ultima volta, quindi questa volta quei monitor sono stati saltati. Usa Rileva monitor per riprovare."
            }
            Key::HotkeyConflict => {
                "Impossibile registrare la scorciatoia {}: è usata da un'altra app"
            }
            Key::Brightness => "Luminosità",
            Key::LinkAll => "Collega tutti",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_translated_in_every_language() {
        for lang in [Lang::En, Lang::It] {
            for &key in ALL_KEYS {
                assert!(!tr(lang, key).is_empty(), "{lang:?} {key:?} is empty");
            }
        }
    }

    #[test]
    fn templates_keep_placeholder() {
        let templates = [
            Key::HotkeyConflict,
            Key::MenuUpdateTo,
            Key::UpdateAvailable,
            Key::UpToDate,
            Key::UpdateFailed,
            Key::Updated,
        ];
        for lang in [Lang::En, Lang::It] {
            for key in templates {
                assert!(tr(lang, key).contains("{}"), "{lang:?} {key:?}");
            }
        }
    }

    #[test]
    fn from_locale_uses_language_subtag() {
        assert_eq!(Lang::from_locale("it"), Lang::It);
        assert_eq!(Lang::from_locale("it-IT"), Lang::It);
        assert_eq!(Lang::from_locale("it_CH"), Lang::It);
        assert_eq!(Lang::from_locale("IT-it"), Lang::It);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("fr-FR"), Lang::En);
        assert_eq!(Lang::from_locale("italian"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
    }

    #[test]
    fn from_setting_values() {
        assert_eq!(Lang::from_setting("auto"), None);
        assert_eq!(Lang::from_setting(""), None);
        assert_eq!(Lang::from_setting("  AUTO "), None);
        assert_eq!(Lang::from_setting("en"), Some(Lang::En));
        assert_eq!(Lang::from_setting(" It "), Some(Lang::It));
        assert_eq!(Lang::from_setting("de"), None);
    }
}
