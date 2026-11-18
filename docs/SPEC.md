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
- Volume, contrast, color temperature, input switching, power state.
- Software dimming (overlays, gamma ramps).
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
| `src/win/tray.rs` | `Shell_NotifyIcon` wrappers |
| `src/win/instance.rs` | Single instance + argument forwarding |
| `src/win/autostart.rs` | `HKCU\...\Run` entry |
| `src/update.rs`, `src/win/update.rs` | Release lookup and self-update: version/JSON/checksum logic (portable) and WinHTTP + CNG + file swap |

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

- **Config location**: `--config <path>`; else `Nitor.ini` next to the exe if that folder is writable; else `%APPDATA%\Nitor\Nitor.ini`.
- **Settings**: the "Settings" menu item opens `Nitor.ini` in the default editor; Nitor reloads it when the file changes. A graphical settings page is not planned; common settings are reachable from the flyout and tray menu.
- **Tray icon identity**: by `uID`, never GUID (GUID icons are bound to the exe path, which breaks portability).
- **Autostart**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, written only when enabled; a stale path is fixed on launch.
- **Single instance**: named mutex; a second launch forwards its arguments via `WM_COPYDATA` and exits.
- **Explorer restarts**: icons re-added on `TaskbarCreated`.
- **Updates**: Nitor checks the latest GitHub release quietly in a background thread after the first probe, then at most every 24 h (an hourly timer tests whether 24 h passed, so sleep is handled). `[general] check_updates = false` turns automatic checks off; `last_update_check` (Unix seconds) records the last automatic check. When a newer version exists, a tray balloon announces it once per version and session and the tray menu gains "Update to <version>". Installing always needs that click. "Check for updates" in the menu checks on demand, also when `check_updates` is off, and reports the result by balloon.
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
