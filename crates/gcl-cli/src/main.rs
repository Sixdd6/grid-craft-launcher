//! `gcl`: every MVP feature is reachable from here so agents can verify without a display.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gcl", version = gcl_core::VERSION, about = "GRID Craft Launcher CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Debug {
            command: DebugCommand::VerifySource { source },
        } => {
            eprintln!("verify-source {source}: not implemented");
            ExitCode::from(2)
        }
    }
}
