# Nitor Brightness Control

Nitor is a lightweight, easy-to-use brightness control for external monitors on Windows, filling the gap Windows leaves.

It's a single portable `.exe` that sits in the notification area with **one tray icon per monitor**, each showing that monitor's brightness as a number in its own color. **Scroll over an icon** to change that monitor's brightness. Hardware brightness is set over DDC/CI, the same way the monitor's own buttons do it.

- One small native exe: no installer, no runtime, no admin rights, no telemetry. The only network use is the update check (see [Updates](#updates)).
- Brightness only, done well. No overlays or gamma tricks (color temperature is an opt-in Nitor Plus feature).
- Windows 10 (1809+) and Windows 11, x64 and ARM64.
- English and Italian, following the Windows display language.

> **Status:** early development. See [`docs/SPEC.md`](docs/SPEC.md) for the design and milestones.

## Download

Get the exe for your processor from the [Releases](https://github.com/filobus97/nitor-brightness-controll/releases) page, put it in any folder and run it:

- **`Nitor-x64.exe`**: most PCs (Intel or AMD).
- **`Nitor-arm64.exe`**: ARM PCs only (Snapdragon, e.g. Surface Pro X, Copilot+ PCs). On an Intel/AMD PC it fails with "This app can't run on your PC".

Not sure? **Settings → System → About → System type**. The binary isn't code-signed yet, so Windows SmartScreen may warn on first launch ("More info" → "Run anyway").

Windows 11 hides new tray icons in the overflow menu. To keep Nitor's icons visible, drag them onto the taskbar, or turn them on in **Settings → Personalization → Taskbar → Other system tray icons**.

## Editions

- **Nitor** (`Nitor-x64.exe`, `Nitor-arm64.exe`): per-monitor brightness, everything described here.
- **Nitor Plus** (`NitorPlus-x64.exe`, `NitorPlus-arm64.exe`): everything in Nitor plus automation features: automatic brightness and color temperature (see below).

Each edition updates itself to the same edition's newer release. Build Plus with `cargo build --release --features plus`.

### Nitor Plus: automatic brightness and color temperature

Set your location (or let Windows provide it) and the day and night brightness in **Settings → Automation**, and Nitor follows the sun: dim at night, bright by day, changing gradually around dawn and dusk, each monitor within its own linked range. If the PC has an ambient light sensor, Nitor can follow the room's light instead. If you change a monitor by hand, automation leaves it alone until the next sunrise or sunset (or for a set number of minutes); **Pause automation for 1 hour** in the tray menu stops it temporarily. Nitor stays idle in between: it sleeps until the next change is due and does nothing else. Optionally (off by default) Nitor can dim the day level a little on cloudy days, using the cloud-cover forecast from Open-Meteo: it sends only your approximate location (rounded to about 10 km), at most every 3 hours; the light sensor, when used, takes precedence. On a laptop, turn off Windows' own "Change brightness automatically when lighting changes" so the two don't compete. 

**Color temperature (experimental).** Turn on *Automatic color temperature* in **Settings → Automation** and set a day and a night temperature (2700 to 6500 K); Nitor warms the screen after sunset on the same schedule, waking only when the color has moved by about 100 K. By default it uses a gamma ramp (software), which always works and never touches the monitor. Per monitor you can choose **Hardware** in the Color card to use the monitor's own RGB gains over DDC/CI instead; most monitors apply them only in their User/Custom color mode, and if the monitor ignores the change Nitor notices, puts the gains back and uses Software (Nitor saves the monitor's original values in `Nitor.ini` the first time you use Hardware and puts exactly those back when you return to 6500 K, turn it off, pause, switch method, or exit; your own changes apply immediately, while the automatic schedule writes at most every 5 minutes to spare the monitor's memory; if a write fails, the originals go straight back. **Restore monitor colors** under the Hardware choice writes them back and switches the monitor to Software). The built-in display always uses gamma. Windows limits gamma ramps to 4000 K; going warmer needs *Allow warmer than 4000 K (experimental)*, a one-time registry change that asks for administrator rights (you may need to sign out and in). Displays in HDR are skipped. If Windows' own Night Light is on, Nitor shows a hint, since both would tint the screen. **Pause automation for 1 hour** restores neutral color until you resume.

**Color slider in the flyout.** Nitor Plus adds a row under the monitors with a slider from 2700 K (warm; 4000 K unless the experimental warm option is on) to 6500 K (neutral) that shows the current color temperature. With automatic color on, moving it holds the automation off like a manual brightness change (until the next sunrise or sunset, or for the set minutes) and then it resumes; with it off, the slider sets a fixed temperature that Nitor keeps and re-applies at start (6500 K goes back to neutral). While you drag, only the screen's gamma follows; monitors that use DDC/CI get the final value as soon as you let go.

**Location search.** In **Settings → Automation → Location**, type a city name and press Search to pick your place.

See [`docs/SPEC.md` §16](docs/SPEC.md#16-nitor-plus-automation).

## Usage

| Action | Result |
|---|---|
| Mouse wheel over an icon | Brightness ± step (default 5) |
| Shift + wheel | ± 1 |
| Left click | Flyout with one slider per monitor; click a color dot to change that monitor's color |
| Right click | Rescan monitors · Settings · Start with Windows · Control built-in display (laptops) · Exit |

**Link all** (in the flyout footer) adds a master slider that moves every monitor together. Each monitor's slider then shows two small handles: drag them to set the range that monitor follows, so screens with different panels look equally bright.

**Laptops:** the built-in screen is left alone by default. Enable **Control built-in display** in the tray menu to get an icon and slider for it too.

### Command line

```
Nitor.exe [target] b <n | +n | -n> [[target] b ...]
Nitor.exe [target] color <#RRGGBB>
Nitor.exe list | get [target] | rescan | quit

target := alias | name-substring | serial | #index | all | cursor
```

Examples: `Nitor.exe b 50`, `Nitor.exe Dell b +10 LG b 30`, `Nitor.exe all b -20`.

Commands are sent to the running instance; if Nitor isn't running, it starts.

If both editions are installed, **Start with Windows** follows whichever you ran last; Nitor tells you when it switches the entry.

### Settings

**Settings** in the tray menu (or the gear in the flyout) opens a window with everything you can change: start with Windows, language, update checks, tray icon style and wheel step, and per monitor the color, name, badge, whether it shows in the tray, its brightness limits and its position in the tray. Changes apply immediately.

For power users everything is also in `Nitor.ini`, next to the exe when that folder is writable, otherwise in `%APPDATA%\Nitor\`; **Open config file** in the Settings window opens it. It also holds the options without a screen (hotkeys, schedule). Nitor reads it again on **Rescan monitors**. See [`docs/SPEC.md` §10](docs/SPEC.md#10-configuration-and-portability) for all options.

## Updates

Nitor checks GitHub Releases for a newer version at every start and then at most once a day, and tells you with a tray notification (which Windows may hold back during quiet time or Focus). **Check for updates** shows the result in Settings (General: "Up to date", "vX available" with an **Install** button, or the error); without Settings open it uses a notification. Every check is logged in `Nitor.log`. It never installs anything on its own: choose **Update to x.y.z** in the tray menu to download the new exe, verify its SHA-256 checksum, replace itself and restart. The only data sent is a `Nitor/<version>` user agent; nothing else leaves your PC. To turn the automatic check off, untick **Check for updates automatically** in Settings (`check_updates = false` in `Nitor.ini`). If Nitor's folder isn't writable, the menu item opens the release page instead.

## Monitors that don't respond

Nitor talks to monitors over DDC/CI. If a monitor doesn't get an icon or shows "–":

- Turn on **DDC/CI** in the monitor's on-screen menu (some models ship with it off).
- Docks, KVM switches, DisplayLink adapters and wireless displays often don't pass DDC/CI. Try a direct cable.
- Game modes, HDR presets and some driver color settings lock brightness; the tooltip says so when that happens.

## Building

Requires Rust (stable) with the MSVC toolchain on Windows:

```
cargo build --release
```

The platform-independent logic (CLI grammar, config, colors, write throttling) builds and tests on any OS with `cargo test`.

## Releasing

Bump `version` in `Cargo.toml` and merge to `main`: CI publishes `vX.Y.Z` automatically when that release doesn't exist yet (a matching pushed tag, or **Run workflow** with **release** ticked, also work). CI builds both exes and publishes them with `SHA256SUMS.txt`; running copies of Nitor pick the new version up through their update check.

## Support

Nitor is free and open source. If it's useful to you, you can support me and this project on Ko-fi; it is entirely optional. Nitor shows a small, occasional reminder about this, which you can turn off in Settings (`support_reminders = false`).

[![Support me on Ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/pippo97)

## License

[GPL-3.0-or-later](LICENSE). Nitor is inspired by ClickMonitorDDC, [Twinkle Tray](https://github.com/xanderfrangos/twinkle-tray) and [Monitorian](https://github.com/emoacht/Monitorian), but shares no code with them.
