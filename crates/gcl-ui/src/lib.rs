//! Desktop UI for GRID Craft Launcher. Core logic stays in `gcl-core`; this crate renders and
//! forwards events.
//!
//! The crate is a library so integration tests under `tests/` can build the real `AppWindow`
//! over a `Launcher` and drive it through the Slint testing backend. `src/main.rs` is a thin
//! binary: arguments, logging, [`app::build`], run.

slint::include_modules!();

pub mod app;
pub mod bridge;
pub mod events;
pub mod keys;
pub mod launch_flow;
pub mod logging;
pub mod models;
pub mod screens;
pub mod state;
pub mod toasts;
