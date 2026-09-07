//! Desktop UI entry point. Core logic stays in `gcl-core`; this crate renders and forwards events.

slint::include_modules!();

mod app;
mod bridge;
mod events;
mod keys;
// The converters land with the screens that use them, in tasks 3 to 7.
#[allow(dead_code)]
mod models;
mod screens;
mod state;
mod toasts;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use gcl_core::Launcher;
use gcl_core::events::Event;
use slint::ComponentHandle;

/// How long `--smoke` keeps the window open before it quits the event loop.
const SMOKE_DELAY: Duration = Duration::from_millis(500);

/// What the command line asked for.
#[derive(Debug)]
struct Args {
    /// Open the window, run one synthetic task, and quit. For CI and a first-run check.
    smoke: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Parses the arguments, opens the launcher, builds the window, and runs the event loop.
fn run() -> Result<(), String> {
    let args = match parse_args(std::env::args().skip(1))? {
        Some(args) => args,
        // `--help` or `--version` printed already.
        None => return Ok(()),
    };

    // A failure here has no window to show it in yet; the error window is later polish.
    let (launcher, rx) =
        Launcher::new(None).map_err(|err| bridge::error_chain(&err) + " (could not start)")?;
    let launcher = Arc::new(launcher);

    let window = app::build(Arc::clone(&launcher), rx).map_err(|err| err.to_string())?;

    if args.smoke {
        smoke(&launcher);
        slint::Timer::single_shot(SMOKE_DELAY, || {
            let _ = slint::quit_event_loop();
        });
    }

    window.run().map_err(|err| err.to_string())
}

/// Sends synthetic events through the sink, so a smoke run exercises the forwarder.
///
/// Three of them, one per thing the shell has to draw: a task that finishes, a task that
/// fails so a row shows in the danger color, and a warning so the toast stack is filled.
fn smoke(launcher: &Launcher) {
    let sink = launcher.events();
    let _ = sink.send(Event::TaskStarted {
        id: 0,
        label: "smoke test".into(),
        total_bytes: Some(1024),
    });
    let _ = sink.send(Event::TaskFinished { id: 0 });
    let _ = sink.send(Event::TaskStarted {
        id: 1,
        label: "smoke failure".into(),
        total_bytes: Some(2048),
    });
    let _ = sink.send(Event::TaskFailed {
        id: 1,
        error: "synthetic failure".into(),
    });
    let _ = sink.send(Event::Warning("synthetic warning".into()));
}

/// Reads the command line. `Ok(None)` means the caller asked for help or the version.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut parsed = Args { smoke: false };
    for arg in args {
        match arg.as_str() {
            // `just run-ui -- --smoke` passes the separator through; ignore it.
            "--" => continue,
            "--smoke" => parsed.smoke = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("grid-craft-launcher {}", gcl_core::VERSION);
                return Ok(None);
            }
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    Ok(Some(parsed))
}

/// The `--help` text.
const USAGE: &str = "\
GRID Craft Launcher

Usage: grid-craft-launcher [OPTIONS]

Options:
  --smoke        Open the window, run one synthetic task, then quit
  -h, --help     Print this help
  -V, --version  Print the version";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_reads_smoke() {
        let args = parse_args(["--smoke".to_string()].into_iter())
            .expect("parses")
            .expect("runs");
        assert!(args.smoke);
    }

    #[test]
    fn parse_args_defaults_to_no_smoke() {
        let args = parse_args(std::iter::empty())
            .expect("parses")
            .expect("runs");
        assert!(!args.smoke);
    }

    #[test]
    fn parse_args_ignores_the_separator() {
        let args = parse_args(["--".to_string(), "--smoke".to_string()].into_iter())
            .expect("parses")
            .expect("runs");
        assert!(args.smoke);
    }

    #[test]
    fn parse_args_rejects_unknown() {
        let err = parse_args(["--nope".to_string()].into_iter()).expect_err("rejected");
        assert!(err.contains("--nope"));
    }
}
