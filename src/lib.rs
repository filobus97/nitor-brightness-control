//! Nitor: per-monitor DDC/CI brightness tray for Windows.
//!
//! The top-level modules are platform-independent and unit-tested on any OS,
//! so a future sibling project (e.g. a Linux/Wayland front-end) can reuse them.
//! Everything that touches Win32 lives in [`win`].

pub mod cli;
pub mod color;
pub mod config;
pub mod hotkey;
pub mod i18n;
pub mod link;
pub mod throttle;

#[cfg(windows)]
pub mod win;
