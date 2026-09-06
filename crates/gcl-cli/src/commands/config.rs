//! `gcl config`: show the config with keys redacted, and change the root or the JVM defaults.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::config::Config;
use serde::Serialize;

use crate::output::{Format, print_json};

/// Subcommands under `gcl config`.
#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Print the current config. API keys show as `<set>` or `<unset>`, never their value.
    Show,
    /// Point the launcher at another app root.
    SetRoot {
        /// Directory to use as the app root.
        path: PathBuf,
    },
    /// Change the default JVM heap bounds.
    SetJvm {
        /// Minimum heap size in MiB.
        #[arg(long)]
        min: Option<u32>,
        /// Maximum heap size in MiB.
        #[arg(long)]
        max: Option<u32>,
    },
}

/// The config as printed: the same shape, with the key values replaced by their state.
#[derive(Serialize)]
struct RedactedConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    parallel_downloads: usize,
    jvm: RedactedJvm,
    keys: RedactedKeys,
    game_defaults: BTreeMap<String, String>,
}

/// JVM defaults as printed.
#[derive(Serialize)]
struct RedactedJvm {
    min_mib: u32,
    max_mib: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    java_path: Option<String>,
}

/// Key states as printed. Never the values.
#[derive(Serialize)]
struct RedactedKeys {
    curseforge_api_key: &'static str,
    msa_client_id: &'static str,
}

impl RedactedConfig {
    /// Builds the printable view of a config.
    fn new(config: &Config) -> RedactedConfig {
        RedactedConfig {
            root: config.root.as_ref().map(|p| p.display().to_string()),
            parallel_downloads: config.parallel_downloads,
            jvm: RedactedJvm {
                min_mib: config.jvm.min_mib,
                max_mib: config.jvm.max_mib,
                java_path: config
                    .jvm
                    .java_path
                    .as_ref()
                    .map(|p| p.display().to_string()),
            },
            keys: RedactedKeys {
                curseforge_api_key: state(config.curseforge_api_key().is_some()),
                msa_client_id: state(config.msa_client_id().is_some()),
            },
            game_defaults: config.game_defaults.clone(),
        }
    }
}

/// The word printed in place of a key value.
fn state(present: bool) -> &'static str {
    if present { "<set>" } else { "<unset>" }
}

/// Runs one `gcl config` subcommand.
pub fn run(launcher: &mut Launcher, format: Format, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => {
            let view = RedactedConfig::new(launcher.config());
            match format {
                Format::Json => print_json(&view),
                Format::Text => {
                    print!("{}", toml::to_string_pretty(&view)?);
                    Ok(())
                }
            }
        }
        ConfigCommand::SetRoot { path } => {
            launcher.config_mut().root = Some(path.clone());
            launcher.save_config()?;
            report(format, "root", &path.display().to_string())
        }
        ConfigCommand::SetJvm { min, max } => {
            if min.is_none() && max.is_none() {
                bail!("set-jvm needs --min, --max, or both");
            }
            if let Some(min) = min {
                launcher.config_mut().jvm.min_mib = min;
            }
            if let Some(max) = max {
                launcher.config_mut().jvm.max_mib = max;
            }
            let jvm = &launcher.config().jvm;
            let (min_mib, max_mib) = (jvm.min_mib, jvm.max_mib);
            if min_mib > max_mib {
                bail!("min heap {min_mib} MiB is above max heap {max_mib} MiB");
            }
            launcher.save_config()?;
            match format {
                Format::Json => {
                    print_json(&serde_json::json!({ "min_mib": min_mib, "max_mib": max_mib }))
                }
                Format::Text => {
                    println!("jvm min_mib = {min_mib}, max_mib = {max_mib}");
                    Ok(())
                }
            }
        }
    }
}

/// Prints one changed setting.
fn report(format: Format, key: &str, value: &str) -> Result<()> {
    match format {
        Format::Json => print_json(&serde_json::json!({ key: value })),
        Format::Text => {
            println!("{key} = {value}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_key_never_prints_its_value() {
        let mut config = Config::default();
        config.keys.curseforge_api_key = Some("super-secret".to_string());
        let text = toml::to_string_pretty(&RedactedConfig::new(&config)).expect("serialize");
        assert!(text.contains("curseforge_api_key = \"<set>\""), "{text}");
        assert!(!text.contains("super-secret"), "{text}");
    }

    #[test]
    fn an_unset_key_prints_unset() {
        let text =
            toml::to_string_pretty(&RedactedConfig::new(&Config::default())).expect("serialize");
        assert!(text.contains("msa_client_id = \"<unset>\""), "{text}");
        assert!(text.contains("max_mib = 4096"), "{text}");
    }
}
