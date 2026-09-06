//! GRID Craft Launcher core.
//!
//! Module map lives in `ARCHITECTURE.md`. This crate has no UI and no CLI parsing.

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
