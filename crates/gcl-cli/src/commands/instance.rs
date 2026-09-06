//! `gcl instance`: create, list, rename, and delete instances.

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use gcl_core::Launcher;
use gcl_core::instances::model::Loader;
use serde::Serialize;

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
        InstanceCommand::Rename { slug, new_name } => {
            let renamed = launcher.instances().rename(&slug, &new_name)?;
            report_one(format, &renamed, "renamed")
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
    fn loader_arg_maps_onto_the_core_loader() {
        assert_eq!(Loader::from(LoaderArg::NeoForge), Loader::NeoForge);
        assert_eq!(Loader::from(LoaderArg::None), Loader::None);
    }
}
