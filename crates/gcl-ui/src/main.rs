//! The launcher binary: parse the command line, start logging, build the window, run it.
//!
//! Everything else lives in the `gcl_ui` library, so integration tests can build the same
//! window this binary does.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gcl_core::Launcher;
use gcl_core::events::Event;
use gcl_ui::{AppWindow, app, bridge, logging};
use slint::{ComponentHandle, RenderingState};

/// How long `--smoke` keeps the window open before it quits the event loop.
const SMOKE_DELAY: Duration = Duration::from_millis(500);

/// How long `--screenshot` waits before it grabs the window.
///
/// The first frame is drawn before this fires, so the shell, the rail, and the instances list
/// are all on screen by the time the snapshot is taken.
const SHOT_DELAY: Duration = Duration::from_millis(700);

/// What the command line asked for.
#[derive(Debug)]
struct Args {
    /// Open the window, run one synthetic task, and quit. For CI and a first-run check.
    smoke: bool,
    /// Open the window, write a PNG of it to this path, and quit.
    screenshot: Option<PathBuf>,
}

fn main() -> ExitCode {
    // First of all, before any thread starts: `time` reads the local UTC offset only from a
    // single-threaded process, and game log timestamps are shown in local time.
    gcl_core::launch::init_local_offset();

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

    // Held to the end of `run`, so the log writer's worker thread outlives the event loop.
    let _log_guard = logging::init(launcher.root());
    tracing::info!(version = gcl_core::VERSION, "gui start");

    let window = app::build(Arc::clone(&launcher), rx).map_err(|err| err.to_string())?;

    if args.smoke {
        smoke(&launcher);
        slint::Timer::single_shot(SMOKE_DELAY, || {
            let _ = slint::quit_event_loop();
        });
    }

    if let Some(path) = args.screenshot {
        arm_screenshot(&window, path)?;
    }

    window.run().map_err(|err| err.to_string())
}

/// Arranges for one PNG of the window to be written, then quits the event loop.
///
/// The snapshot has to be taken inside the renderer's `AfterRendering` callback. The FemtoVG
/// renderer reads the OpenGL back buffer, and that buffer only holds the frame between the
/// draw and the buffer swap; asking for it from a plain timer gives a blank image. So the
/// timer only raises a flag and asks for a redraw, and the callback below does the work on
/// the frame that redraw produces.
fn arm_screenshot(window: &AppWindow, path: PathBuf) -> Result<(), String> {
    let wanted = Arc::new(AtomicBool::new(false));

    let weak = window.as_weak();
    let flag = Arc::clone(&wanted);
    window
        .window()
        .set_rendering_notifier(move |state, _api| {
            if !matches!(state, RenderingState::AfterRendering)
                || !flag.swap(false, Ordering::SeqCst)
            {
                return;
            }
            if let Some(window) = weak.upgrade()
                && let Err(message) = screenshot(&window, &path)
            {
                eprintln!("error: {message}");
            }
            let _ = slint::quit_event_loop();
        })
        .map_err(|err| format!("could not watch the renderer: {err}"))?;

    let weak = window.as_weak();
    slint::Timer::single_shot(SHOT_DELAY, move || {
        wanted.store(true, Ordering::SeqCst);
        if let Some(window) = weak.upgrade() {
            window.window().request_redraw();
        }
    });
    Ok(())
}

/// Writes a PNG of the window's current frame to `path`.
fn screenshot(window: &AppWindow, path: &std::path::Path) -> Result<(), String> {
    let buffer = window
        .window()
        .take_snapshot()
        .map_err(|err| format!("could not take a snapshot: {err}"))?;
    let (width, height) = (buffer.width(), buffer.height());
    let file = std::fs::File::create(path)
        .map_err(|err| format!("could not create {}: {err}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|err| format!("could not write the PNG header: {err}"))?;
    writer
        .write_image_data(buffer.as_bytes())
        .map_err(|err| format!("could not write the PNG data: {err}"))?;
    tracing::info!(path = %path.display(), width, height, "screenshot written");
    Ok(())
}

/// Sends synthetic events through the sink, so a smoke run exercises the forwarder.
///
/// Three of them, one per thing the shell has to draw: a task that finishes, a task that
/// fails so a row shows in the danger color, and a warning so the toast stack is filled. The
/// ids come from the same allocator real work uses, so a synthetic row can never land on a
/// real task's row.
fn smoke(launcher: &Launcher) {
    let sink = launcher.events();
    let done = gcl_core::events::next_task_id();
    let _ = sink.send(Event::TaskStarted {
        id: done,
        label: "smoke test".into(),
        total_bytes: Some(1024),
    });
    let _ = sink.send(Event::TaskFinished { id: done });

    let failed = gcl_core::events::next_task_id();
    let _ = sink.send(Event::TaskStarted {
        id: failed,
        label: "smoke failure".into(),
        total_bytes: Some(2048),
    });
    let _ = sink.send(Event::TaskFailed {
        id: failed,
        error: "synthetic failure".into(),
    });
    let _ = sink.send(Event::Warning("synthetic warning".into()));
}

/// Reads the command line. `Ok(None)` means the caller asked for help or the version.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut parsed = Args {
        smoke: false,
        screenshot: None,
    };
    let mut want_path = false;
    for arg in args {
        // `just run-ui -- --smoke` passes the separator through; ignore it. This runs before
        // the path branch on purpose: `--screenshot -- shot.png` must take `shot.png` as the
        // path, not the separator.
        if arg == "--" {
            continue;
        }
        if want_path {
            parsed.screenshot = Some(PathBuf::from(arg));
            want_path = false;
            continue;
        }
        match arg.as_str() {
            "--smoke" => parsed.smoke = true,
            "--screenshot" => want_path = true,
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
    if want_path {
        return Err(format!("`--screenshot` needs a path\n\n{USAGE}"));
    }
    Ok(Some(parsed))
}

/// The `--help` text.
const USAGE: &str = "\
GRID Craft Launcher

Usage: grid-craft-launcher [OPTIONS]

Options:
  --smoke              Open the window, run one synthetic task, then quit
  --screenshot <PATH>  Open the window, write a PNG of it to PATH, then quit
  -h, --help           Print this help
  -V, --version        Print the version";

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
        assert!(args.screenshot.is_none());
    }

    #[test]
    fn parse_args_ignores_the_separator() {
        let args = parse_args(["--".to_string(), "--smoke".to_string()].into_iter())
            .expect("parses")
            .expect("runs");
        assert!(args.smoke);
    }

    #[test]
    fn parse_args_reads_the_screenshot_path() {
        let args = parse_args(["--screenshot".to_string(), "/tmp/a.png".to_string()].into_iter())
            .expect("parses")
            .expect("runs");
        assert_eq!(
            args.screenshot.as_deref(),
            Some(std::path::Path::new("/tmp/a.png"))
        );
    }

    #[test]
    fn parse_args_skips_the_separator_before_a_screenshot_path() {
        let args = parse_args(
            [
                "--screenshot".to_string(),
                "--".to_string(),
                "/tmp/a.png".to_string(),
            ]
            .into_iter(),
        )
        .expect("parses")
        .expect("runs");
        assert_eq!(
            args.screenshot.as_deref(),
            Some(std::path::Path::new("/tmp/a.png"))
        );
    }

    #[test]
    fn parse_args_rejects_a_screenshot_whose_only_argument_is_the_separator() {
        let err = parse_args(["--screenshot".to_string(), "--".to_string()].into_iter())
            .expect_err("rejected");
        assert!(err.contains("--screenshot"));
    }

    #[test]
    fn parse_args_rejects_a_screenshot_without_a_path() {
        let err = parse_args(["--screenshot".to_string()].into_iter()).expect_err("rejected");
        assert!(err.contains("--screenshot"));
    }

    #[test]
    fn parse_args_rejects_unknown() {
        let err = parse_args(["--nope".to_string()].into_iter()).expect_err("rejected");
        assert!(err.contains("--nope"));
    }
}
