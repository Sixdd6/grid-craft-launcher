//! `gcl version`: list Mojang's versions and install a vanilla one.

use anyhow::Result;
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::mojang::VersionType;
use serde::Serialize;

use crate::output::{Format, print_json, print_table};

/// Subcommands under `gcl version`.
#[derive(Subcommand)]
pub enum VersionCommand {
    /// List the versions in Mojang's manifest.
    List {
        /// Also list snapshots and the old alpha and beta builds.
        #[arg(long)]
        snapshots: bool,
    },
    /// Install a vanilla version's metadata, libraries, assets, and natives.
    Install {
        /// Version id, such as 1.20.1.
        id: String,
    },
}

/// One manifest entry as the CLI reports it.
#[derive(Serialize)]
struct VersionRow {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "releaseTime")]
    release_time: String,
}

/// What an install produced.
#[derive(Serialize)]
struct InstallRow {
    id: String,
    client_jar: String,
    libraries: usize,
    asset_index: String,
    java_major: u32,
    files: usize,
}

/// Runs one `gcl version` subcommand.
pub fn run(launcher: &Launcher, format: Format, command: VersionCommand) -> Result<()> {
    match command {
        VersionCommand::List { snapshots } => list(launcher, format, snapshots),
        VersionCommand::Install { id } => install(launcher, format, &id),
    }
}

/// Prints the manifest, releases only unless `snapshots` is set.
fn list(launcher: &Launcher, format: Format, snapshots: bool) -> Result<()> {
    let manifest = launcher.list_versions()?;
    let rows: Vec<VersionRow> = manifest
        .versions
        .into_iter()
        .filter(|entry| snapshots || entry.kind == VersionType::Release)
        .map(|entry| VersionRow {
            id: entry.id,
            kind: kind_name(entry.kind),
            release_time: entry.release_time,
        })
        .collect();
    match format {
        Format::Json => print_json(&rows),
        Format::Text => {
            let cells: Vec<Vec<String>> = rows
                .iter()
                .map(|r| vec![r.id.clone(), r.kind.to_string(), r.release_time.clone()])
                .collect();
            print_table(&["ID", "TYPE", "RELEASED"], &cells);
            Ok(())
        }
    }
}

/// Installs a vanilla version and reports what landed in the cache.
fn install(launcher: &Launcher, format: Format, id: &str) -> Result<()> {
    let plan = launcher.install_version(id)?;
    let row = InstallRow {
        id: id.to_string(),
        client_jar: plan.client_jar.display().to_string(),
        libraries: plan.classpath.len(),
        asset_index: plan.asset_index_id,
        java_major: plan.java_major,
        files: plan.specs.len(),
    };
    match format {
        Format::Json => print_json(&row),
        Format::Text => {
            println!("installed {id}");
            Ok(())
        }
    }
}

/// Manifest name of a release channel.
fn kind_name(kind: VersionType) -> &'static str {
    match kind {
        VersionType::Release => "release",
        VersionType::Snapshot => "snapshot",
        VersionType::OldBeta => "old_beta",
        VersionType::OldAlpha => "old_alpha",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_names_match_the_manifest_spelling() {
        assert_eq!(kind_name(VersionType::Release), "release");
        assert_eq!(kind_name(VersionType::OldAlpha), "old_alpha");
    }
}
