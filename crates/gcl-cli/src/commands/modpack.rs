//! `gcl modpack`: import a pack from a source, or from an archive on disk, as an instance.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::modpacks::ImportOutcome;

use crate::commands::content::{EXIT_MANUAL_PENDING, resolve_source};
use crate::output::{Format, print_json};

/// Subcommands under `gcl modpack`.
#[derive(Subcommand)]
pub enum ModpackCommand {
    /// Download a modpack from a source and import it as a new instance.
    Install {
        /// Source to fetch the pack from: modrinth or curseforge.
        #[arg(long)]
        source: String,
        /// Project id or slug of the pack.
        #[arg(long, value_name = "ID|SLUG")]
        project: String,
        /// Version id to pin. The newest release wins without one.
        #[arg(long, value_name = "ID")]
        version: Option<String>,
        /// Display name of the new instance. The pack's own name is the default.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// Import a modpack archive that is already on disk.
    InstallFile {
        /// Path of the `.mrpack` or CurseForge pack zip.
        path: PathBuf,
        /// Display name of the new instance. The pack's own name is the default.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

/// Runs one `gcl modpack` subcommand. Returns the exit code the CLI should end with.
pub fn run(launcher: &Launcher, format: Format, command: ModpackCommand) -> Result<ExitCode> {
    let outcome = match command {
        ModpackCommand::Install {
            source,
            project,
            version,
            name,
        } => {
            let id = resolve_source(launcher, &source)?;
            launcher.import_modpack(id, &project, version.as_deref(), name)?
        }
        ModpackCommand::InstallFile { path, name } => launcher.import_modpack_file(&path, name)?,
    };
    report(format, &outcome)
}

/// Prints the new instance and returns [`EXIT_MANUAL_PENDING`] when a hand download remains.
fn report(format: Format, outcome: &ImportOutcome) -> Result<ExitCode> {
    match format {
        Format::Json => print_json(&serde_json::json!({
            "slug": outcome.instance.slug,
            "name": outcome.instance.config.name,
            "minecraft": outcome.instance.config.minecraft,
            "loader": outcome.instance.config.loader.to_string(),
            "installed": outcome.installed,
            "manual": outcome
                .manual
                .iter()
                .map(|manual| serde_json::json!({
                    "file_name": manual.file_name,
                    "page_url": manual.page_url,
                    "project_id": manual.project_id,
                }))
                .collect::<Vec<_>>(),
        }))?,
        Format::Text => {
            println!(
                "installed {} into {} ({} files)",
                outcome.instance.config.name, outcome.instance.slug, outcome.installed
            );
            for manual in &outcome.manual {
                println!(
                    "manual download {} -> {}",
                    manual.file_name, manual.page_url
                );
            }
        }
    }
    if outcome.manual.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(EXIT_MANUAL_PENDING))
    }
}
