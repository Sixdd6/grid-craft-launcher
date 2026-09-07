//! `gcl launch`: install what is missing, then print or run the game's command line.

use std::process::ExitCode;

use anyhow::Result;
use clap::Args;
use gcl_core::Launcher;
use gcl_core::launch::LaunchCommand;
use gcl_core::launcher::LaunchOutcome;

use crate::output::{Format, print_json};

/// Arguments of `gcl launch`.
#[derive(Args)]
pub struct LaunchArgs {
    /// Slug of the instance to launch.
    pub slug: String,
    /// Saved account to launch with, by id or name. Defaults to the active account.
    #[arg(long, value_name = "ID|NAME")]
    pub account: Option<String>,
    /// Launch as this offline player instead. The account is created if it is new.
    #[arg(long, value_name = "NAME")]
    pub offline_user: Option<String>,
    /// Print the command line instead of starting the game.
    #[arg(long)]
    pub dry_run: bool,
}

/// What a failed sign-in tells the user to do instead.
const OFFLINE_HINT: &str = "hint: use --offline-user <name> to play offline";

/// Runs `gcl launch`. Returns the exit code the CLI itself should end with.
///
/// An account failure is answered here rather than by `main`: the message is followed by
/// [`OFFLINE_HINT`], so a user whose Microsoft token cannot be refreshed is told how to
/// play anyway. Every other failure is returned and reported by `main`.
pub fn run(launcher: &Launcher, format: Format, args: LaunchArgs) -> Result<ExitCode> {
    let outcome = match launcher.launch_instance(
        &args.slug,
        args.account.as_deref(),
        args.offline_user.as_deref(),
        args.dry_run,
    ) {
        Ok(outcome) => outcome,
        Err(err) if is_auth_error(&err) => {
            eprintln!("error: {err}");
            eprintln!("{OFFLINE_HINT}");
            return Ok(ExitCode::FAILURE);
        }
        Err(err) => return Err(err.into()),
    };
    match outcome {
        LaunchOutcome::DryRun(cmd) => {
            match format {
                // `LaunchCommand` serializes its arguments in the redacted form.
                Format::Json => print_json(&cmd)?,
                Format::Text => print_command(&cmd.redacted()),
            }
            Ok(ExitCode::SUCCESS)
        }
        LaunchOutcome::Exited {
            code,
            log_path,
            hint,
        } => {
            match format {
                Format::Json => print_json(&serde_json::json!({
                    "exit_code": code,
                    "log": log_path.display().to_string(),
                    "hint": hint,
                }))?,
                Format::Text => {
                    println!("exited {code} (log: {})", log_path.display());
                    if let Some(hint) = &hint {
                        println!("hint: {hint}");
                    }
                }
            }
            Ok(ExitCode::from(exit_code(code)))
        }
    }
}

/// Whether a launch failure came from the account: no account, or a refused sign-in.
fn is_auth_error(err: &gcl_core::Error) -> bool {
    matches!(err, gcl_core::Error::Auth(_))
}

/// Prints the program, the working directory, and one argument per line.
fn print_command(cmd: &LaunchCommand) {
    println!("program: {}", cmd.program.display());
    println!("cwd: {}", cmd.cwd.display());
    println!("args:");
    for arg in &cmd.args {
        println!("  {arg}");
    }
}

/// The process exit code for a game exit code.
///
/// A code outside `0..=255`, which is what a signal-killed process reports, becomes 1: it
/// failed, and reporting 0 would call a killed game a clean run.
fn exit_code(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exit_code_outside_a_byte_becomes_one() {
        assert_eq!(exit_code(0), 0);
        assert_eq!(exit_code(3), 3);
        assert_eq!(exit_code(-1), 1);
        assert_eq!(exit_code(256), 1);
    }
}
