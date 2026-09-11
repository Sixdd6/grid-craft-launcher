//! `gcl config`: show the config with keys redacted, and change the root or the JVM defaults.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::config::Config;
use gcl_core::instances::model::GcPreset;
use serde::Serialize;

use crate::commands::GcPresetArg;
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
    /// Delete the oldest cached icons and description images over their size caps.
    ///
    /// `cache/icons` is capped at 64 MiB and `cache/images` at 256 MiB. The launcher runs
    /// this once at every start; the command is the way to run it now.
    PruneCache,
    /// Change the default JVM heap bounds and garbage collector preset.
    ///
    /// The preset seeds a new instance's own `jvm.gc` when it is created. It is not read
    /// again at launch: change an existing instance with `gcl instance jvm <slug> --gc`.
    SetJvm {
        /// Minimum heap size in MiB.
        #[arg(long)]
        min: Option<u32>,
        /// Maximum heap size in MiB.
        #[arg(long)]
        max: Option<u32>,
        /// Garbage collector preset new instances start with.
        #[arg(long, value_enum)]
        gc: Option<GcPresetArg>,
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
    gc: String,
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
                gc: config.jvm.gc.to_string(),
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
            let view = RedactedConfig::new(&launcher.config());
            match format {
                Format::Json => print_json(&view),
                Format::Text => {
                    print!("{}", toml::to_string_pretty(&view)?);
                    Ok(())
                }
            }
        }
        ConfigCommand::SetRoot { path } => {
            let old_root = launcher.root().path().display().to_string();
            let new_root = path.display().to_string();
            launcher.update_config(|config| config.root = Some(path))?;
            match format {
                Format::Json => print_json(&serde_json::json!({
                    "root": new_root,
                    "previous_root": old_root,
                })),
                Format::Text => {
                    println!("root = {new_root}");
                    println!(
                        "existing data stays at {old_root}; move it by hand if you want it in \
                         the new root"
                    );
                    Ok(())
                }
            }
        }
        ConfigCommand::PruneCache => {
            let (icons, images) = launcher.prune_media_caches()?;
            match format {
                Format::Json => print_json(&serde_json::json!({
                    "icons": {
                        "removed": icons.removed,
                        "freed_bytes": icons.freed_bytes,
                        "remaining_bytes": icons.remaining_bytes,
                    },
                    "images": {
                        "removed": images.removed,
                        "freed_bytes": images.freed_bytes,
                        "remaining_bytes": images.remaining_bytes,
                    },
                })),
                Format::Text => {
                    for (name, pruned) in [("icons", icons), ("images", images)] {
                        println!(
                            "{name}: removed {} files, freed {} bytes, {} bytes left",
                            pruned.removed, pruned.freed_bytes, pruned.remaining_bytes
                        );
                    }
                    Ok(())
                }
            }
        }
        ConfigCommand::SetJvm { min, max, gc } => {
            if min.is_none() && max.is_none() && gc.is_none() {
                bail!("set-jvm needs at least one of --min, --max, or --gc");
            }
            let (min_mib, max_mib, preset) = {
                let jvm = &launcher.config().jvm;
                (
                    min.unwrap_or(jvm.min_mib),
                    max.unwrap_or(jvm.max_mib),
                    gc.map_or(jvm.gc, GcPreset::from),
                )
            };
            if min_mib > max_mib {
                bail!("min heap {min_mib} MiB is above max heap {max_mib} MiB");
            }
            launcher.update_config(|config| {
                config.jvm.min_mib = min_mib;
                config.jvm.max_mib = max_mib;
                config.jvm.gc = preset;
            })?;
            match format {
                Format::Json => print_json(&serde_json::json!({
                    "min_mib": min_mib,
                    "max_mib": max_mib,
                    "gc": preset.to_string(),
                })),
                Format::Text => {
                    println!("jvm min_mib = {min_mib}, max_mib = {max_mib}, gc = {preset}");
                    Ok(())
                }
            }
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
    fn the_printed_config_names_the_default_gc_preset() {
        let mut config = Config::default();
        let text = toml::to_string_pretty(&RedactedConfig::new(&config)).expect("serialize");
        assert!(text.contains("gc = \"default\""), "{text}");
        config.jvm.gc = GcPreset::ZgcGenerational;
        let text = toml::to_string_pretty(&RedactedConfig::new(&config)).expect("serialize");
        assert!(text.contains("gc = \"zgc_generational\""), "{text}");
    }

    #[test]
    fn an_unset_key_prints_unset() {
        let text =
            toml::to_string_pretty(&RedactedConfig::new(&Config::default())).expect("serialize");
        assert!(text.contains("msa_client_id = \"<unset>\""), "{text}");
        assert!(text.contains("max_mib = 4096"), "{text}");
    }
}
