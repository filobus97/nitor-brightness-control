//! Nitor Plus automation: the platform-independent core.
//!
//! Everything here is pure and deterministic: no I/O, no clocks (time is passed in as
//! unix seconds) and no `windows` crate. The Windows glue (timers, sensor, location,
//! DDC writes, gamma ramps) lives later in `src/win/auto/` and only feeds inputs to
//! these functions and applies their outputs.
//!
//! Idle-first design: nothing polls. [`plan::plan`] answers both "what level now?"
//! and "when must I wake up next?", so the app sleeps until the next transition
//! start, and wakes during a transition only when the level will have moved visibly.
//!
//! * [`solar`]: sun elevation and the next time it crosses a given elevation.
//! * [`curve`]: level as a function of elevation (brightness or kelvin), cloud dimming.
//! * [`ambient`]: ambient-light sensor mapping, smoothing and hysteresis.
//! * [`plan`]: scheduler decision, manual override and pause handling.
//! * [`drive`]: per-monitor step combining plan, sensor level, link range and overrides.
//! * [`color`]: kelvin to RGB multipliers and gamma ramps.
//! * [`geocode`]: place-search URL and answer parsing.
//! * [`weather`]: Open-Meteo cloud-cover forecast URL, parsing and lookups.
//! * [`map`]: offline world map data and the picker's zoom/pan view.

pub mod ambient;
pub mod color;
pub mod curve;
pub mod drive;
pub mod geocode;
pub mod map;
pub mod plan;
pub mod solar;
pub mod weather;
