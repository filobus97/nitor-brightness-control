//! Windows front-end. Module ownership:
//! - `app`: UI thread, message window, glue between everything below.
//! - `monitors` + `ddc`: discovery, identity, DDC/CI worker threads.
//! - `icon` + `theme`: tray icon rendering and taskbar/app theme.
//! - `flyout` + `settings`: the Direct2D flyout and the Settings window.
//! - `http` + `update`: WinHTTP GET helper; release check and self-update.
//! - `tray`, `wheel`, `instance`, `autostart`, `util`: small Win32 wrappers.

pub mod app;
#[cfg(feature = "plus")]
pub mod auto;
pub mod autostart;
pub mod ddc;
pub mod flyout;
#[cfg(feature = "plus")]
pub mod geo;
pub mod http;
pub mod icon;
pub mod instance;
pub mod monitors;
pub mod settings;
pub mod theme;
pub mod tray;
pub mod update;
pub mod util;
pub mod wheel;
pub mod wmi;

use crate::cli::{self, Command};
use crate::config;
use instance::Instance;
use std::path::PathBuf;
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
use windows::core::PCWSTR;

/// Process entry point: parse CLI, attach the parent console for output,
/// forward to a running instance or run standalone `list`/`get`, otherwise run the app.
pub fn main() -> i32 {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let after_update = take_after_update(&mut args);
    if let Some(pid) = after_update {
        // Our predecessor holds the single-instance mutex until it exits.
        update::wait_for_exit(pid);
    }
    update::remove_old_exe();
    if !args.is_empty() {
        util::attach_parent_console();
    }
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "-h" | "--help" | "/?"))
    {
        print!("{}", cli::USAGE);
        return 0;
    }
    let inv = match cli::parse(&args) {
        Ok(inv) => inv,
        Err(e) => {
            eprintln!("{e}\n{}", cli::USAGE);
            return 2;
        }
    };

    let config_path = config::resolve_path(
        inv.config_path.as_deref(),
        &util::exe_dir(),
        util::appdata_dir().as_deref(),
    );
    util::set_log_path(config_path.with_file_name("Nitor.log"));
    util::install_panic_hook();

    match instance::acquire() {
        Instance::Secondary(hwnd) => forward_to_primary(hwnd, &args),
        // The guard keeps the single-instance mutex until the process exits.
        Instance::Primary(_guard) => run_primary(config_path, inv.commands, after_update.is_some()),
    }
}

/// Remove `--after-update <pid>` (added by the previous version when it restarts into an
/// update) from `args`; the pid of that process.
fn take_after_update(args: &mut Vec<String>) -> Option<u32> {
    let i = args.iter().position(|a| a == "--after-update")?;
    let end = (i + 2).min(args.len());
    let pid = args.get(i + 1).and_then(|p| p.parse().ok());
    args.drain(i..end);
    pid
}

/// A second launch: hand the command line to the running instance and relay its answer.
fn forward_to_primary(mut hwnd: HWND, args: &[String]) -> i32 {
    if hwnd.0.is_null() {
        hwnd = wait_for_primary_window();
    }
    if args.is_empty() {
        return 0;
    }
    if hwnd.0.is_null() {
        eprintln!("Nitor is running but did not answer.");
        return 1;
    }
    match instance::forward(hwnd, args) {
        Some((code, text)) => {
            if !text.is_empty() {
                println!("{text}");
            }
            code
        }
        None => {
            eprintln!("Could not send the command to the running Nitor.");
            1
        }
    }
}

/// The primary may still be starting up: look for its window for up to 2 s.
fn wait_for_primary_window() -> HWND {
    let class = util::wide(instance::WINDOW_CLASS);
    for _ in 0..20 {
        // SAFETY: `class` is a NUL-terminated wide string that outlives the call.
        if let Ok(hwnd) = unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) } {
            return hwnd;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    HWND::default()
}

/// No other instance runs (we hold the mutex): decide between standalone and the tray app.
fn run_primary(config_path: PathBuf, commands: Vec<Command>, updated: bool) -> i32 {
    let is_query = |c: &Command| matches!(c, Command::List | Command::Get(_));
    if !commands.is_empty() && commands.iter().all(is_query) {
        return app::standalone(&config_path, &commands);
    }
    // `quit` with nothing running does nothing.
    if !commands.is_empty() && commands.iter().all(|c| matches!(c, Command::Quit)) {
        return 0;
    }
    let initial = commands
        .into_iter()
        .filter(|c| !matches!(c, Command::Quit | Command::Rescan))
        .collect();
    app::run(config_path, initial, updated)
}
