//! GRID Craft Launcher core.
//!
//! Module map lives in `ARCHITECTURE.md`. This crate has no UI and no CLI parsing.

pub mod config;
pub mod download;
pub mod error;
pub mod events;
pub mod http;
pub mod java;
pub mod mojang;
pub mod paths;

pub use error::Error;

/// Result alias using the crate-level [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Launcher version, taken from the workspace package version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// User-Agent for every outbound HTTP request. Modrinth requires a contact address.
pub const USER_AGENT: &str = concat!(
    "sixdd6/grid-craft-launcher/",
    env!("CARGO_PKG_VERSION"),
    " (sixdd6@gmail.com)"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_names_app_and_contact() {
        assert!(USER_AGENT.starts_with("sixdd6/grid-craft-launcher/"));
        assert!(USER_AGENT.contains(VERSION));
        assert!(USER_AGENT.ends_with("(sixdd6@gmail.com)"));
    }
}
