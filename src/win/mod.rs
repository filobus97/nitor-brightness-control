//! Windows front-end. Module ownership:
//! - `app`: UI thread, message window, glue between everything below.
//! - `monitors` + `ddc`: discovery, identity, DDC/CI worker threads.
//! - `icon` + `theme`: tray icon rendering and taskbar/app theme.
//! - `tray`, `wheel`, `instance`, `autostart`, `util`: small Win32 wrappers.

pub mod app;
pub mod autostart;
pub mod ddc;
pub mod icon;
pub mod instance;
pub mod monitors;
pub mod theme;
pub mod tray;
pub mod util;
pub mod wheel;

/// Process entry point: parse CLI, attach the parent console for output,
/// forward to a running instance or run standalone `list`/`get`, otherwise run the app.
pub fn main() -> i32 {
    todo!()
}
