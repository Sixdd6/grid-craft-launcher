//! `gcl loader`: list the builds a Minecraft version has, and install an instance's loader.

use anyhow::Result;
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::instances::model::Loader;
use serde::Serialize;

use crate::commands::instance::LoaderArg;
use crate::output::{Format, print_json, print_table};

/// Subcommands under `gcl loader`.
#[derive(Subcommand)]
pub enum LoaderCommand {
    /// List the loader builds published for one Minecraft version, newest first.
    List {
        /// Minecraft version id, such as 1.20.1.
        minecraft: String,
        /// Which loader to list.
        #[arg(long, value_enum)]
        loader: LoaderArg,
    },
    /// Install the instance's loader and print the version id a launch resolves.
    Install {
        /// Slug of the instance.
        slug: String,
    },
}

/// One loader build as the CLI reports it.
#[derive(Serialize)]
struct LoaderRow {
    version: String,
    stable: bool,
    recommended: bool,
}

/// Runs one `gcl loader` subcommand.
pub fn run(launcher: &Launcher, format: Format, command: LoaderCommand) -> Result<()> {
    match command {
        LoaderCommand::List { minecraft, loader } => {
            let loader: Loader = loader.into();
            let versions = launcher.list_loader_versions(loader, &minecraft)?;
            let rows: Vec<LoaderRow> = versions
                .into_iter()
                .map(|v| LoaderRow {
                    version: v.version,
                    stable: v.stable,
                    recommended: v.recommended,
                })
                .collect();
            match format {
                Format::Json => print_json(&rows),
                Format::Text => {
                    let cells: Vec<Vec<String>> = rows
                        .iter()
                        .map(|r| {
                            vec![
                                r.version.clone(),
                                r.stable.to_string(),
                                r.recommended.to_string(),
                            ]
                        })
                        .collect();
                    print_table(&["VERSION", "STABLE", "RECOMMENDED"], &cells);
                    Ok(())
                }
            }
        }
        LoaderCommand::Install { slug } => {
            let id = launcher.install_loader(&slug)?;
            match format {
                Format::Json => print_json(&serde_json::json!({ "version_id": id })),
                Format::Text => {
                    println!("{id}");
                    Ok(())
                }
            }
        }
    }
}
