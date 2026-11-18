# Nitor Brightness Control

Nitor is a lightweight, easy-to-use brightness control for external monitors on Windows, filling the gap Windows leaves.

It's a single portable `.exe` that sits in the notification area with **one tray icon per monitor**, each showing that monitor's brightness as a number in its own color. **Scroll over an icon** to change that monitor's brightness. Hardware brightness is set over DDC/CI, the same way the monitor's own buttons do it.

- One small native exe: no installer, no runtime, no admin rights, no telemetry. The only network use is the update check (see [Updates](#updates)).
- Brightness only, done well. No overlays or gamma tricks.
- Windows 10 (1809+) and Windows 11, x64 and ARM64.
- English and Italian, following the Windows display language.

> **Status:** early development. See [`docs/SPEC.md`](docs/SPEC.md) for the design and milestones.

## Download

Get `Nitor-x64.exe` (or `Nitor-arm64.exe`) from the [Releases](https://github.com/filobus97/nitor-brightness-controll/releases) page, put it in any folder and run it. The binary isn't code-signed yet, so Windows SmartScreen may warn on first launch ("More info" → "Run anyway").

Windows 11 hides new tray icons in the overflow menu. To keep Nitor's icons visible, drag them onto the taskbar, or turn them on in **Settings → Personalization → Taskbar → Other system tray icons**.

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

### Settings

**Settings** in the tray menu (or the gear in the flyout) opens a window with everything you can change: start with Windows, language, update checks, tray icon style and wheel step, and per monitor the color, name, badge, whether it shows in the tray, its brightness limits and its position in the tray. Changes apply immediately.

For power users everything is also in `Nitor.ini`, next to the exe when that folder is writable, otherwise in `%APPDATA%\Nitor\`; **Open config file** in the Settings window opens it. It also holds the options without a screen (hotkeys, schedule). Nitor reads it again on **Rescan monitors**. See [`docs/SPEC.md` §10](docs/SPEC.md#10-configuration-and-portability) for all options.

## Updates

Nitor checks GitHub Releases for a newer version at startup and then at most once a day, and tells you with a tray notification. It never installs anything on its own: choose **Update to x.y.z** in the tray menu to download the new exe, verify its SHA-256 checksum, replace itself and restart. **Check for updates** in the menu checks on demand. The only data sent is a `Nitor/<version>` user agent; nothing else leaves your PC. To turn the automatic check off, untick **Check for updates automatically** in Settings (`check_updates = false` in `Nitor.ini`). If Nitor's folder isn't writable, the menu item opens the release page instead.

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

## License

[GPL-3.0-or-later](LICENSE). Nitor is inspired by ClickMonitorDDC, [Twinkle Tray](https://github.com/xanderfrangos/twinkle-tray) and [Monitorian](https://github.com/emoacht/Monitorian), but shares no code with them.
