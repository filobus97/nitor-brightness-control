# Nitor: Portable DDC/CI Brightness Tray for Windows

| | |
|---|---|
| **Status** | v0.4 (open questions resolved, see §17) |
| **Date** | 2026-11-18 |
| **Name** | *Nitor* (Latin: "brightness, sheen") |
| **Platform** | Windows 10 (1809+) / Windows 11, x64 + ARM64 |
| **License** | GPL-3.0-or-later |

---

## 1. Summary

Nitor is a single portable `.exe` that lives in the Windows notification area. It shows **one tray icon per external monitor**. Each icon displays that monitor's current **hardware backlight brightness as a number**, in a **user-chosen color per monitor**. Scrolling the mouse wheel over an icon adjusts that monitor. Clicking an icon opens a Windows 11–style flyout with one slider per monitor.

It controls **brightness only**. It needs no installer, no admin rights, no runtime and no admin rights. It uses the network only to check GitHub Releases for updates (opt-out, §10) and never for anything else; no telemetry.

---

## 2. Background and decision

### 2.1 Why
Windows has no built-in brightness control for external monitors. The author has relied on **ClickMonitorDDC 7.2**, which is closed-source freeware (the original website is offline, there is no source to fork) and unmaintained (it officially targets Windows 7/8/10).

### 2.2 Alternatives evaluated

| Tool | Verdict | Reason |
|---|---|---|
| **PowerToys Power Display** (PowerToys 0.99+) | Excluded | A whole suite (bundled .NET runtime, many modules) for one capability. Single tray icon, no per-monitor values in the tray. |
| **Twinkle Tray** (MIT) | Design and behavior inspiration | Electron: large download, high RAM. Installer-only. Single tray icon. |
| **Monitorian** (MIT) | Excluded | Hotkeys and CLI are a paid add-on in the Store version. |
| **pratikpwr/monitor-brightness** | Excluded | Sets every monitor to the same level. |
| **poulicek/BrightnessControl** | Reference only | Tiny DDC tray app with scroll-over-icon. |

### 2.3 Gap that justifies Nitor
1. **Per-monitor tray icons showing the live value**, with a color per monitor.
2. **Mouse wheel over each icon** adjusting that specific monitor.
3. **True portability**: one small native exe, no install, no runtime.
4. **Minimal footprint**: about 15 MB RAM idle and zero CPU when idle.

---

## 3. Goals and non-goals

### Goals
- **G1**: Per-monitor tray icons rendering brightness (0–100) in a per-monitor color.
- **G2**: Mouse wheel over a tray icon adjusts that monitor.
- **G3**: A flyout that looks native on Windows 11 and acceptable on Windows 10.
- **G4**: Portable: one exe, runs from any folder, writes only its config file (and, during an update, the new exe next to itself).
- **G5**: Global hotkeys and a CLI with ClickMonitorDDC-flavored brightness syntax.
- **G6**: Safe DDC/CI behavior: no write floods, no risky probes, minimal EEPROM writes.

### Non-goals
- Volume, contrast, input switching, power state.
- Software dimming (overlays). Color temperature and gamma ramps exist only as the opt-in Nitor Plus feature of §16.
- An installer, the Microsoft Store, telemetry. Updates are never installed without a click (§10).
- Linux and macOS in this project (GNOME and KDE handle DDC brightness natively). The platform-independent logic lives outside `src/win/` so a future sibling project (e.g. `nitor-hyprland`) can reuse it.

---

## 4. User experience

### 4.1 Tray icons (P0)
- One icon per controllable monitor, in a user-configurable order.
- Each icon renders the brightness number in the monitor's color (see 4.2).
- An unresponsive monitor shows a dimmed "–".
- The tooltip shows the monitor's friendly name and value, e.g. `DELL U2723QE — 45%`.
- Optional **single-icon mode**: a sun glyph that controls all monitors.
- On first run, a one-time hint explains how to pin the icons out of the overflow area (Windows 11 hides new tray icons by default).

### 4.2 Per-monitor color (P0)

Each monitor has a color, used for: the tray number (or pill), a dot next to the monitor's name in the flyout, and that monitor's slider fill.

**Default palette** (assigned in order to new monitors):

| # | Name | Hex |
|---|---|---|
| 1 | Blue | `#378ADD` |
| 2 | Coral | `#D85A30` |
| 3 | Teal | `#1D9E75` |
| 4 | Purple | `#7F77DD` |
| 5 | Amber | `#BA7517` |
| 6 | Pink | `#D4537E` |
| 7 | Gray | `#888780` |

**Changing a color**: click the color dot in the flyout (inline swatch picker), or set a hex value in the config.

**Tray style** (global):
- **Colored number** (default): digits in the monitor's color on a transparent background.
- **Colored pill**: digits on a rounded rectangle in the monitor's color, with white or black text chosen for contrast.

**Contrast safeguard**
- Nitor reads the taskbar background: light or dark (`SystemUsesLightTheme`), or accent-colored (`ColorPrevalence` + DWM accent color).
- In colored-number style, if the color has less than 3:1 contrast against the taskbar, Nitor adjusts only its **OKLCH lightness** until it passes.
- The picked color is stored unchanged; only the rendered color is adjusted, and recomputed on theme/accent change.

**Not relying on color alone**: the tooltip always shows the name; an optional 1-character corner badge is available.

### 4.3 Interactions

| Action | Result |
|---|---|
| Wheel over tray icon | ±step on that monitor (default 5; options 1/2/5/10) |
| Shift + wheel | ±1 (fine adjust) |
| Left click | Toggle flyout |
| Middle click | Optional: toggle between the current value and a preset |
| Right click | Menu: Rescan monitors · Settings · Start with Windows ✓ · Exit |

The wheel step is fixed by Nitor's setting, independent of the Windows "lines per scroll" setting.

### 4.4 Flyout (P0)
- **Layout**: header ("Brightness"); per monitor a caption row (color dot + name) and a slider row (sun glyph, slider, numeric value); a footer band with "Link all", Rescan, Settings.
- **Position**: anchored to the clicked tray icon, respecting the taskbar edge and monitor.
- **Keyboard**: wheel, arrows, PgUp/PgDn, Home/End adjust the focused slider. Esc or clicking outside closes.
- **Updates**: UI updates instantly while dragging; DDC writes are throttled (§7.2).
- **Freshness**: on open, values are refreshed from the monitors (configurable).

### 4.5 Hotkeys (P1)
- Global hotkeys bind a **target** (alias, `all`, `cursor`) to an **action** (`+n`, `-n`, `set n`, `toggle a/b`).
- None bound by default.
- A hotkey that can't be registered produces a non-modal notification naming the conflict.

### 4.6 OSD (P1)
- When a hotkey or CLI changes brightness while the flyout is closed, a small toast confirms it in the monitor's color.
- Off by default. Suppressed when a fullscreen app is in the foreground (`SHQueryUserNotificationState`).

### 4.7 Linked brightness (P1)
- "Link all" adds a master slider (0–100) moving all monitors together.
- Per-monitor calibration range: the master value maps linearly into `link_min..=link_max`, so panels of different brightness can be matched (e.g. Dell 20–80, LG 0–100). While linked, each monitor's slider shows two handles to calibrate its range. The per-monitor `min`/`max` clamp still applies afterwards.

### 4.8 Schedule (P2)
- ClickMonitorDDC semantics: each rule fires once per day, e.g. `07:00 all b 30`.
- On startup or resume, the most recent rule whose time has passed is applied.
- Manual changes are never overridden until the next rule fires.

---

## 5. Visual design

### Windows 11
- Transient acrylic backdrop (`DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_TRANSIENTWINDOW`), rounded corners (`DWMWA_WINDOW_CORNER_PREFERENCE = ROUND`), 1 px subtle border.
- Segoe UI Variable, 14 px body / 12 px captions. Segoe Fluent Icons, sun glyph U+E706.
- Fluent slider: 4 px track, 20 px thumb with an inner dot, fill in the monitor's color.
- Row height about 48 px; flyout width 340 px at 100% scaling.

### Windows 10
- Square corners, solid background in the theme color, Segoe MDL2 Assets as glyph fallback.

### Theming
- Light/dark follows `AppsUseLightTheme`; tray icon contrast follows `SystemUsesLightTheme` and the taskbar accent setting.
- Re-theme on `WM_SETTINGCHANGE("ImmersiveColorSet")`.

### Tray icon rendering
- Drawn at runtime with Direct2D + DirectWrite at `GetSystemMetricsForDpi(SM_CXSMICON)`.
- Semibold condensed text with grid-fitting; "100" must stay legible at 16 px.
- Pill corner radius 2 px at 16 px, 3 px at 24 px and above.
- Re-rendered only when value, color, DPI or theme changes.

### Motion and DPI
- Short fade/slide on flyout open, disabled when `SPI_GETCLIENTAREAANIMATION` is off.
- Per-monitor DPI awareness v2, `WM_DPICHANGED` handled.

---

## 6. Architecture

**Rust + `windows-rs`** (raw Win32, Direct2D/DirectWrite, DWM). Static single exe, static CRT, no runtime dependency.

| Module | Role |
|---|---|
| `src/cli.rs`, `config.rs`, `color.rs`, `throttle.rs`, `hotkey.rs`, `i18n.rs` | Platform-independent logic, unit-tested on any OS |
| `src/win/app.rs` | Hidden message window (UI thread): owns tray icons, menu, hotkeys, system events |
| `src/win/monitors.rs` | Enumeration, stable identity, physical-monitor handles |
| `src/win/ddc.rs` | One worker thread per physical monitor with a coalescing command slot |
| `src/win/wheel.rs` | Temporary low-level mouse hook (§9) |
| `src/win/icon.rs`, `theme.rs` | Direct2D → HICON rendering; taskbar/app theme |
| `src/win/flyout.rs`, `settings.rs` | Direct2D flyout and the Settings window (one thread-local state each; actions go to `app.rs` through a callback) |
| `src/win/tray.rs` | `Shell_NotifyIcon` wrappers |
| `src/win/instance.rs` | Single instance + argument forwarding |
| `src/win/autostart.rs` | `HKCU\...\Run` entry |
| `src/auto/`, `src/win/auto/` | Nitor Plus only (Cargo feature `plus`, off by default): automation logic (portable) and its Windows side. Declared with `#[cfg(feature = "plus")]`; the plain build contains none of it |
| `src/startup.rs` | `--autostart` flag, Run-entry check, re-scan backoff schedule (portable) |
| `src/update.rs`, `src/win/update.rs` | Release lookup and self-update: version/JSON/checksum logic (portable) and WinHTTP + CNG + file swap |

**Editions.** One codebase, two builds: **Nitor** (brightness only) and **Nitor Plus** (`--features plus`: Nitor plus automation). The binary is `Nitor.exe` in both; CI publishes them as `Nitor-{x64,arm64}.exe` and `NitorPlus-{x64,arm64}.exe`. `update::edition()` names the edition and the updater picks the `{edition}-{arch}.exe` asset, so an install stays on its edition. CI runs clippy and tests with `--all-features`.

---

## 7. DDC/CI layer

### 7.1 Access
- dxva2: `GetPhysicalMonitorsFromHMONITOR`, `GetVCPFeatureAndVCPFeatureReply(0x10)`, `SetVCPFeature(0x10, v)`.
- Values are scaled to 0–100 using the maximum the monitor reports.
- **Never request the capabilities string** (kernel crash risk on some monitors). A successful read of VCP 0x10 is the only probe.

### 7.2 Write discipline
- The UI thread never calls DDC directly.
- Each worker has one "latest wanted value" slot; the latest value wins.
- At most one write every `write_interval_ms` (default 80) per monitor.
- After input stops, the final value is written within 150 ms.
- Never write a value equal to the last confirmed one (protects EEPROM).

### 7.3 Read policy
- No background polling. Values are read at startup, after a rescan, and when the flyout opens (configurable).

### 7.4 Errors
- A transient error gets one retry after 100 ms; then the monitor is "unresponsive" (icon shows "–") until the next rescan or successful read.
- Rejected writes (game/HDR presets) mark the monitor read-only, explained in the tooltip.
- **Crash-safe detection**: `detection_in_progress` is written to `Nitor.ini` before the first probe and cleared afterwards. If it's still set on the next start, the listed monitors are skipped for that run.

---

## 8. Monitor discovery and identity

**Built-in laptop panels** don't speak DDC/CI. They are detected via the DisplayConfig output technology (internal/embedded) and, only when `internal_display = true` (tray menu: "Control built-in display", shown only on laptops), controlled through WMI (`root\WMI`: `WmiMonitorBrightness`, `WmiSetBrightness`) by the same worker/throttle logic. Changes made with Fn keys or Quick Settings are picked up through the `GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS` power notification, without polling.

1. `EnumDisplayMonitors` → `MONITORINFOEX.szDevice`.
2. `QueryDisplayConfig` + `DisplayConfigGetDeviceInfo` → friendly name and device path.
3. **Stable ID**: EDID manufacturer + product code + serial (EDID from the monitor's registry key), device path as fallback.
4. Per-monitor settings are keyed by the stable ID.

- Cloned displays: several physical monitors behind one HMONITOR are handled separately.
- Rescan on `WM_DISPLAYCHANGE`, device arrival/removal, resume from sleep, display power-on (`GUID_CONSOLE_DISPLAY_STATE`), after `rescan_delay_ms` (default 2 s). Old handles are released with `DestroyPhysicalMonitors`.
- Direct API calls only; never shell out (no WMIC).

---

## 9. Mouse wheel over tray icons

The shell never forwards `WM_MOUSEWHEEL` to a tray icon's callback window.

1. Use `NOTIFYICON_VERSION_4`. On the icon's `WM_MOUSEMOVE`, install a `WH_MOUSE_LL` hook and cache icon rectangles from `Shell_NotifyIconGetRect`.
2. While armed, a 100 ms timer checks the cursor; when it leaves all Nitor icon rectangles, the hook is removed.
3. The hook callback only posts a message to the UI thread and returns immediately.
4. Wheel events inside a Nitor icon rectangle are consumed; everything else passes through.

---

## 10. Configuration and portability

- **Config location**: `--config <path>`; else an existing `Nitor.ini` next to the exe; else an existing `%APPDATA%\Nitor\Nitor.ini`; else (neither exists) next to the exe if that folder is writable; else `%APPDATA%\Nitor\Nitor.ini`.
- **Settings**: the "Settings" menu item and the flyout's gear open a graphical Settings window (`src/win/settings.rs`): a normal top-level window in the Windows 11 Settings style (Mica backdrop, grouped cards, light/dark, per-monitor DPI, scrolls when taller than the screen). Sections: General (start with Windows, language, built-in display, update checks), Tray (icon style, wheel step, refresh on open), Monitors (per monitor: color, name, badge, show in tray, brightness limits, order), Advanced (DDC write interval; the paths of `Nitor.ini` and `Nitor.log`, each with an Open button). Changes apply immediately and are saved atomically; there is no OK/Cancel. `Nitor.ini` stays the source of truth for power users and holds the options without a screen (hotkeys, schedule); Nitor reloads it on Rescan.
- **Tray icon identity**: by `uID`, never GUID (GUID icons are bound to the exe path, which breaks portability).
- **Autostart**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, written only when enabled; the value is `"<exe>" --autostart`; a stale path is fixed on launch and an older entry without the flag is upgraded silently. Started with `--autostart` (stripped before CLI parsing), the first scan waits 3 s (one-shot timer) because DDC/CI is often not ready at logon. After any probe, if a monitor with a DDC handle failed its first probe or none is controllable, re-scans follow after 5, 15, 30 and 60 s (one-shot timer, logged); they stop once every such monitor answers. Any other rescan (manual or system event) restarts the schedule.
- **Single instance**: named mutex; a second launch forwards its arguments via `WM_COPYDATA` and exits.
- **Explorer restarts**: icons re-added on `TaskbarCreated`.
- **Updates**: Nitor checks the latest GitHub release quietly in a background thread once per launch after the first probe (when `check_updates` is on), then at most every 24 h (an hourly timer tests whether 24 h passed, so sleep is handled). `[general] check_updates = false` turns automatic checks off; `last_update_check` (Unix seconds) records the last automatic check. When a newer version exists, a tray balloon announces it once per version and session and the tray menu gains "Update to <version>". Installing always needs that click. "Check for updates" in the menu checks on demand, also when `check_updates` is off, and shows the result in Settings > General (status line: Checking, Up to date, "vX available" with an Install button, or the error); if Settings is not open the result is a balloon without `NIIF_RESPECT_QUIET_TIME` (automatic notices keep respecting quiet time). While an update is known, "Update to <version>" is the menu's default (bold) item. Every check and install step is logged to `Nitor.log`.
  - **Network**: HTTPS to `api.github.com` and the release download host only, via WinHTTP, sending just `User-Agent: Nitor/<version>`. Prereleases (tags containing `-`) are ignored by `releases/latest`.
  - **Install**: download `Nitor-x64.exe` or `Nitor-arm64.exe` and `SHA256SUMS.txt` into `<exe>.new`, verify SHA-256 (Windows CNG), delete a stale `<exe>.old`, rename the running exe to `<exe>.old`, rename `<exe>.new` to `<exe>`; on failure roll back, delete `.new`, show "Update failed" and open the release page. If the exe's folder is not writable, only the release page is opened.
  - **Restart**: the old process starts the new exe with `--after-update <pid>` and exits normally. The new process waits up to 10 s for that pid before the single-instance check, then shows "Updated to <version>". At every startup `<exe>.old` is deleted if present (errors ignored).
- **Signing**: unsigned for now (SmartScreen warns on first run).

### Example `Nitor.ini`
```ini
[general]
tray_style = text          ; text | pill
wheel_step = 5
single_icon_mode = false
refresh_on_open = true
osd = false
write_interval_ms = 80
rescan_delay_ms = 2000
autostart = true
language = auto            ; auto | en | it
internal_display = false   ; also control the laptop's built-in screen
linked = false             ; "Link all" in the flyout
check_updates = true       ; check GitHub Releases at startup and every 24 h
last_update_check = 0      ; Unix seconds, written by Nitor

[monitor.DEL41D9-ABC123]
alias = Dell
color = #378ADD
order = 1
min = 0
max = 100
badge = D
link_min = 20              ; calibrated range used by "Link all"
link_max = 80

[hotkeys]
Ctrl+Alt+PgUp   = all +10
Ctrl+Alt+PgDn   = all -10
Ctrl+Alt+Home   = cursor set 80

[schedule]
07:00 = all b 40
20:30 = all b 20
```

---

## 11. Command line

GUI-subsystem exe; output goes to the parent console via `AttachConsole(ATTACH_PARENT_PROCESS)`. Commands are forwarded to the running instance; if none is running, `list`/`get` run standalone, `quit` does nothing, and anything else starts Nitor and then applies the commands.

```
Nitor.exe [target] b <n | +n | -n> [[target] b ...]
Nitor.exe [target] color <#RRGGBB>
Nitor.exe list                 # index, alias, stable ID, value, color
Nitor.exe get [target]         # prints value; also the exit code
Nitor.exe rescan | quit
Nitor.exe --config <path> ...

target := alias | name-substring | serial | #index | all | cursor
```

---

## 12. Edge cases

- **Docks, KVMs, DisplayLink, Miracast** usually don't pass DDC: the monitor gets no tray icon (listed by `Nitor.exe list` as not controllable).
- **Driver color settings, game modes, HDR presets** may block writes: read-only with explanation.
- **Sleep and wake**: never write while the display is off; optionally restore the last value after wake.
- **Values changed on the monitor's buttons**: picked up on flyout open or rescan.
- **Crashes**: a panic hook writes `Nitor.log` next to the config.

---

## 13. Non-functional targets

| Metric | Target |
|---|---|
| Exe size | < 2 MB |
| Idle RAM | < 15 MB |
| Idle CPU | ~0% (no polling) |
| Flyout open | < 100 ms |
| Wheel → icon update | < 16 ms |
| Cold start → icons visible | < 500 ms (values may arrive later) |

---

## 14. Milestones

| ID | Scope | Exit criterion |
|---|---|---|
| **M1** | Core logic, DDC workers, numbered per-monitor tray icons with colors, wheel hook, right-click menu, single instance, config, autostart, CLI, CI + release pipeline | Usable daily driver |
| **M2** | Fluent flyout: theming, DPI, color picker, calibrated link all; opt-in built-in laptop panel (WMI) | Visual parity with the spec |
| **M3** | Hotkeys, OSD, config hot-reload, single-icon mode | ClickMonitorDDC replacement complete |
| **M4** | Hardening: hot-plug, sleep, docks, 16 px legibility pass | Stable for 2 weeks of daily use |
| **M5** *(optional)* | Schedule, multi-point calibration curves | — |

---

## 15. Testing

- **Unit tests** (any OS): CLI parser, value scaling, throttling, OKLCH contrast, config round-trip, hotkey parsing.
- **Manual hardware matrix**: ≥3 monitor brands; HDMI, DisplayPort, USB-C, dock; 100/150/200% scaling; Windows 10 and 11; light, dark and accent taskbars; taskbar on each edge; Explorer restart, sleep/wake, hot-plug.

---

## 16. Nitor Plus: automation

Only in the Plus build (`--features plus`); `[auto]` is parsed in both builds but acted on only in Plus. The pure decisions live in `src/auto/` (`plan`, `drive`, `ambient`, `curve`, `solar`, `weather`); `src/win/auto/` owns timers, sensor, location and the writes.

**Engine.** Per monitor the engine keeps the level it last applied and the end of a manual override. A planning round (`drive::step`) turns the day/night curve (`day_brightness`/`night_brightness` over sun elevation, location from `[auto]`) into a master level, maps it through the monitor's `link_min..link_max` (`link::link_value`) and writes only if the monitor value moves by at least 2 points. The write goes through the normal brightness path (`set_value`: min/max limits, DDC throttling, EEPROM discipline of §7.2) but is marked automatic, so it is not a user override. A round runs when: the single timer fires, the first probe or a rescan finishes (startup, resume from sleep, display power-on, `WM_DISPLAYCHANGE`), settings are saved, the clock or time zone changes (`WM_TIMECHANGE`), a sensor level or a location arrives, or the pause is toggled. After resume the applied levels are forgotten, so a panel reset by sleep is corrected.

**Idle guarantees.** Nothing polls. There is exactly one recurring wake-up: `TIMER_AUTO`, set to the earliest `next_wake_unix` across monitors (`plan` sleeps until the next twilight transition start, wakes during a transition only when the level will have moved by 2 points, at most every 60 s, at most 12 h when the sun never crosses a boundary) and re-armed after every round; it is killed when nothing is scheduled (automation off, no location and no sensor). The sensor is a WinRT event; the location is a one-shot background thread. Every write and every planned wake is logged in `Nitor.log` (`auto write id=... target=...% reason=solar|sensor`, `auto next wake in N s`) so idle behaviour can be audited.

**Overrides and pause.** Any user change of a monitor's brightness (wheel, flyout slider or master, CLI; a built-in panel changed outside Nitor by at least 2 points, e.g. Fn keys) records an override for that monitor: until the next transition boundary (`override_mode = until_transition`, `plan::override_expiry`) or `override_minutes` minutes. The monitor's applied level is forgotten, so when the override ends the schedule is applied again. Overrides survive rescans, not restarts. The tray menu (Plus, automation on) has "Pause automation for 1 hour" / "Resume automation": a global pause kept in memory only.

**Ambient light sensor.** With `use_sensor` and a sensor present (`LightSensor::GetDefault`, looked up once), the engine subscribes to `ReadingChanged` with a report interval of at least 1 s and, where the system supports it (Windows 10 2004+), a report threshold (10 % / 5 lux) so the driver suppresses small changes. The event runs on a sensor thread: it stores the lux in an atomic and posts `WM_APP_AUTO_SENSOR`; no UI state is touched there. The UI thread feeds `AmbientFilter` (log-lux smoothing, outlier hold of 4 s, 2-point hysteresis). While the sensor has produced a level it replaces the solar curve, clamped to `[night_brightness, day_brightness]`; no timer is needed in that mode except overrides ending. Because a held-back jump needs a later reading to be believed and the sensor may stay silent, each reading arms a one-shot 5 s timer (`TIMER_SENSOR`) that judges the last reading again. Disabling automation or `use_sensor` unsubscribes and restores the sensor's default report interval. `auto::sensor_available()` tells the Settings UI whether to offer the option.

**Windows adaptive brightness.** If the built-in panel is automated while "Change brightness automatically when lighting changes" is on, Windows and Nitor would fight. `auto::adaptive_brightness_on()` reads `GUID_VIDEO_ADAPTIVE_DISPLAY_BRIGHTNESS` of the active scheme (AC or DC as appropriate) for the UI to show a hint; Nitor never changes it. A change of the built-in brightness by Windows is also treated as a user override, so automation steps back.

**Weather (opt-in).** `[auto] weather = true` (Settings, Automation; greyed out until a location is set) lowers the day level on cloudy days: `curve::with_clouds(cover, 0.3)`, i.e. up to 30 % dimmer under a fully overcast sky, never below the night level, so only the daytime part of the curve changes. Source: the Open-Meteo forecast API (`api.open-meteo.com/v1/forecast?...&hourly=cloud_cover&forecast_days=2&timezone=GMT`; the parser also accepts the older `cloudcover` key). Privacy: nothing is sent except the coordinates rounded to 1 decimal (about 10 km) and the usual `User-Agent: Nitor/<version>`; there is no request at all unless automation and weather are on and a location is set. Cadence: a background thread (shared `win::http`) fetches when enabled, then at most every 3 hours, and after a resume if due; a failure is retried after 30 minutes. There is no loop: the next fetch time is folded into the single `TIMER_AUTO` wake. The last good forecast is kept in memory only and is used for at most 36 h after it was fetched. Planning uses the cover of the current hour rounded down to 10 points, interpolation is only used for display; the next wake is the earlier of the curve's wake and the next hour boundary at which that bucket changes. Precedence: while the light sensor produces a level it replaces the solar curve and weather is ignored (the sensor already measures the real light). Each fetch logs one line (`weather: ok, cover now N%` or `weather: err: ...`); Settings shows the time and cover of the last update, or the error.

**Color temperature (experimental).** `[auto] color_enabled` (independent of `enabled`; Settings, Automation) follows the same twilight curve with `day_kelvin` (default 6500) and `night_kelvin` (default 4000), sliders 2700-6500 K in 100 K steps. Planning reuses `plan::plan` with `min_delta` = 100 K, so the planner wakes only when the temperature will have moved by 100 K, at most every 60 s, on its own one-shot `TIMER_COLOR` (killed when color is off, paused, or no location is set; no polling). Pausing automation from the tray restores neutral (6500 K) at once and resuming applies the plan again; there is no per-monitor override for color.

*Method per display, chosen automatically, hardware first* (`src/win/auto/color.rs`; each decision and each application is one `Nitor.log` line):

1. **DDC/CI gains**: an external monitor with a DDC handle is probed once, on its DDC worker, by reading VCP 0x16/0x18/0x1A (red/green/blue video gain; never the capabilities string). If all three answer, the worker remembers the original values and sets `gain_c = round(original_c * multiplier_c)` (`auto::color::rgb_multipliers`, `gain_target`), at most the monitor's reported maximum. EEPROM rules: a channel is written only if it changes by at least 1 raw step, and a monitor's gains are written at most every 5 minutes (a latest-wins request slot of the worker, throttled like brightness writes; it never blocks the UI). The originals are restored (immediately, not throttled) when color automation is turned off or paused, on exit, and before a rescan releases the handles (the UI waits up to 1.5 s). A rescan after display power-on or resume therefore restores and then applies again, which can write the gains twice. The experimental warm limit does not apply: hardware gets the full requested kelvin. A failed write puts the originals back and falls the display back to gamma.
2. **Gamma ramps**: the fallback, and always for the built-in panel. `SetDeviceGammaRamp` on a DC from `CreateDCW(None, device_name, ...)`. An identity ramp is set on disable, pause and exit; ramps are applied again after every rescan (`WM_DISPLAYCHANGE`, resume, display power-on), because Windows resets them. Displays with HDR / advanced colour on (`DisplayConfigGetDeviceInfo`, `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO`, `advancedColorEnabled`, read at scan time) are skipped and shown as "HDR: not supported".
3. **Warm limit.** Windows rejects ramps warmer than about 4000 K unless `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM\GdiIcmGammaRange` (DWORD) is 256. `experimental_warm = false` (default) clamps ramps to 4000 K and the sliders to 4000-6500 K (`auto::color::gamma_kelvin`); with it on the sliders go down to 2700 K, and while the registry value is missing the ramps still stay at 4000 K and Settings offers "Enable (needs administrator)": `reg.exe add ... /v GdiIcmGammaRange /t REG_DWORD /d 256 /f` through `ShellExecuteExW` with verb `runas` (UAC) on a background thread; when it ends Settings re-checks the value (a sign-out may be needed before Windows honours it). The whole option is labelled experimental.

*Windows Night Light.* Best effort: the `Data` blob of `HKCU\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate` is read (`auto::color::night_light_on`: `43 42 01 00` header, byte 18 is `0x15` while on; an undocumented, community-known layout, `false` on any doubt). When on while color automation is on, Settings shows a hint with a button that opens `ms-settings:nightlight`. Nitor never changes Night Light.

**Windows location.** With `location_source = windows`, `Geolocator` (access request, then one position with a 60 s timeout, a cached fix up to 1 h old accepted) runs on a background thread when automation is enabled and then at most once per 24 h (checked when a round runs, so also after resume); without coordinates a failed attempt is retried after 10 min. The result is rounded to 2 decimals, stored in `[auto] latitude/longitude`, saved, and a round runs. Failures are only logged.

---

## 17. Decisions (formerly open questions)

1. **Windows 10 and 11** are both supported (Windows 10 gets the simple visual fallback).
2. **External monitors by default**; the built-in laptop panel is opt-in (`internal_display`, or the tray menu).
3. **No hotkeys bound by default.**
4. **English and Italian**, picked from the Windows UI language (`language = auto`), with a string table that makes adding languages easy.
5. **GPL-3.0-or-later.**

---

## 18. References

- ClickMonitorDDC 7.2 instructions mirror: https://github.com/chrismah/ClickMonitorDDC7.2
- Twinkle Tray: https://github.com/xanderfrangos/twinkle-tray
- Monitorian: https://github.com/emoacht/Monitorian
- PowerToys Power Display: https://learn.microsoft.com/en-us/windows/powertoys/power-display
- PowerToys tray-wheel PR (low-level hook rationale): https://github.com/microsoft/PowerToys/pull/49580
- poulicek/BrightnessControl: https://github.com/poulicek/BrightnessControl
