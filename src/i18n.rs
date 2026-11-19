//! UI strings. Add a language by adding a `Lang` variant and one arm per key in `tr`.

#[cfg(feature = "plus")]
mod plus;
#[cfg(feature = "plus")]
pub use plus::PlusKey;

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

    /// The config value for this language.
    pub fn as_setting(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::It => "it",
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
    /// Plus: tray menu items for the global automation pause.
    MenuPauseAuto,
    MenuResumeAuto,
    /// Menu item to install a found update; `{}` is the version.
    MenuUpdateTo,
    /// Update balloons; `{}` is the version (or the error for `UpdateFailed`).
    UpdateAvailable,
    UpToDate,
    UpdateFailed,
    Updated,
    /// Tooltip suffix for a monitor that doesn't answer.
    Unresponsive,
    /// Tooltip suffix for the built-in display when it never answered.
    InternalNoResponse,
    /// Name of the laptop panel (Windows only reports "Generic PnP Monitor").
    BuiltInDisplay,
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
    /// Balloon when the autostart entry pointed to another existing exe; `{}` is the product.
    AutostartSwitched,
    /// Balloon after a crash during detection; monitors were skipped.
    SafeModeText,
    /// Balloon when a hotkey couldn't be registered; `{}` is replaced by the hotkey.
    HotkeyConflict,
    /// Flyout header.
    Brightness,
    LinkAll,
    /// Settings window.
    SettingsTitle,
    SecGeneral,
    SecTray,
    SecMonitors,
    SecAdvanced,
    LblLanguage,
    LangAuto,
    LblCheckUpdates,
    /// Version line; `{}` is the version.
    LblVersion,
    BtnCheckNow,
    BtnInstall,
    /// Settings status line of the update check; `{}` is the version (or the error for `UpdCheckFailed`).
    UpdChecking,
    UpdUpToDate,
    UpdAvailable,
    UpdCheckFailed,
    LblLogFile,
    LblTrayStyle,
    StyleNumber,
    StylePill,
    LblWheelStep,
    LblRefreshOnOpen,
    LblRefreshOnOpenHint,
    LblName,
    /// Hint under the name field; `{}` is the monitor's own name.
    HintAlias,
    LblBadge,
    LblShowInTray,
    LblLimits,
    LblWriteInterval,
    LblConfigFile,
    BtnOpen,
    SupportTitle,
    SupportCaption,
    SupportText,
    BtnSupport,
    BtnKofi,
    BtnNotNow,
    LblDontRemind,
    LblSupportReminders,
    StateHidden,
    StateNotControllable,
    StateBuiltInOff,
    /// Nitor Plus Settings strings.
    #[cfg(feature = "plus")]
    Plus(PlusKey),
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
    Key::MenuPauseAuto,
    Key::MenuResumeAuto,
    Key::MenuUpdateTo,
    Key::UpdateAvailable,
    Key::UpToDate,
    Key::UpdateFailed,
    Key::Updated,
    Key::Unresponsive,
    Key::InternalNoResponse,
    Key::BuiltInDisplay,
    Key::ReadOnly,
    Key::ReadOnlyHint,
    Key::NoMonitors,
    Key::FirstRunTitle,
    Key::FirstRunText,
    Key::AutostartFixed,
    Key::AutostartSwitched,
    Key::SafeModeText,
    Key::HotkeyConflict,
    Key::Brightness,
    Key::LinkAll,
    Key::SettingsTitle,
    Key::SecGeneral,
    Key::SecTray,
    Key::SecMonitors,
    Key::SecAdvanced,
    Key::LblLanguage,
    Key::LangAuto,
    Key::LblCheckUpdates,
    Key::LblVersion,
    Key::BtnCheckNow,
    Key::BtnInstall,
    Key::UpdChecking,
    Key::UpdUpToDate,
    Key::UpdAvailable,
    Key::UpdCheckFailed,
    Key::LblLogFile,
    Key::LblTrayStyle,
    Key::StyleNumber,
    Key::StylePill,
    Key::LblWheelStep,
    Key::LblRefreshOnOpen,
    Key::LblRefreshOnOpenHint,
    Key::LblName,
    Key::HintAlias,
    Key::LblBadge,
    Key::LblShowInTray,
    Key::LblLimits,
    Key::LblWriteInterval,
    Key::LblConfigFile,
    Key::BtnOpen,
    Key::SupportTitle,
    Key::SupportCaption,
    Key::SupportText,
    Key::BtnSupport,
    Key::BtnKofi,
    Key::BtnNotNow,
    Key::LblDontRemind,
    Key::LblSupportReminders,
    Key::StateHidden,
    Key::StateNotControllable,
    Key::StateBuiltInOff,
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
            Key::MenuPauseAuto => "Pause automation for 1 hour",
            Key::MenuResumeAuto => "Resume automation",
            Key::MenuUpdateTo => "Update to {}",
            Key::UpdateAvailable => "Nitor {} is available",
            Key::UpToDate => "You're up to date ({})",
            Key::UpdateFailed => "Update failed: {}",
            Key::Updated => "Updated to {}",
            Key::Unresponsive => "not responding",
            Key::InternalNoResponse => "the built-in display didn't respond",
            Key::BuiltInDisplay => "Built-in display",
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
            Key::AutostartSwitched => "\"Start with Windows\" now opens {}.",
            Key::SafeModeText => {
                "Nitor closed unexpectedly while detecting monitors last time, so those monitors were skipped this time. Use Rescan to try again."
            }
            Key::HotkeyConflict => "Couldn't register hotkey {}: it's used by another app",
            Key::Brightness => "Brightness",
            Key::LinkAll => "Link all",
            Key::SettingsTitle => "Nitor — Settings",
            Key::SecGeneral => "General",
            Key::SecTray => "Tray",
            Key::SecMonitors => "Monitors",
            Key::SecAdvanced => "Advanced",
            Key::LblLanguage => "Language",
            Key::LangAuto => "Auto",
            Key::LblCheckUpdates => "Check for updates automatically",
            Key::LblVersion => "Version {}",
            Key::BtnCheckNow => "Check now",
            Key::BtnInstall => "Install",
            Key::UpdChecking => "Checking\u{2026}",
            Key::UpdUpToDate => "Up to date (v{})",
            Key::UpdAvailable => "v{} available",
            Key::UpdCheckFailed => "Check failed: {}",
            Key::LblLogFile => "Log file",
            Key::LblTrayStyle => "Icon style",
            Key::StyleNumber => "Number",
            Key::StylePill => "Pill",
            Key::LblWheelStep => "Wheel step",
            Key::LblRefreshOnOpen => "Re-read brightness when opening the flyout",
            Key::LblRefreshOnOpenHint => "Picks up changes made with the monitor's own buttons",
            Key::LblName => "Name",
            Key::HintAlias => "Leave empty to use {}",
            Key::LblBadge => "Badge",
            Key::LblShowInTray => "Show in tray",
            Key::LblLimits => "Brightness limits",
            Key::LblWriteInterval => "DDC write interval",
            Key::LblConfigFile => "Configuration file",
            Key::BtnOpen => "Open",
            Key::SupportTitle => "Support me and this project",
            Key::SupportCaption => "Nitor is free and open source.",
            Key::SupportText => {
                "Nitor is free and open source. If it's useful to you, you can support it on Ko-fi."
            }
            Key::BtnSupport => "Support on Ko-fi",
            Key::BtnKofi => "Ko-fi",
            Key::BtnNotNow => "Not now",
            Key::LblDontRemind => "Don't remind me again",
            Key::LblSupportReminders => "Occasional support reminders",
            Key::StateHidden => "Hidden",
            Key::StateNotControllable => "Not controllable",
            Key::StateBuiltInOff => "Built-in display (control off)",
            #[cfg(feature = "plus")]
            Key::Plus(k) => plus::tr(Lang::En, k),
        },
        Lang::It => match key {
            Key::AppName => "Nitor",
            Key::MenuRescan => "Rileva monitor",
            Key::MenuSettings => "Impostazioni",
            Key::MenuStartWithWindows => "Avvia con Windows",
            Key::MenuInternalDisplay => "Controlla schermo integrato",
            Key::MenuExit => "Esci",
            Key::MenuCheckUpdates => "Controlla aggiornamenti",
            Key::MenuPauseAuto => "Sospendi automazione per 1 ora",
            Key::MenuResumeAuto => "Riprendi automazione",
            Key::MenuUpdateTo => "Aggiorna a {}",
            Key::UpdateAvailable => "È disponibile Nitor {}",
            Key::UpToDate => "Nitor è aggiornato ({})",
            Key::UpdateFailed => "Aggiornamento non riuscito: {}",
            Key::Updated => "Aggiornato a {}",
            Key::Unresponsive => "non risponde",
            Key::InternalNoResponse => "lo schermo integrato non ha risposto",
            Key::BuiltInDisplay => "Schermo integrato",
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
            Key::AutostartSwitched => "\"Avvia con Windows\" ora apre {}.",
            Key::SafeModeText => {
                "Nitor si è chiuso in modo imprevisto durante il rilevamento dei monitor l'ultima volta, quindi questa volta quei monitor sono stati saltati. Usa Rileva monitor per riprovare."
            }
            Key::HotkeyConflict => {
                "Impossibile registrare la scorciatoia {}: è usata da un'altra app"
            }
            Key::Brightness => "Luminosità",
            Key::LinkAll => "Collega tutti",
            Key::SettingsTitle => "Nitor — Impostazioni",
            Key::SecGeneral => "Generali",
            Key::SecTray => "Area di notifica",
            Key::SecMonitors => "Monitor",
            Key::SecAdvanced => "Avanzate",
            Key::LblLanguage => "Lingua",
            Key::LangAuto => "Auto",
            Key::LblCheckUpdates => "Controlla automaticamente gli aggiornamenti",
            Key::LblVersion => "Versione {}",
            Key::BtnCheckNow => "Controlla ora",
            Key::BtnInstall => "Installa",
            Key::UpdChecking => "Controllo in corso\u{2026}",
            Key::UpdUpToDate => "Aggiornato (v{})",
            Key::UpdAvailable => "v{} disponibile",
            Key::UpdCheckFailed => "Controllo non riuscito: {}",
            Key::LblLogFile => "File di log",
            Key::LblTrayStyle => "Stile dell'icona",
            Key::StyleNumber => "Numero",
            Key::StylePill => "Pillola",
            Key::LblWheelStep => "Passo della rotellina",
            Key::LblRefreshOnOpen => "Rileggi la luminosità all'apertura del pannello",
            Key::LblRefreshOnOpenHint => "Rileva le modifiche fatte con i tasti del monitor",
            Key::LblName => "Nome",
            Key::HintAlias => "Lascia vuoto per usare {}",
            Key::LblBadge => "Badge",
            Key::LblShowInTray => "Mostra nell'area di notifica",
            Key::LblLimits => "Limiti di luminosità",
            Key::LblWriteInterval => "Intervallo di scrittura DDC",
            Key::LblConfigFile => "File di configurazione",
            Key::BtnOpen => "Apri",
            Key::SupportTitle => "Supporta me e questo progetto",
            Key::SupportCaption => "Nitor è gratuito e open source.",
            Key::SupportText => {
                "Nitor è gratuito e open source. Se ti è utile, puoi supportarlo su Ko-fi."
            }
            Key::BtnSupport => "Supporta su Ko-fi",
            Key::BtnKofi => "Ko-fi",
            Key::BtnNotNow => "Non ora",
            Key::LblDontRemind => "Non ricordarmelo più",
            Key::LblSupportReminders => "Promemoria occasionali di supporto",
            Key::StateHidden => "Nascosto",
            Key::StateNotControllable => "Non controllabile",
            Key::StateBuiltInOff => "Schermo integrato (controllo disattivato)",
            #[cfg(feature = "plus")]
            Key::Plus(k) => plus::tr(Lang::It, k),
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
    #[cfg(feature = "plus")]
    fn plus_keys_translated_in_every_language() {
        for lang in [Lang::En, Lang::It] {
            for &key in plus::ALL {
                assert!(!plus::tr(lang, key).is_empty(), "{lang:?} {key:?} is empty");
            }
            assert!(plus::tr(lang, PlusKey::AutoOverrideMinutes).contains("{}"));
        }
    }

    #[test]
    fn templates_keep_placeholder() {
        let templates = [
            Key::HotkeyConflict,
            Key::AutostartSwitched,
            Key::MenuUpdateTo,
            Key::UpdateAvailable,
            Key::UpToDate,
            Key::UpdateFailed,
            Key::Updated,
            Key::LblVersion,
            Key::UpdUpToDate,
            Key::UpdAvailable,
            Key::UpdCheckFailed,
            Key::HintAlias,
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
