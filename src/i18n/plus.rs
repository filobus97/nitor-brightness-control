//! Strings of the Nitor Plus Settings (automation and location). Compiled only with `plus`.

use super::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlusKey {
    SecAutomation,
    SecLocation,
    AutoEnabled,
    AutoEnabledHint,
    AutoDay,
    AutoNight,
    /// Note under the two level sliders.
    AutoLevelsNote,
    AutoSensor,
    AutoAdaptiveTitle,
    AutoAdaptiveNote,
    AutoAdaptiveBtn,
    AutoOverride,
    AutoOverrideTransition,
    AutoOverrideHour,
    /// `{}` is the number of minutes.
    AutoOverrideMinutes,
    LocCurrent,
    LocNotSet,
    LocSource,
    LocManual,
    LocWindows,
    LocWindowsNote,
    LocLat,
    LocLon,
    LocSearch,
    LocSearchBtn,
    LocSearchNote,
    LocSearching,
    LocNoResults,
    LocNoResultsDigits,
    LocSearchFailed,
    LocMap,
    LocMapBtn,
    MapTitle,
    MapHint,
    MapUse,
    MapCancel,
}

/// All keys, for the translation check.
#[cfg(test)]
pub const ALL: &[PlusKey] = &[
    PlusKey::SecAutomation,
    PlusKey::SecLocation,
    PlusKey::AutoEnabled,
    PlusKey::AutoEnabledHint,
    PlusKey::AutoDay,
    PlusKey::AutoNight,
    PlusKey::AutoLevelsNote,
    PlusKey::AutoSensor,
    PlusKey::AutoAdaptiveTitle,
    PlusKey::AutoAdaptiveNote,
    PlusKey::AutoAdaptiveBtn,
    PlusKey::AutoOverride,
    PlusKey::AutoOverrideTransition,
    PlusKey::AutoOverrideHour,
    PlusKey::AutoOverrideMinutes,
    PlusKey::LocCurrent,
    PlusKey::LocNotSet,
    PlusKey::LocSource,
    PlusKey::LocManual,
    PlusKey::LocWindows,
    PlusKey::LocWindowsNote,
    PlusKey::LocLat,
    PlusKey::LocLon,
    PlusKey::LocSearch,
    PlusKey::LocSearchBtn,
    PlusKey::LocSearchNote,
    PlusKey::LocSearching,
    PlusKey::LocNoResults,
    PlusKey::LocNoResultsDigits,
    PlusKey::LocSearchFailed,
    PlusKey::LocMap,
    PlusKey::LocMapBtn,
    PlusKey::MapTitle,
    PlusKey::MapHint,
    PlusKey::MapUse,
    PlusKey::MapCancel,
];

pub fn tr(lang: Lang, key: PlusKey) -> &'static str {
    use PlusKey as K;
    match lang {
        Lang::En => match key {
            K::SecAutomation => "Automation",
            K::SecLocation => "Location",
            K::AutoEnabled => "Automatic brightness",
            K::AutoEnabledHint => "Follows sunrise and sunset at your location.",
            K::AutoDay => "Day level",
            K::AutoNight => "Night level",
            K::AutoLevelsNote => {
                "Each monitor follows these levels through its Link-all calibration range."
            }
            K::AutoSensor => "Use the light sensor",
            K::AutoAdaptiveTitle => "Windows adaptive brightness",
            K::AutoAdaptiveNote => {
                "Windows also adjusts the built-in display automatically; turn one off to avoid conflicts."
            }
            K::AutoAdaptiveBtn => "Display settings",
            K::AutoOverride => "After a manual change",
            K::AutoOverrideTransition => "Until next sunrise/sunset",
            K::AutoOverrideHour => "For 1 hour",
            K::AutoOverrideMinutes => "For {} min",
            K::LocCurrent => "Current location",
            K::LocNotSet => "Not set",
            K::LocSource => "Location source",
            K::LocManual => "Manual",
            K::LocWindows => "Windows location",
            K::LocWindowsNote => {
                "Windows may ask for permission to use your location. The position is stored rounded to about 1 km."
            }
            K::LocLat => "Latitude",
            K::LocLon => "Longitude",
            K::LocSearch => "Search postcode or city",
            K::LocSearchBtn => "Search",
            K::LocSearchNote => "Only the text you search is sent, to open-meteo.com.",
            K::LocSearching => "Searching…",
            K::LocNoResults => "No results",
            K::LocNoResultsDigits => "No results — try the city name",
            K::LocSearchFailed => "Search failed — check your connection",
            K::LocMap => "Choose on a world map",
            K::LocMapBtn => "Pick on map",
            K::MapTitle => "Pick your location",
            K::MapHint => "Click to mark · wheel to zoom · drag to move",
            K::MapUse => "Use this location",
            K::MapCancel => "Cancel",
        },
        Lang::It => match key {
            K::SecAutomation => "Automazione",
            K::SecLocation => "Posizione",
            K::AutoEnabled => "Luminosità automatica",
            K::AutoEnabledHint => "Segue alba e tramonto nella tua posizione.",
            K::AutoDay => "Livello diurno",
            K::AutoNight => "Livello notturno",
            K::AutoLevelsNote => {
                "Ogni monitor segue questi livelli nel proprio intervallo di calibrazione di Collega tutti."
            }
            K::AutoSensor => "Usa il sensore di luce",
            K::AutoAdaptiveTitle => "Luminosità adattiva di Windows",
            K::AutoAdaptiveNote => {
                "Windows regola automaticamente anche lo schermo integrato; disattiva uno dei due per evitare conflitti."
            }
            K::AutoAdaptiveBtn => "Impostazioni schermo",
            K::AutoOverride => "Dopo una modifica manuale",
            K::AutoOverrideTransition => "Fino ad alba/tramonto",
            K::AutoOverrideHour => "Per 1 ora",
            K::AutoOverrideMinutes => "Per {} min",
            K::LocCurrent => "Posizione attuale",
            K::LocNotSet => "Non impostata",
            K::LocSource => "Origine della posizione",
            K::LocManual => "Manuale",
            K::LocWindows => "Posizione di Windows",
            K::LocWindowsNote => {
                "Windows potrebbe chiedere il permesso di usare la posizione. La posizione viene salvata arrotondata a circa 1 km."
            }
            K::LocLat => "Latitudine",
            K::LocLon => "Longitudine",
            K::LocSearch => "Cerca CAP o città",
            K::LocSearchBtn => "Cerca",
            K::LocSearchNote => "Viene inviato solo il testo cercato, a open-meteo.com.",
            K::LocSearching => "Ricerca in corso…",
            K::LocNoResults => "Nessun risultato",
            K::LocNoResultsDigits => "Nessun risultato — prova il nome della città",
            K::LocSearchFailed => "Ricerca non riuscita — controlla la connessione",
            K::LocMap => "Scegli su una mappa del mondo",
            K::LocMapBtn => "Scegli sulla mappa",
            K::MapTitle => "Scegli la tua posizione",
            K::MapHint => "Clic per scegliere · rotellina: zoom · trascina: sposta",
            K::MapUse => "Usa questa posizione",
            K::MapCancel => "Annulla",
        },
    }
}
