//! UI thread: hidden window, tray icons, menu, wheel, system events (spec §4.1–4.3, §8–10).

use crate::cli::Command;
use std::path::PathBuf;

/// Run the tray app until Exit/`quit`. `initial` are CLI commands to apply once
/// monitors are known. Returns the process exit code.
pub fn run(config_path: PathBuf, initial: Vec<Command>) -> i32 {
    let _ = (config_path, initial);
    todo!()
}
