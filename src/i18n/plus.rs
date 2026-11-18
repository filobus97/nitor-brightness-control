//! Strings of the Nitor Plus Settings (automation and location). Compiled only with `plus`.

use super::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlusKey {
    SecAutomation,
    SecLocation,
    SecColor,
    /// Caption of the flyout's color temperature slider.
    FlyoutColor,
    ColorEnabled,
    ColorEnabledHint,
    ColorDay,
    ColorNight,
    ColorWarm,
    ColorWarmNote,
    ColorWarmBtn,
    ColorNightLightNote,
    ColorNightLightBtn,
    ColorMethodHw,
    ColorMethodGamma,
    ColorMethodHdr,
    AutoEnabled,
    AutoEnabledHint,
    AutoDay,
    AutoNight,
    /// Note under the two level sliders.
    AutoLevelsNote,
    AutoSensor,
    AutoWeather,
    AutoWeatherHint,
    /// `{}` twice: the time (HH:MM) and the cloud cover in percent.
    AutoWeatherOk,
    /// `{}` is the error.
    AutoWeatherFailed,
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
    PlusKey::SecColor,
    PlusKey::FlyoutColor,
    PlusKey::ColorEnabled,
    PlusKey::ColorEnabledHint,
    PlusKey::ColorDay,
    PlusKey::ColorNight,
    PlusKey::ColorWarm,
    PlusKey::ColorWarmNote,
    PlusKey::ColorWarmBtn,
    PlusKey::ColorNightLightNote,
    PlusKey::ColorNightLightBtn,
    PlusKey::ColorMethodHw,
    PlusKey::ColorMethodGamma,
    PlusKey::ColorMethodHdr,
    PlusKey::AutoEnabled,
    PlusKey::AutoEnabledHint,
    PlusKey::AutoDay,
    PlusKey::AutoNight,
    PlusKey::AutoLevelsNote,
    PlusKey::AutoSensor,
    PlusKey::AutoWeather,
    PlusKey::AutoWeatherHint,
    PlusKey::AutoWeatherOk,
    PlusKey::AutoWeatherFailed,
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
            K::SecColor => "Color temperature (experimental)",
            K::FlyoutColor => "Color temperature",
            K::ColorEnabled => "Automatic color temperature",
            K::ColorEnabledHint => "Warmer light after sunset, like f.lux.",
            K::ColorDay => "Day temperature",
            K::ColorNight => "Night temperature",
            K::ColorWarm => "Allow warmer than 4000 K (experimental)",
            K::ColorWarmNote => {
                "Windows blocks gamma colors below 4000 K until a one-time registry change is made; you may need to sign out and in. Monitors set through their own RGB gains are not affected."
            }
            K::ColorWarmBtn => "Enable (needs administrator)",
            K::ColorNightLightNote => "Windows Night Light is on and also tints the screen.",
            K::ColorNightLightBtn => "Night light settings",
            K::ColorMethodHw => "Hardware (DDC)",
            K::ColorMethodGamma => "Gamma",
            K::ColorMethodHdr => "HDR: not supported",
            K::AutoEnabled => "Automatic brightness",
            K::AutoEnabledHint => "Follows sunrise and sunset at your location.",
            K::AutoDay => "Day level",
            K::AutoNight => "Night level",
            K::AutoLevelsNote => {
                "Each monitor follows these levels through its Link-all calibration range."
            }
            K::AutoSensor => "Use the light sensor",
            K::AutoWeather => "Adjust for cloudy weather (uses Open-Meteo)",
            K::AutoWeatherHint => {
                "Sends only your approximate location (about 10 km) every few hours"
            }
            K::AutoWeatherOk => "Last update: {}, cloud cover {}%",
            K::AutoWeatherFailed => "Weather update failed: {}",
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
            K::LocSearch => "Search city",
            K::LocSearchBtn => "Search",
            K::LocSearchNote => "Only the text you search is sent, to open-meteo.com.",
            K::LocSearching => "Searching…",
            K::LocNoResults => "No results",
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
            K::SecColor => "Temperatura colore (sperimentale)",
            K::FlyoutColor => "Temperatura colore",
            K::ColorEnabled => "Temperatura colore automatica",
            K::ColorEnabledHint => "Luce più calda dopo il tramonto, come f.lux.",
            K::ColorDay => "Temperatura diurna",
            K::ColorNight => "Temperatura notturna",
            K::ColorWarm => "Consenti sotto 4000 K (sperimentale)",
            K::ColorWarmNote => {
                "Windows blocca i colori gamma sotto 4000 K finché non si modifica il registro (una volta); potrebbe servire uscire e rientrare. I monitor regolati con i propri guadagni RGB non sono interessati."
            }
            K::ColorWarmBtn => "Attiva (richiede amministratore)",
            K::ColorNightLightNote => {
                "La Luce notturna di Windows è attiva e colora lo schermo a sua volta."
            }
            K::ColorNightLightBtn => "Impostazioni luce notturna",
            K::ColorMethodHw => "Hardware (DDC)",
            K::ColorMethodGamma => "Gamma",
            K::ColorMethodHdr => "HDR: non supportato",
            K::AutoEnabled => "Luminosità automatica",
            K::AutoEnabledHint => "Segue alba e tramonto nella tua posizione.",
            K::AutoDay => "Livello diurno",
            K::AutoNight => "Livello notturno",
            K::AutoLevelsNote => {
                "Ogni monitor segue questi livelli nel proprio intervallo di calibrazione di Collega tutti."
            }
            K::AutoSensor => "Usa il sensore di luce",
            K::AutoWeather => "Adatta al meteo nuvoloso (usa Open-Meteo)",
            K::AutoWeatherHint => {
                "Invia solo la posizione approssimativa (circa 10 km) ogni poche ore"
            }
            K::AutoWeatherOk => "Ultimo aggiornamento: {}, nuvolosità {}%",
            K::AutoWeatherFailed => "Aggiornamento meteo non riuscito: {}",
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
            K::LocSearch => "Cerca città",
            K::LocSearchBtn => "Cerca",
            K::LocSearchNote => "Viene inviato solo il testo cercato, a open-meteo.com.",
            K::LocSearching => "Ricerca in corso…",
            K::LocNoResults => "Nessun risultato",
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
