//! `gcl settings`: per-instance `options.txt` overrides and the launcher-wide defaults.
//!
//! An override is written to `instance.toml` only. It reaches `options.txt` at the next
//! launch, so a running game never has its settings changed underneath it.

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
        /// Value to write.
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
        /// Value to write.
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
            let mut instance = launcher.instances().get(&slug)?;
            instance
                .config
                .settings_overrides
                .insert(key.clone(), value.clone());
            instance.save()?;
            report_change(format, &key, Some(&value))
        }
        SettingsCommand::Unset { slug, key } => {
            let mut instance = launcher.instances().get(&slug)?;
            instance.config.settings_overrides.remove(&key);
            instance.save()?;
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
            launcher
                .config_mut()
                .game_defaults
                .insert(key.clone(), value.clone());
            launcher.save_config()?;
            report_change(format, &key, Some(&value))
        }
        DefaultsCommand::Unset { key } => {
            launcher.config_mut().game_defaults.remove(&key);
            launcher.save_config()?;
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
