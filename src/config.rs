//! `Nitor.ini` model, parsing, serialization and location (spec §10).
//! Parsing is lenient: unknown keys are ignored, invalid values fall back to defaults.

use crate::color::Rgb;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrayStyle {
    #[default]
    Text,
    Pill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct General {
    pub tray_style: TrayStyle,
    /// Wheel step in percent; one of 1, 2, 5, 10 (other values are clamped to 1..=25).
    pub wheel_step: u8,
    pub single_icon_mode: bool,
    pub refresh_on_open: bool,
    /// Flyout "Link all" toggle.
    pub linked: bool,
    pub osd: bool,
    pub write_interval_ms: u32,
    pub rescan_delay_ms: u32,
    pub autostart: bool,
    /// Also control the laptop's built-in display (through WMI).
    pub internal_display: bool,
    /// `auto`, `en`, `it`.
    pub language: String,
    /// Set once the first-run pinning hint was shown.
    pub hint_shown: bool,
    /// Check GitHub Releases for a newer version (at startup, then every 24 h).
    pub check_updates: bool,
    /// Unix seconds of the last automatic update check.
    pub last_update_check: u64,
    /// Awake minutes Nitor has been running in total (see `support`).
    pub usage_minutes: u64,
    /// Show the occasional, dismissable support reminder.
    pub support_reminders: bool,
    /// `usage_minutes` at which the reminder is next due.
    pub support_next_minutes: u64,
    /// Stable IDs being probed; non-empty at startup means the last probe crashed (§7.4).
    pub detection_in_progress: Vec<String>,
}

impl Default for General {
    fn default() -> Self {
        General {
            tray_style: TrayStyle::Text,
            wheel_step: 5,
            single_icon_mode: false,
            refresh_on_open: true,
            linked: false,
            osd: false,
            write_interval_ms: 80,
            rescan_delay_ms: 2000,
            autostart: false,
            internal_display: false,
            language: "auto".into(),
            hint_shown: false,
            check_updates: true,
            last_update_check: 0,
            usage_minutes: 0,
            support_reminders: true,
            support_next_minutes: crate::support::FIRST_MINUTES,
            detection_in_progress: Vec::new(),
        }
    }
}

/// Per-monitor settings, keyed by stable ID in `Config::monitors`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorSettings {
    pub alias: Option<String>,
    pub color: Option<Rgb>,
    pub order: Option<u32>,
    /// Per-monitor wheel step override.
    pub step: Option<u8>,
    pub min: u8,
    pub max: u8,
    /// Calibrated range the flyout's linked master slider maps onto (0..=100).
    pub link_min: u8,
    pub link_max: u8,
    pub badge: Option<char>,
    /// Hidden from the tray.
    pub hidden: bool,
    /// Plus: how its color temperature is set.
    pub color_method: ColorMethodPref,
    /// Plus: the monitor's own red/green/blue video gains, saved the first time Hardware color
    /// was used (`ddc_gains`), and the maxima they were read with (`ddc_gains_max`).
    pub ddc_gains: Option<[u32; 3]>,
    pub ddc_gains_max: Option<[u32; 3]>,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        MonitorSettings {
            alias: None,
            color: None,
            order: None,
            step: None,
            min: 0,
            max: 100,
            link_min: 0,
            link_max: 100,
            badge: None,
            hidden: false,
            color_method: ColorMethodPref::Gamma,
            ddc_gains: None,
            ddc_gains_max: None,
        }
    }
}

/// Where the Plus automation gets the location from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LocationSource {
    #[default]
    Manual,
    Windows,
}

/// How long a manual change pauses automation for that monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OverrideMode {
    #[default]
    UntilTransition,
    Minutes,
}

/// How a monitor gets its color temperature (Nitor Plus; `[monitor.<id>] color_method`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorMethodPref {
    /// Software: a gamma ramp, which always works and never touches the monitor's memory.
    #[default]
    Gamma,
    /// RGB gains over DDC/CI, if the monitor really applies them.
    Ddc,
}

impl ColorMethodPref {
    fn parse(v: &str) -> ColorMethodPref {
        match v.trim().to_ascii_lowercase().as_str() {
            "ddc" | "hardware" => ColorMethodPref::Ddc,
            // "gamma", "software", the retired "auto" and anything else.
            _ => ColorMethodPref::Gamma,
        }
    }
}

/// `[auto]` section: Nitor Plus automation. Parsed always, only acted on in Plus builds.
#[derive(Clone, Debug, PartialEq)]
pub struct Auto {
    pub enabled: bool,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub location_source: LocationSource,
    pub day_brightness: u8,
    pub night_brightness: u8,
    pub use_sensor: bool,
    pub weather: bool,
    pub color_enabled: bool,
    pub day_kelvin: u32,
    pub night_kelvin: u32,
    /// Allow colour temperatures below 4000 K (needs the GdiIcmGammaRange registry value).
    pub experimental_warm: bool,
    /// Fixed temperature set from the flyout while automatic color is off; `None`: neutral.
    pub manual_kelvin: Option<u16>,
    pub override_mode: OverrideMode,
    pub override_minutes: u32,
}

// Coordinates are always finite (rejected otherwise when parsing), so equality is total.
impl Eq for Auto {}

impl Default for Auto {
    fn default() -> Self {
        Auto {
            enabled: false,
            latitude: None,
            longitude: None,
            location_source: LocationSource::Manual,
            day_brightness: 80,
            night_brightness: 30,
            use_sensor: true,
            weather: false,
            color_enabled: false,
            day_kelvin: 6500,
            night_kelvin: 4000,
            experimental_warm: false,
            manual_kelvin: None,
            override_mode: OverrideMode::UntilTransition,
            override_minutes: 60,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub general: General,
    pub auto: Auto,
    pub monitors: BTreeMap<String, MonitorSettings>,
    /// `(hotkey, action)` pairs in file order, unparsed.
    pub hotkeys: Vec<(String, String)>,
    /// `(HH:MM, action)` pairs in file order, unparsed.
    pub schedule: Vec<(String, String)>,
}

fn parse_bool(v: &str, default: bool) -> bool {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => true,
        "false" | "0" | "no" => false,
        _ => default,
    }
}

fn parse_num<T: std::str::FromStr>(v: &str) -> Option<T> {
    v.parse().ok()
}

/// Strip a trailing `;` comment (one preceded by whitespace) and trim.
fn strip_inline_comment(v: &str) -> &str {
    let b = v.as_bytes();
    for i in 0..b.len() {
        if b[i] == b';' && (i == 0 || b[i - 1].is_ascii_whitespace()) {
            return v[..i].trim();
        }
    }
    v.trim()
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

enum Section {
    None,
    General,
    Auto,
    Monitor(String),
    Hotkeys,
    Schedule,
}

impl Config {
    /// Parse INI text (sections `[general]`, `[auto]`, `[monitor.<id>]`, `[hotkeys]`, `[schedule]`;
    /// `;` and `#` comments, also trailing inline `;` comments).
    pub fn parse(text: &str) -> Config {
        let mut cfg = Config::default();
        let mut section = Section::None;
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                let name = rest.rfind(']').map_or(rest, |e| &rest[..e]).trim();
                section = if name.eq_ignore_ascii_case("general") {
                    Section::General
                } else if name.eq_ignore_ascii_case("auto") {
                    Section::Auto
                } else if name.eq_ignore_ascii_case("hotkeys") {
                    Section::Hotkeys
                } else if name.eq_ignore_ascii_case("schedule") {
                    Section::Schedule
                } else if let Some(id) = name.strip_prefix("monitor.").filter(|id| !id.is_empty()) {
                    cfg.monitors.entry(id.to_string()).or_default();
                    Section::Monitor(id.to_string())
                } else {
                    Section::None
                };
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), strip_inline_comment(v));
            match &section {
                Section::None => {}
                Section::General => cfg.general.set(k, v),
                Section::Auto => cfg.auto.set(k, v),
                Section::Monitor(id) => {
                    if let Some(m) = cfg.monitors.get_mut(id) {
                        m.set(k, v);
                    }
                }
                Section::Hotkeys if !k.is_empty() && !v.is_empty() => {
                    cfg.hotkeys.push((k.into(), v.into()))
                }
                Section::Schedule if !k.is_empty() && !v.is_empty() => {
                    cfg.schedule.push((k.into(), v.into()))
                }
                _ => {}
            }
        }
        cfg
    }

    /// Serialize to INI text that `parse` reads back to an equal `Config`.
    /// Default/None values may be omitted.
    pub fn to_ini(&self) -> String {
        let g = &self.general;
        let style = match g.tray_style {
            TrayStyle::Text => "text",
            TrayStyle::Pill => "pill",
        };
        let mut o = String::from("[general]\n");
        o += &format!("tray_style = {style}          ; text | pill\n");
        o += &format!("wheel_step = {}\n", g.wheel_step);
        o += &format!("single_icon_mode = {}\n", g.single_icon_mode);
        o += &format!("refresh_on_open = {}\n", g.refresh_on_open);
        o += &format!("linked = {}\n", g.linked);
        o += &format!("osd = {}\n", g.osd);
        o += &format!("write_interval_ms = {}\n", g.write_interval_ms);
        o += &format!("rescan_delay_ms = {}\n", g.rescan_delay_ms);
        o += &format!("autostart = {}\n", g.autostart);
        o += &format!("internal_display = {}\n", g.internal_display);
        o += &format!(
            "language = {}          ; auto | en | it\n",
            one_line(&g.language)
        );
        o += &format!("hint_shown = {}\n", g.hint_shown);
        o += &format!("check_updates = {}\n", g.check_updates);
        o += &format!("last_update_check = {}\n", g.last_update_check);
        o += &format!("usage_minutes = {}\n", g.usage_minutes);
        o += &format!("support_reminders = {}\n", g.support_reminders);
        o += &format!("support_next_minutes = {}\n", g.support_next_minutes);
        o += &format!(
            "detection_in_progress = {}\n",
            one_line(&g.detection_in_progress.join(","))
        );
        if self.auto != Auto::default() {
            o += &self.auto.to_ini();
        }
        for (id, m) in &self.monitors {
            o += &format!("\n[monitor.{}]\n", one_line(id));
            if let Some(a) = &m.alias {
                o += &format!("alias = {}\n", one_line(a));
            }
            if let Some(c) = m.color {
                o += &format!("color = {}\n", c.to_hex());
            }
            if let Some(v) = m.order {
                o += &format!("order = {v}\n");
            }
            if let Some(v) = m.step {
                o += &format!("step = {v}\n");
            }
            if m.min != 0 {
                o += &format!("min = {}\n", m.min);
            }
            if m.max != 100 {
                o += &format!("max = {}\n", m.max);
            }
            if m.link_min != 0 {
                o += &format!("link_min = {}\n", m.link_min);
            }
            if m.link_max != 100 {
                o += &format!("link_max = {}\n", m.link_max);
            }
            if let Some(b) = m.badge {
                o += &format!("badge = {}\n", one_line(&b.to_string()));
            }
            if m.hidden {
                o += "hidden = true\n";
            }
            match m.color_method {
                ColorMethodPref::Ddc => o += "color_method = ddc          ; gamma | ddc\n",
                ColorMethodPref::Gamma => o += "color_method = gamma        ; gamma | ddc\n",
            }
            for (key, gains) in [
                ("ddc_gains", m.ddc_gains),
                ("ddc_gains_max", m.ddc_gains_max),
            ] {
                if let Some([r, g, b]) = gains {
                    o += &format!("{key} = {r},{g},{b}\n");
                }
            }
        }
        o += "\n[hotkeys]\n";
        for (k, v) in &self.hotkeys {
            o += &format!("{} = {}\n", one_line(k), one_line(v));
        }
        o += "\n[schedule]\n";
        for (k, v) in &self.schedule {
            o += &format!("{} = {}\n", one_line(k), one_line(v));
        }
        o
    }

    /// Load from disk; a missing file yields the default config.
    pub fn load(path: &Path) -> std::io::Result<Config> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Config::parse(&String::from_utf8_lossy(&bytes))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e),
        }
    }

    /// Atomic save: write `<path>.tmp`, then rename over `path`. Creates the parent directory.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, self.to_ini())?;
        std::fs::rename(&tmp, path)
    }
}

impl General {
    fn set(&mut self, k: &str, v: &str) {
        let d = General::default();
        match k.to_ascii_lowercase().as_str() {
            "tray_style" => {
                self.tray_style = match v.to_ascii_lowercase().as_str() {
                    "pill" => TrayStyle::Pill,
                    _ => TrayStyle::Text,
                }
            }
            "wheel_step" => {
                self.wheel_step = parse_num::<u8>(v).unwrap_or(d.wheel_step).clamp(1, 25)
            }
            "single_icon_mode" => self.single_icon_mode = parse_bool(v, d.single_icon_mode),
            "refresh_on_open" => self.refresh_on_open = parse_bool(v, d.refresh_on_open),
            "linked" => self.linked = parse_bool(v, d.linked),
            "osd" => self.osd = parse_bool(v, d.osd),
            "write_interval_ms" => {
                self.write_interval_ms = parse_num::<u32>(v)
                    .unwrap_or(d.write_interval_ms)
                    .clamp(20, 2000)
            }
            "rescan_delay_ms" => self.rescan_delay_ms = parse_num(v).unwrap_or(d.rescan_delay_ms),
            "autostart" => self.autostart = parse_bool(v, d.autostart),
            "internal_display" => self.internal_display = parse_bool(v, d.internal_display),
            "language" => {
                self.language = if v.is_empty() {
                    d.language
                } else {
                    v.to_string()
                }
            }
            "hint_shown" => self.hint_shown = parse_bool(v, d.hint_shown),
            "check_updates" => self.check_updates = parse_bool(v, d.check_updates),
            "last_update_check" => self.last_update_check = parse_num(v).unwrap_or(0),
            "usage_minutes" => self.usage_minutes = parse_num(v).unwrap_or(0),
            "support_reminders" => self.support_reminders = parse_bool(v, d.support_reminders),
            "support_next_minutes" => {
                self.support_next_minutes = parse_num(v).unwrap_or(d.support_next_minutes)
            }
            "detection_in_progress" => {
                self.detection_in_progress = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            }
            _ => {}
        }
    }
}

fn parse_coord(v: &str, limit: f64) -> Option<f64> {
    parse_num::<f64>(v)
        .filter(|x| x.is_finite())
        .map(|x| x.clamp(-limit, limit))
}

impl Auto {
    fn set(&mut self, k: &str, v: &str) {
        let d = Auto::default();
        match k.to_ascii_lowercase().as_str() {
            "enabled" => self.enabled = parse_bool(v, d.enabled),
            "latitude" => self.latitude = parse_coord(v, 90.0),
            "longitude" => self.longitude = parse_coord(v, 180.0),
            "location_source" => {
                self.location_source = match v.to_ascii_lowercase().as_str() {
                    "windows" => LocationSource::Windows,
                    _ => LocationSource::Manual,
                }
            }
            "day_brightness" => {
                self.day_brightness = parse_num::<u8>(v).unwrap_or(d.day_brightness).min(100)
            }
            "night_brightness" => {
                self.night_brightness = parse_num::<u8>(v).unwrap_or(d.night_brightness).min(100)
            }
            "use_sensor" => self.use_sensor = parse_bool(v, d.use_sensor),
            "weather" => self.weather = parse_bool(v, d.weather),
            "color_enabled" => self.color_enabled = parse_bool(v, d.color_enabled),
            "day_kelvin" => {
                self.day_kelvin = parse_num::<u32>(v)
                    .unwrap_or(d.day_kelvin)
                    .clamp(1000, 6500)
            }
            "night_kelvin" => {
                self.night_kelvin = parse_num::<u32>(v)
                    .unwrap_or(d.night_kelvin)
                    .clamp(1000, 6500)
            }
            "experimental_warm" => self.experimental_warm = parse_bool(v, d.experimental_warm),
            "manual_kelvin" => {
                self.manual_kelvin = parse_num::<u16>(v).map(|k| k.clamp(1000, 6500))
            }
            "override_mode" => {
                self.override_mode = match v.to_ascii_lowercase().as_str() {
                    "minutes" => OverrideMode::Minutes,
                    _ => OverrideMode::UntilTransition,
                }
            }
            "override_minutes" => {
                self.override_minutes = parse_num::<u32>(v)
                    .unwrap_or(d.override_minutes)
                    .clamp(1, 1440)
            }
            _ => {}
        }
    }

    fn to_ini(&self) -> String {
        let mut o = String::from("\n[auto]\n");
        o += &format!("enabled = {}\n", self.enabled);
        if let Some(v) = self.latitude {
            o += &format!("latitude = {v}\n");
        }
        if let Some(v) = self.longitude {
            o += &format!("longitude = {v}\n");
        }
        let src = match self.location_source {
            LocationSource::Manual => "manual",
            LocationSource::Windows => "windows",
        };
        o += &format!("location_source = {src}          ; manual | windows\n");
        o += &format!("day_brightness = {}\n", self.day_brightness);
        o += &format!("night_brightness = {}\n", self.night_brightness);
        o += &format!("use_sensor = {}\n", self.use_sensor);
        o += &format!("weather = {}\n", self.weather);
        o += &format!("color_enabled = {}\n", self.color_enabled);
        o += &format!("day_kelvin = {}\n", self.day_kelvin);
        o += &format!("night_kelvin = {}\n", self.night_kelvin);
        o += &format!("experimental_warm = {}\n", self.experimental_warm);
        if let Some(k) = self.manual_kelvin {
            o += &format!("manual_kelvin = {k}\n");
        }
        let mode = match self.override_mode {
            OverrideMode::UntilTransition => "until_transition",
            OverrideMode::Minutes => "minutes",
        };
        o += &format!("override_mode = {mode}          ; until_transition | minutes\n");
        o += &format!("override_minutes = {}\n", self.override_minutes);
        o
    }
}

impl MonitorSettings {
    fn set(&mut self, k: &str, v: &str) {
        match k.to_ascii_lowercase().as_str() {
            "alias" => self.alias = Some(v.to_string()).filter(|s| !s.is_empty()),
            "color" => self.color = Rgb::from_hex(v),
            "order" => self.order = parse_num(v),
            "step" => self.step = parse_num::<u8>(v).map(|s| s.clamp(1, 25)),
            "min" => self.min = parse_num::<u8>(v).unwrap_or(0).min(100),
            "max" => self.max = parse_num::<u8>(v).unwrap_or(100).min(100),
            "link_min" => self.link_min = parse_num::<u8>(v).unwrap_or(0).min(100),
            "link_max" => self.link_max = parse_num::<u8>(v).unwrap_or(100).min(100),
            "badge" => self.badge = v.chars().next(),
            "hidden" => self.hidden = parse_bool(v, false),
            "color_method" => self.color_method = ColorMethodPref::parse(v),
            "ddc_gains" => self.ddc_gains = parse_gains(v),
            "ddc_gains_max" => self.ddc_gains_max = parse_gains(v),
            _ => return,
        }
        if self.min > self.max {
            std::mem::swap(&mut self.min, &mut self.max);
        }
        if self.link_min > self.link_max {
            std::mem::swap(&mut self.link_min, &mut self.link_max);
        }
    }
}

/// Three video-gain values, `R,G,B` (spaces allowed); `None` unless exactly three numbers.
fn parse_gains(v: &str) -> Option<[u32; 3]> {
    let mut it = v.split(',').map(|p| p.trim().parse::<u32>().ok());
    let gains = [it.next()??, it.next()??, it.next()??];
    it.next().is_none().then_some(gains)
}

/// Resolve the config path: explicit CLI path; else an existing `Nitor.ini` in `exe_dir`;
/// else an existing `<appdata>/Nitor/Nitor.ini` (so one failed writability probe cannot
/// split the config); else `Nitor.ini` in `exe_dir` if that directory is writable (probe
/// by creating and deleting a temp file); else `<appdata>/Nitor/Nitor.ini`; else
/// `exe_dir/Nitor.ini`.
pub fn resolve_path(cli: Option<&Path>, exe_dir: &Path, appdata: Option<&Path>) -> PathBuf {
    if let Some(p) = cli {
        return p.to_path_buf();
    }
    let in_exe = exe_dir.join("Nitor.ini");
    let in_appdata = appdata.map(|a| a.join("Nitor").join("Nitor.ini"));
    if in_exe.is_file() {
        return in_exe;
    }
    if let Some(p) = in_appdata.as_ref().filter(|p| p.is_file()) {
        return p.clone();
    }
    let probe = exe_dir.join(format!(".nitor_probe_{}", std::process::id()));
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return in_exe;
    }
    in_appdata.unwrap_or(in_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nitor_cfg_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> Config {
        let mut c = Config::default();
        c.general.tray_style = TrayStyle::Pill;
        c.general.wheel_step = 10;
        c.general.osd = true;
        c.general.internal_display = true;
        c.general.linked = true;
        c.general.refresh_on_open = false;
        c.general.write_interval_ms = 150;
        c.general.language = "it".into();
        c.general.check_updates = false;
        c.general.last_update_check = 1_700_000_000;
        c.general.usage_minutes = 1234;
        c.general.support_reminders = false;
        c.general.support_next_minutes = 6600;
        c.general.detection_in_progress = vec!["DEL41D9-ABC#1".into(), "A&B-2".into()];
        c.auto = Auto {
            enabled: true,
            latitude: Some(41.9028),
            longitude: Some(-12.5),
            location_source: LocationSource::Windows,
            day_brightness: 90,
            night_brightness: 10,
            use_sensor: false,
            weather: true,
            color_enabled: true,
            day_kelvin: 6000,
            night_kelvin: 3200,
            experimental_warm: true,
            manual_kelvin: Some(4200),
            override_mode: OverrideMode::Minutes,
            override_minutes: 30,
        };
        c.monitors.insert(
            "DEL41D9-ABC123".into(),
            MonitorSettings {
                alias: Some("Dell".into()),
                color: Some(Rgb::new(0x37, 0x8A, 0xDD)),
                order: Some(1),
                step: Some(2),
                min: 10,
                max: 90,
                link_min: 20,
                link_max: 80,
                badge: Some('D'),
                hidden: true,
                color_method: ColorMethodPref::Ddc,
                ddc_gains: Some([100, 92, 88]),
                ddc_gains_max: Some([100, 100, 100]),
            },
        );
        c.monitors
            .insert("X#&-1".into(), MonitorSettings::default());
        c.hotkeys = vec![
            ("Ctrl+Alt+PgUp".into(), "all +10".into()),
            ("Ctrl+Alt+PgDn".into(), "all -10".into()),
        ];
        c.schedule = vec![
            ("07:00".into(), "all b 40".into()),
            ("22:30".into(), "all b 10".into()),
        ];
        c
    }

    #[test]
    fn roundtrip() {
        let c = sample();
        assert_eq!(Config::parse(&c.to_ini()), c);
        assert_eq!(
            Config::parse(&Config::default().to_ini()),
            Config::default()
        );
    }

    #[test]
    fn parses_spec_example_with_comments() {
        let c = Config::parse(
            "; top\n[general]\ntray_style = pill ; text | pill\nwheel_step = 99\nosd = yes\n\
             write_interval_ms = 5\nautostart = maybe\nhint_shown = 1\nusage_minutes = x\nsupport_reminders = maybe\nsupport_next_minutes = 77\ndetection_in_progress = a, b#c ,\n\
             [monitor.DEL41D9-ABC123]\nalias = Dell\ncolor = #378ADD ; blue\nmin = 80\nmax = 20\nbadge = D\n\
             [hotkeys]\nCtrl+Alt+PgUp = all +10\n[schedule]\n07:00 = all b 40 ; morning\n",
        );
        assert_eq!(c.general.tray_style, TrayStyle::Pill);
        assert_eq!(c.general.wheel_step, 25);
        assert!(c.general.osd);
        assert!(!c.general.autostart);
        assert!(c.general.hint_shown);
        assert_eq!(c.general.usage_minutes, 0);
        assert!(c.general.support_reminders);
        assert_eq!(c.general.support_next_minutes, 77);
        assert_eq!(c.general.write_interval_ms, 20);
        assert_eq!(c.general.detection_in_progress, vec!["a", "b#c"]);
        let m = &c.monitors["DEL41D9-ABC123"];
        assert_eq!(m.color, Some(Rgb::new(0x37, 0x8A, 0xDD)));
        assert_eq!((m.min, m.max), (20, 80));
        assert_eq!(m.badge, Some('D'));
        assert_eq!(
            c.hotkeys,
            vec![("Ctrl+Alt+PgUp".to_string(), "all +10".to_string())]
        );
        assert_eq!(
            c.schedule,
            vec![("07:00".to_string(), "all b 40".to_string())]
        );
    }

    #[test]
    fn auto_section_only_written_when_non_default() {
        assert!(!Config::default().to_ini().contains("[auto]"));
        assert!(sample().to_ini().contains("[auto]"));
    }

    #[test]
    fn auto_parsing_is_lenient() {
        let c = Config::parse(
            "[auto]\nenabled = yes\nlatitude = 95\nlongitude = abc\nlocation_source = windows\n\
             day_brightness = 300\nday_kelvin = 100\nnight_kelvin = 99999\nmanual_kelvin = 10\noverride_mode = Minutes\n\
             override_minutes = 0\nunknown = 1\n",
        );
        let a = &c.auto;
        assert!(a.enabled);
        assert_eq!(a.latitude, Some(90.0));
        assert_eq!(a.longitude, None);
        assert_eq!(a.location_source, LocationSource::Windows);
        assert_eq!(a.day_brightness, 80);
        assert_eq!((a.day_kelvin, a.night_kelvin), (1000, 6500));
        assert_eq!(a.manual_kelvin, Some(1000));
        assert_eq!(
            Config::parse("[auto]\nmanual_kelvin = x\n")
                .auto
                .manual_kelvin,
            None
        );
        assert_eq!(a.override_mode, OverrideMode::Minutes);
        assert_eq!(a.override_minutes, 1);
        assert_eq!(
            Config::parse("[auto]\nlatitude = NaN\n").auto.latitude,
            None
        );
    }

    #[test]
    fn invalid_values_use_defaults() {
        let c = Config::parse(
            "[general]\nwheel_step = abc\ntray_style = ???\n[monitor.M]\ncolor = nope\nmax = 500\n",
        );
        assert_eq!(c.general.wheel_step, 5);
        assert_eq!(c.general.tray_style, TrayStyle::Text);
        assert_eq!(c.monitors["M"].color, None);
        assert_eq!(c.monitors["M"].max, 100);
        assert_eq!(c.monitors["M"].color_method, ColorMethodPref::Gamma);
        for (text, want) in [
            ("DDC", ColorMethodPref::Ddc),
            ("hardware", ColorMethodPref::Ddc),
            (" Gamma ", ColorMethodPref::Gamma),
            ("software", ColorMethodPref::Gamma),
            ("auto", ColorMethodPref::Gamma),
            ("???", ColorMethodPref::Gamma),
        ] {
            let c = Config::parse(&format!("[monitor.M]\ncolor_method = {text}\n"));
            assert_eq!(c.monitors["M"].color_method, want, "{text}");
        }
    }

    #[test]
    fn gains_parse_leniently() {
        let gains = |v: &str| {
            Config::parse(&format!("[monitor.M]\nddc_gains = {v}\n")).monitors["M"].ddc_gains
        };
        assert_eq!(gains("100,92,88"), Some([100, 92, 88]));
        assert_eq!(gains(" 100 , 92 ,88 ; saved"), Some([100, 92, 88]));
        for bad in ["", "1,2", "1,2,3,4", "a,b,c", "1,2,-3", "100 92 88"] {
            assert_eq!(gains(bad), None, "{bad}");
        }
        let c = Config::parse("[monitor.M]\nddc_gains_max = 255,255,255\n");
        assert_eq!(c.monitors["M"].ddc_gains_max, Some([255; 3]));
        let mut c = Config::default();
        c.monitors.insert("M".into(), MonitorSettings::default());
        assert!(!c.to_ini().contains("ddc_gains"));
    }

    #[test]
    fn load_save() {
        let d = tmp_dir("save");
        let p = d.join("sub").join("Nitor.ini");
        assert_eq!(Config::load(&p).unwrap(), Config::default());
        let c = sample();
        c.save(&p).unwrap();
        assert!(!d.join("sub").join("Nitor.ini.tmp").exists());
        assert_eq!(Config::load(&p).unwrap(), c);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve() {
        let d = tmp_dir("resolve");
        let cli = d.join("my.ini");
        assert_eq!(resolve_path(Some(&cli), &d, None), cli);
        assert_eq!(resolve_path(None, &d, None), d.join("Nitor.ini"));
        let missing = d.join("nope");
        let ad = d.join("appdata");
        assert_eq!(
            resolve_path(None, &missing, Some(&ad)),
            ad.join("Nitor").join("Nitor.ini")
        );
        assert_eq!(
            resolve_path(None, &missing, None),
            missing.join("Nitor.ini")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_prefers_existing_file() {
        let d = tmp_dir("resolve_existing");
        let (exe, ad) = (d.join("exe"), d.join("appdata"));
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(ad.join("Nitor")).unwrap();
        let in_ad = ad.join("Nitor").join("Nitor.ini");
        // Only in appdata: wins although the exe folder is writable.
        std::fs::write(&in_ad, "").unwrap();
        assert_eq!(resolve_path(None, &exe, Some(&ad)), in_ad);
        // Both exist: the exe folder wins.
        std::fs::write(exe.join("Nitor.ini"), "").unwrap();
        assert_eq!(resolve_path(None, &exe, Some(&ad)), exe.join("Nitor.ini"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
