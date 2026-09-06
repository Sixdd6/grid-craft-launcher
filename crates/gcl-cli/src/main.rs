//! `gcl`: every MVP feature is reachable from here so agents can verify without a display.

mod commands;
mod output;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gcl_core::Launcher;

use commands::config::ConfigCommand;
use commands::instance::InstanceCommand;
use commands::java::JavaCommand;
use commands::version::VersionCommand;
use output::Format;

#[derive(Parser)]
#[command(name = "gcl", version = gcl_core::VERSION, about = "GRID Craft Launcher CLI")]
struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Use this directory as the app root instead of the platform data directory.
    #[arg(long, global = true, value_name = "PATH")]
    root: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Mojang versions: list them, install one.
    Version {
        #[command(subcommand)]
        command: VersionCommand,
    },
    /// Instances: create, list, rename, delete.
    Instance {
        #[command(subcommand)]
        command: InstanceCommand,
    },
    /// Java runtimes: list them, ensure one.
    Java {
        #[command(subcommand)]
        command: JavaCommand,
    },
    /// Launcher config: show it, change the root or the JVM defaults.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Developer checks that talk to live services.
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
}

#[derive(Subcommand)]
enum DebugCommand {
    /// Fetch live responses from a source and run them through our parsers.
    VerifySource {
        /// One of: mojang, fabric, quilt, forge, neoforge, modrinth, curseforge
        source: String,
    },
}

/// Exit code for a source `verify-source` cannot check yet.
const EXIT_NOT_IMPLEMENTED: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();

    // A source we cannot verify yet answers before a launcher, and so before any app root
    // is created.
    if let Command::Debug {
        command: DebugCommand::VerifySource { source },
    } = &cli.command
        && !commands::debug::is_implemented(source)
    {
        eprintln!("verify-source {source}: not implemented");
        return ExitCode::from(EXIT_NOT_IMPLEMENTED);
    }

    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Builds the launcher, runs the command, and joins the event printer.
fn run(cli: Cli) -> Result<ExitCode> {
    let format = Format::from_flag(cli.json);
    let (mut launcher, events) = Launcher::new(cli.root)?;
    let printer = output::spawn_event_printer(events, format);
    let result = dispatch(&mut launcher, format, cli.command);
    // Dropping the launcher closes the event sender, which ends the printer thread.
    drop(launcher);
    let _ = printer.join();
    result
}

/// Sends one parsed command to its module.
fn dispatch(launcher: &mut Launcher, format: Format, command: Command) -> Result<ExitCode> {
    match command {
        Command::Version { command } => {
            commands::version::run(launcher, format, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Instance { command } => {
            commands::instance::run(launcher, format, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Java { command } => {
            commands::java::run(launcher, format, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Config { command } => {
            commands::config::run(launcher, format, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Debug {
            command: DebugCommand::VerifySource { source },
        } => {
            // `main` has already turned away every source we cannot verify.
            let passed = commands::debug::verify_mojang(launcher)?;
            if passed {
                Ok(ExitCode::SUCCESS)
            } else {
                eprintln!("verify-source {source}: one or more endpoints failed");
                Ok(ExitCode::FAILURE)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_parse_after_the_subcommand() {
        let cli = Cli::parse_from(["gcl", "instance", "list", "--json"]);
        assert!(cli.json);
    }
}
