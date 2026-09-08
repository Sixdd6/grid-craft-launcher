//! `gcl instance`: create, list, rename, and delete instances.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use gcl_core::Launcher;
use gcl_core::instances::model::{GcPreset, InstanceJvm, Loader};
use serde::Serialize;

use crate::commands::GcPresetArg;
use crate::output::{Format, print_json, print_table};

/// Subcommands under `gcl instance`.
#[derive(Subcommand)]
pub enum InstanceCommand {
    /// Create an instance directory and its `instance.toml`.
    Create {
        /// Display name. The slug is derived from it.
        name: String,
        /// Minecraft version id, such as 1.20.1.
        #[arg(long)]
        minecraft: String,
        /// Mod loader this instance runs.
        #[arg(long, value_enum, default_value_t = LoaderArg::None)]
        loader: LoaderArg,
        /// Loader version, when the loader needs one.
        #[arg(long)]
        loader_version: Option<String>,
    },
    /// List every instance under the app root.
    List,
    /// Delete an instance directory and everything in it.
    Delete {
        /// Slug of the instance to delete.
        slug: String,
        /// Confirm the deletion. Required.
        #[arg(long)]
        yes: bool,
    },
    /// Change one instance's JVM settings. Flags left out keep their current value.
    ///
    /// `--gc` is checked against the Java this instance will launch with once this command
    /// has run — `--java-path` included — which is probed first. A preset that Java lacks is
    /// refused and nothing at all is written, heap fields included.
    Jvm {
        /// Slug of the instance.
        slug: String,
        /// Minimum heap size in MiB.
        #[arg(long)]
        min: Option<u32>,
        /// Maximum heap size in MiB.
        #[arg(long)]
        max: Option<u32>,
        /// Extra JVM argument. Repeat the flag for more than one; replaces the whole list.
        /// Passing none keeps the current list: use --clear-extra to empty it.
        #[arg(long)]
        extra: Vec<String>,
        /// Empty the extra JVM argument list.
        #[arg(long, conflicts_with = "extra")]
        clear_extra: bool,
        /// Path to the `java` binary this instance launches with.
        #[arg(long)]
        java_path: Option<PathBuf>,
        /// Forget this instance's java path, so the launcher picks a runtime again.
        #[arg(long, conflicts_with = "java_path")]
        clear_java_path: bool,
        /// Garbage collector preset. See `gcl instance gc <slug>` for the ones this Java has.
        #[arg(long, value_enum)]
        gc: Option<GcPresetArg>,
    },
    /// Print the garbage collector presets this instance's Java can run.
    Gc {
        /// Slug of the instance.
        slug: String,
    },
    /// Change an instance's display name. The slug stays as it is.
    Rename {
        /// Slug of the instance to rename.
        slug: String,
        /// The new display name.
        new_name: String,
    },
}

/// Mod loader as spelled on the command line.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LoaderArg {
    /// Vanilla, no loader.
    None,
    /// Fabric Loader.
    Fabric,
    /// Quilt Loader.
    Quilt,
    /// Forge.
    Forge,
    /// NeoForge.
    #[value(name = "neoforge")]
    NeoForge,
}

impl From<LoaderArg> for Loader {
    fn from(arg: LoaderArg) -> Loader {
        match arg {
            LoaderArg::None => Loader::None,
            LoaderArg::Fabric => Loader::Fabric,
            LoaderArg::Quilt => Loader::Quilt,
            LoaderArg::Forge => Loader::Forge,
            LoaderArg::NeoForge => Loader::NeoForge,
        }
    }
}

/// One instance as the CLI reports it.
#[derive(Serialize)]
struct InstanceRow {
    slug: String,
    name: String,
    minecraft: String,
    loader: String,
}

impl From<&gcl_core::instances::Instance> for InstanceRow {
    fn from(instance: &gcl_core::instances::Instance) -> InstanceRow {
        InstanceRow {
            slug: instance.slug.clone(),
            name: instance.config.name.clone(),
            minecraft: instance.config.minecraft.clone(),
            loader: instance.config.loader.to_string(),
        }
    }
}

/// Runs one `gcl instance` subcommand.
pub fn run(launcher: &Launcher, format: Format, command: InstanceCommand) -> Result<()> {
    match command {
        InstanceCommand::Create {
            name,
            minecraft,
            loader,
            loader_version,
        } => {
            let created = launcher.instances().create(
                &name,
                &minecraft,
                loader.into(),
                loader_version,
                &launcher.config().game_defaults,
            )?;
            report_one(format, &created, "created")
        }
        InstanceCommand::List => {
            let instances = launcher.instances().list()?;
            let rows: Vec<InstanceRow> = instances.iter().map(InstanceRow::from).collect();
            match format {
                Format::Json => print_json(&rows),
                Format::Text => {
                    let cells: Vec<Vec<String>> = rows
                        .iter()
                        .map(|r| {
                            vec![
                                r.slug.clone(),
                                r.name.clone(),
                                r.minecraft.clone(),
                                r.loader.clone(),
                            ]
                        })
                        .collect();
                    print_table(&["SLUG", "NAME", "MINECRAFT", "LOADER"], &cells);
                    Ok(())
                }
            }
        }
        InstanceCommand::Delete { slug, yes } => {
            if !yes {
                bail!("refusing to delete {slug} without --yes");
            }
            launcher.instances().delete(&slug)?;
            match format {
                Format::Json => print_json(&serde_json::json!({ "deleted": slug })),
                Format::Text => {
                    println!("deleted {slug}");
                    Ok(())
                }
            }
        }
        InstanceCommand::Jvm {
            slug,
            min,
            max,
            extra,
            clear_extra,
            java_path,
            clear_java_path,
            gc,
        } => {
            let previous = launcher.instances().get(&slug)?.config.jvm.clone();
            let next = InstanceJvm {
                min_mib: min.or(previous.min_mib),
                max_mib: max.or(previous.max_mib),
                java_path: if clear_java_path {
                    None
                } else {
                    java_path.or(previous.java_path)
                },
                extra_args: match (clear_extra, extra.is_empty()) {
                    (true, _) => Vec::new(),
                    (false, true) => previous.extra_args,
                    (false, false) => extra,
                },
                gc: gc.map_or(previous.gc, Into::into),
            };
            // One call: it probes the java this command is about to save, checks the preset
            // against it, and writes the whole block only once both hold. Nothing is written
            // when the check fails, so there is nothing to roll back.
            save_jvm(launcher, &slug, next)?;
            let saved = launcher.instances().get(&slug)?;
            report_jvm(format, &slug, &saved.config.jvm)
        }
        InstanceCommand::Gc { slug } => {
            let view = launcher.gc_support(&slug)?;
            report_gc(format, &view)
        }
        InstanceCommand::Rename { slug, new_name } => {
            let renamed = launcher.instances().rename(&slug, &new_name)?;
            report_one(format, &renamed, "renamed")
        }
    }
}

/// One instance's JVM settings as the CLI reports them.
#[derive(Serialize)]
struct JvmRow {
    slug: String,
    min_mib: Option<u32>,
    max_mib: Option<u32>,
    java_path: Option<String>,
    extra_args: Vec<String>,
    gc: String,
}

/// One offered preset as the CLI reports it.
#[derive(Serialize)]
struct PresetRow {
    token: String,
    label: String,
    description: String,
}

/// Saves one instance's JVM block, naming what its Java does carry when a preset is refused.
///
/// Only `Launcher::set_instance_jvm_gc` decides: it probes the java it is about to save and
/// answers `UnsupportedPreset`. This adds the list of presets that java does have, which the
/// error itself does not carry.
fn save_jvm(launcher: &Launcher, slug: &str, jvm: InstanceJvm) -> Result<()> {
    match launcher.set_instance_jvm_gc(slug, jvm) {
        Ok(_) => Ok(()),
        Err(gcl_core::Error::Java(gcl_core::java::Error::UnsupportedPreset {
            preset,
            java,
            major,
        })) => {
            let supported = launcher
                .gc_support_for_java(&java, GcPreset::Default)
                .map(|view| preset_tokens(&view.presets))
                .unwrap_or_default();
            bail!(
                "Java {major} at {} cannot run the {preset} garbage collector; supported: {supported}",
                java.display()
            )
        }
        Err(err) => Err(err.into()),
    }
}

/// The presets as one comma-separated line of the tokens `--gc` takes.
fn preset_tokens(presets: &[GcPreset]) -> String {
    presets
        .iter()
        .map(GcPreset::to_string)
        .collect::<Vec<String>>()
        .join(", ")
}

/// Prints one instance's JVM settings.
fn report_jvm(format: Format, slug: &str, jvm: &InstanceJvm) -> Result<()> {
    let row = JvmRow {
        slug: slug.to_string(),
        min_mib: jvm.min_mib,
        max_mib: jvm.max_mib,
        java_path: jvm.java_path.as_ref().map(|p| p.display().to_string()),
        extra_args: jvm.extra_args.clone(),
        gc: jvm.gc.to_string(),
    };
    match format {
        Format::Json => print_json(&row),
        Format::Text => {
            println!("jvm {slug}");
            println!("min_mib: {}", unset_or(row.min_mib));
            println!("max_mib: {}", unset_or(row.max_mib));
            println!(
                "java_path: {}",
                row.java_path.unwrap_or_else(|| "<unset>".to_string())
            );
            println!("extra_args: {}", row.extra_args.join(" "));
            println!("gc: {}", row.gc);
            Ok(())
        }
    }
}

/// A heap bound, or the word for one that falls back to `config.toml`.
fn unset_or(value: Option<u32>) -> String {
    value.map_or_else(|| "<unset>".to_string(), |v| v.to_string())
}

/// Prints the presets one instance's Java offers and which one is saved.
///
/// The saved preset comes from the view, not from `instance.toml`: `GcSupportView.saved` is
/// the preset as this Java runs it, so a plain ZGC saved before the runtime moved to Java 23
/// reads as the generational row that Java really offers.
fn report_gc(format: Format, view: &gcl_core::launcher::GcSupportView) -> Result<()> {
    let selected = view.saved;
    let supported = selected == GcPreset::Default || view.presets.contains(&selected);
    match format {
        Format::Json => print_json(&serde_json::json!({
            "java_path": view.java_path.display().to_string(),
            "label": view.label,
            "major": view.major,
            "selected": selected.to_string(),
            "selected_supported": supported,
            "presets": view.presets.iter().map(|preset| PresetRow {
                token: preset.to_string(),
                label: preset.label().to_string(),
                description: preset.description().to_string(),
            }).collect::<Vec<PresetRow>>(),
        })),
        Format::Text => {
            println!("java: {}", view.label);
            println!("path: {}", view.java_path.display());
            print_table(
                &["PRESET", "NAME", "DESCRIPTION"],
                &view
                    .presets
                    .iter()
                    .map(|preset| {
                        vec![
                            preset.to_string(),
                            preset.label().to_string(),
                            preset.description().to_string(),
                        ]
                    })
                    .collect::<Vec<Vec<String>>>(),
            );
            println!("selected: {selected}");
            if !supported {
                println!("Unavailable: {selected} — this Java was not built with it");
            }
            Ok(())
        }
    }
}

/// Prints one instance, as JSON or as a short confirmation line.
fn report_one(format: Format, instance: &gcl_core::instances::Instance, verb: &str) -> Result<()> {
    match format {
        Format::Json => print_json(&InstanceRow::from(instance)),
        Format::Text => {
            println!("{verb} {} ({})", instance.slug, instance.config.name);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_tokens_reads_as_the_words_the_gc_flag_takes() {
        let line = preset_tokens(&[GcPreset::Default, GcPreset::G1, GcPreset::ZgcGenerational]);
        assert_eq!(line, "default, g1, zgc_generational");
    }

    #[test]
    fn loader_arg_maps_onto_the_core_loader() {
        assert_eq!(Loader::from(LoaderArg::NeoForge), Loader::NeoForge);
        assert_eq!(Loader::from(LoaderArg::None), Loader::None);
    }
}
