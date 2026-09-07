//! `gcl settings`: per-instance `options.txt` overrides and the launcher-wide defaults.
//!
//! An override is written to `instance.toml` only. It reaches `options.txt` at the next
//! launch, so a running game never has its settings changed underneath it.
//!
//! `set` and `defaults set` go through [`Launcher::set_instance_override`] and
//! [`Launcher::set_game_default`], so the CLI rejects the same out-of-range and bad-choice
//! values as the GUI does. A value is normalised first by
//! [`gcl_core::settings::doc::normalize`], so a choice typed bare is stored the way
//! `options.txt` writes it: `fast` is saved as `"fast"`.
//!
//! Values are in the form `options.txt` stores, not the form the game's own screens show. The
//! one place the two differ is `fov`, stored as `-1..1` for 30° to 110°.

use std::collections::BTreeMap;

use anyhow::Result;
use clap::Subcommand;
use gcl_core::Launcher;
use serde::Serialize;

use crate::output::{Format, print_json};

/// Subcommands under `gcl settings`.
#[derive(Subcommand)]
pub enum SettingsCommand {
    /// Print an instance's override map and its current `options.txt`.
    Show {
        /// Slug of the instance.
        slug: String,
    },
    /// Set one override key. It is applied to `options.txt` at the next launch.
    Set {
        /// Slug of the instance.
        slug: String,
        /// `options.txt` key, such as renderDistance.
        key: String,
        /// Value as `options.txt` stores it. A choice may be typed bare (`fast`). `fov` is
        /// stored as -1..1, where -1 is 30 degrees and 1 is 110.
        value: String,
    },
    /// Remove one override key. The value already in `options.txt` stays as it is.
    Unset {
        /// Slug of the instance.
        slug: String,
        /// `options.txt` key to stop overriding.
        key: String,
    },
    /// The defaults new instances are preseeded with.
    Defaults {
        #[command(subcommand)]
        command: DefaultsCommand,
    },
}

/// Subcommands under `gcl settings defaults`.
#[derive(Subcommand)]
pub enum DefaultsCommand {
    /// Print the launcher-wide `options.txt` defaults.
    Show,
    /// Set one default key.
    Set {
        /// `options.txt` key.
        key: String,
        /// Value as `options.txt` stores it. A choice may be typed bare (`fast`). `fov` is
        /// stored as -1..1, where -1 is 30 degrees and 1 is 110.
        value: String,
    },
    /// Remove one default key.
    Unset {
        /// `options.txt` key.
        key: String,
    },
}

/// What `settings show` prints.
#[derive(Serialize)]
struct SettingsView {
    /// The instance's override map, from `instance.toml`.
    overrides: BTreeMap<String, String>,
    /// Every `key:value` pair currently in the instance's `options.txt`.
    options: BTreeMap<String, String>,
}

/// Runs one `gcl settings` subcommand.
pub fn run(launcher: &mut Launcher, format: Format, command: SettingsCommand) -> Result<()> {
    match command {
        SettingsCommand::Show { slug } => {
            let instance = launcher.instances().get(&slug)?;
            let file = gcl_core::settings::read(&instance.game_dir().join("options.txt"))?;
            let view = SettingsView {
                overrides: instance.config.settings_overrides.clone(),
                options: file
                    .pairs()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            };
            match format {
                Format::Json => print_json(&view),
                Format::Text => {
                    println!("overrides:");
                    print_pairs(&view.overrides);
                    println!("options.txt:");
                    print_pairs(&view.options);
                    Ok(())
                }
            }
        }
        SettingsCommand::Set { slug, key, value } => {
            let value = gcl_core::settings::doc::normalize(&key, &value)?;
            launcher.set_instance_override(&slug, &key, &value)?;
            report_change(format, &key, Some(&value))
        }
        SettingsCommand::Unset { slug, key } => {
            launcher.unset_instance_override(&slug, &key)?;
            report_change(format, &key, None)
        }
        SettingsCommand::Defaults { command } => defaults(launcher, format, command),
    }
}

/// Runs one `gcl settings defaults` subcommand.
fn defaults(launcher: &mut Launcher, format: Format, command: DefaultsCommand) -> Result<()> {
    match command {
        DefaultsCommand::Show => {
            let map = launcher.config().game_defaults.clone();
            match format {
                Format::Json => print_json(&map),
                Format::Text => {
                    print_pairs(&map);
                    Ok(())
                }
            }
        }
        DefaultsCommand::Set { key, value } => {
            let value = gcl_core::settings::doc::normalize(&key, &value)?;
            launcher.set_game_default(&key, &value)?;
            report_change(format, &key, Some(&value))
        }
        DefaultsCommand::Unset { key } => {
            launcher.unset_game_default(&key)?;
            report_change(format, &key, None)
        }
    }
}

/// Prints one `  key: value` line per entry, or a placeholder when there are none.
fn print_pairs(pairs: &BTreeMap<String, String>) {
    if pairs.is_empty() {
        println!("  (none)");
        return;
    }
    for (key, value) in pairs {
        println!("  {key}: {value}");
    }
}

/// Reports a set or unset, as JSON or as one line.
fn report_change(format: Format, key: &str, value: Option<&str>) -> Result<()> {
    match format {
        Format::Json => print_json(&serde_json::json!({ "key": key, "value": value })),
        Format::Text => {
            match value {
                Some(value) => println!("set {key} = {value}"),
                None => println!("unset {key}"),
            }
            Ok(())
        }
    }
}
