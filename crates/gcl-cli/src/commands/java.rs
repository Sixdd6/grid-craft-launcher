//! `gcl java`: list the runtimes on this machine and ensure one of a given major version.

use anyhow::Result;
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::java::{JavaInstall, JavaSource};
use serde::Serialize;

use crate::output::{Format, print_json, print_table};

/// Subcommands under `gcl java`.
#[derive(Subcommand)]
pub enum JavaCommand {
    /// List every Java runtime found on this machine.
    List,
    /// Return a runtime of the given major version, installing Mojang's if none is present.
    Ensure {
        /// Major version, such as 17 or 21.
        major: u32,
    },
}

/// One Java runtime as the CLI reports it.
#[derive(Serialize)]
struct JavaRow {
    path: String,
    major: u32,
    version: String,
    vendor: String,
    source: &'static str,
}

impl From<&JavaInstall> for JavaRow {
    fn from(install: &JavaInstall) -> JavaRow {
        JavaRow {
            path: install.path.display().to_string(),
            major: install.major,
            version: install.version.clone(),
            vendor: install.vendor.clone(),
            source: source_name(install.source),
        }
    }
}

/// Runs one `gcl java` subcommand.
pub fn run(launcher: &Launcher, format: Format, command: JavaCommand) -> Result<()> {
    match command {
        JavaCommand::List => {
            let found = launcher.detect_java();
            let rows: Vec<JavaRow> = found.iter().map(JavaRow::from).collect();
            match format {
                Format::Json => print_json(&rows),
                Format::Text => {
                    let cells: Vec<Vec<String>> = rows
                        .iter()
                        .map(|r| {
                            vec![
                                r.path.clone(),
                                r.major.to_string(),
                                r.version.clone(),
                                r.vendor.clone(),
                            ]
                        })
                        .collect();
                    print_table(&["PATH", "MAJOR", "VERSION", "VENDOR"], &cells);
                    Ok(())
                }
            }
        }
        JavaCommand::Ensure { major } => {
            let install = launcher.ensure_java(major)?;
            match format {
                Format::Json => print_json(&JavaRow::from(&install)),
                Format::Text => {
                    println!("{}", install.path.display());
                    Ok(())
                }
            }
        }
    }
}

/// Lowercase name of where a runtime was found.
fn source_name(source: JavaSource) -> &'static str {
    match source {
        JavaSource::Path => "path",
        JavaSource::JavaHome => "java_home",
        JavaSource::Mojang => "mojang",
        JavaSource::Manual => "manual",
        JavaSource::System => "system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_are_lowercase() {
        assert_eq!(source_name(JavaSource::JavaHome), "java_home");
        assert_eq!(source_name(JavaSource::Mojang), "mojang");
    }
}
