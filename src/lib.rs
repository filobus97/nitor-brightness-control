//! Nitor: per-monitor DDC/CI brightness tray for Windows.
//!
//! The top-level modules are platform-independent and unit-tested on any OS,
//! so a future sibling project (e.g. a Linux/Wayland front-end) can reuse them.
//! Everything that touches Win32 lives in [`win`].

#[cfg(feature = "plus")]
pub mod auto;
pub mod cli;
pub mod color;
pub mod config;
pub mod display_id;
pub mod hotkey;
pub mod i18n;
pub mod link;
pub mod throttle;
pub mod update;

#[cfg(windows)]
pub mod win;
