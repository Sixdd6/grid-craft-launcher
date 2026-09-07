//! The one launch path the GUI has.
//!
//! Both the instances list and the instance detail screen start a game through [`launch`], so
//! a launch behaves the same whichever button began it: the same running flag, the same log
//! tail, the same offline-name prompt when no account is set, and the same refresh at the end.
//!
//! Two threads are involved. One waits for the game and posts its outcome back; the other
//! tails the game's log file into `InstanceState.game_log` while it runs. Everything that
//! touches a Slint property runs inside `upgrade_in_event_loop`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gcl_core::launcher::LaunchOutcome;
use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};

use crate::bridge::{Bridge, warn};
use crate::state::RunState;
use crate::{App, AppWindow, InstanceState, InstancesState, LogLine, Screen};

/// How often the log tail re-reads the running game's log file.
const TAIL_INTERVAL: Duration = Duration::from_millis(250);

/// Most game log lines kept in the model. Older lines are dropped from the front.
const GAME_LOG_LIMIT: usize = 2000;

/// The prompt mode that means "this name starts a launch". `screens::instance` reads it back.
pub const PROMPT_OFFLINE: &str = "offline";

/// Starts `slug`'s game and follows it to its end.
///
/// `offline_user` is the name typed into the prompt, and is `None` for the first attempt: a
/// launch with no account comes back as [`gcl_core::auth::Error::NoAccount`], and that is what
/// opens the prompt. A slug whose game is already up is dropped, so a stale row cannot start a
/// second copy.
pub fn launch(bridge: &Bridge, run: &RunState, slug: String, offline_user: Option<String>) {
    if !run.start(&slug) {
        return;
    }
    if let Some(window) = bridge.weak().upgrade() {
        mark_started(&window, run, &slug);
    }

    let launcher = Arc::clone(bridge.launcher());
    let weak = bridge.weak().clone();
    // The launch thread reports the outcome, so it needs the bridge: it is what knows the
    // log path the error dialog names.
    let reporter = bridge.clone();
    let run = run.clone();
    std::thread::spawn(move || {
        let started = launcher.launch_instance_async(&slug, None, offline_user.as_deref());
        let outcome = match started {
            Ok(running) => {
                let stop = Arc::new(AtomicBool::new(false));
                let tail = start_tail(
                    weak.clone(),
                    slug.clone(),
                    running.log_path.clone(),
                    Arc::clone(&stop),
                );
                let outcome = running.wait_blocking(&launcher);
                stop.store(true, Ordering::Relaxed);
                let _ = tail.join();
                outcome
            }
            Err(err) => Err(err),
        };
        run.finish(&slug);
        finish(&reporter, &run, &slug, outcome);
    });
}

/// Shows the game as running on both screens and empties the log the tail is about to fill.
///
/// Runs on the UI thread, before the launch thread starts.
fn mark_started(window: &AppWindow, run: &RunState, slug: &str) {
    mark_rows(window, run);
    if shows(window, slug) {
        let state = window.global::<InstanceState>();
        state.set_running(true);
        state.set_status_text("Starting…".into());
        state.set_game_log(ModelRc::new(VecModel::from(Vec::<LogLine>::new())));
    }
}

/// Reports what the game did and refreshes both screens. Runs on the launch thread.
fn finish(
    bridge: &Bridge,
    run: &RunState,
    slug: &str,
    outcome: Result<LaunchOutcome, gcl_core::Error>,
) {
    let run = run.clone();
    let slug = slug.to_string();
    let reporter = bridge.clone();
    let _ = bridge.weak().upgrade_in_event_loop(move |window| {
        match outcome {
            // A stop is what the user asked for, so it is said once, quietly: the exit code
            // is non-zero on purpose and no warning toast belongs to it.
            Ok(LaunchOutcome::Exited { stopped: true, .. }) => status(&window, &slug, "Stopped"),
            Ok(LaunchOutcome::Exited { code, hint, .. }) if code != 0 => {
                let hint = hint.unwrap_or_else(|| "see the instance log".to_string());
                let text = format!("Minecraft exited with code {code}: {hint}");
                status(&window, &slug, &text);
                warn(&window, &text);
            }
            Ok(_) => status(&window, &slug, "Minecraft exited"),
            Err(err) if is_no_account(&err) => {
                // The prompt is the answer to this error, so it replaces the error dialog.
                // The prompt belongs to the detail screen, so the shell moves there first:
                // a launch begun from the list has no prompt of its own.
                let app = window.global::<App>();
                app.set_current_slug(slug.as_str().into());
                app.set_screen(Screen::Instance);
                let state = window.global::<InstanceState>();
                state.set_status_text("No account: choose an offline name".into());
                state.set_prompt_mode(PROMPT_OFFLINE.into());
                state.set_prompt_title("Play offline".into());
                state.set_prompt_label("Player name".into());
                state.set_prompt_accept("Launch".into());
                state.set_prompt_value("".into());
                state.set_prompt_open(true);
            }
            Err(err) => {
                status(&window, &slug, "Launch failed");
                reporter.show_error(&window, "Launch", &err);
            }
        }
        // The running flag and the last-launched stamp both changed.
        mark_rows(&window, &run);
        window.global::<InstancesState>().invoke_refresh();
        // `InstanceState` belongs to whichever instance the detail screen shows. Writing its
        // running flag for another slug would mark the wrong game, so both writes are guarded
        // the same way `mark_started` guards its own.
        if shows(&window, &slug) {
            let state = window.global::<InstanceState>();
            state.set_running(run.is_running(&slug));
            state.invoke_load(slug.as_str().into());
        }
    });
}

/// Starts the thread that copies new lines of the game's log into the detail screen.
///
/// The game writes both of its output streams to one file, so tailing that file is how the
/// UI sees the game without core growing a second event kind. Lines are only appended while
/// the detail screen shows `slug`: another instance's log must not land in this one's view.
fn start_tail(
    weak: Weak<AppWindow>,
    slug: String,
    path: PathBuf,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut read = 0usize;
        loop {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                let (lines, next) = tail_new_lines(read, &contents);
                read = next;
                let slug = slug.clone();
                if !lines.is_empty()
                    && weak
                        .upgrade_in_event_loop(move |window| {
                            if shows(&window, &slug) {
                                append_game_log(&window, &lines);
                            }
                        })
                        .is_err()
                {
                    return;
                }
            }
            // Checked after the read, so the last lines of a finished game still land.
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(TAIL_INTERVAL);
        }
    })
}

/// Appends log lines, dropping the oldest once the model is full. Runs on the UI thread.
fn append_game_log(window: &AppWindow, lines: &[String]) {
    let state = window.global::<InstanceState>();
    let mut log: Vec<LogLine> = state.get_game_log().iter().collect();
    log.extend(lines.iter().map(|text| LogLine {
        level: "info".into(),
        text: text.as_str().into(),
    }));
    if log.len() > GAME_LOG_LIMIT {
        log.drain(..log.len() - GAME_LOG_LIMIT);
    }
    state.set_game_log(ModelRc::new(VecModel::from(log)));
}

/// Rewrites the `running` flag on every list row from the live set. Runs on the UI thread.
fn mark_rows(window: &AppWindow, run: &RunState) {
    let live = run.snapshot();
    let state = window.global::<InstancesState>();
    let rows: Vec<crate::InstanceRow> = state
        .get_all_rows()
        .iter()
        .map(|mut row| {
            row.running = live.contains(row.slug.as_str());
            row
        })
        .collect();
    state.set_all_rows(ModelRc::new(VecModel::from(rows.clone())));
    let filter = state.get_filter().to_string();
    state.set_rows(ModelRc::new(VecModel::from(
        crate::screens::instances::filter_rows(&rows, &filter),
    )));
}

/// Puts one line on the detail screen's status text, if it is showing this instance.
fn status(window: &AppWindow, slug: &str, text: &str) {
    if shows(window, slug) {
        window
            .global::<InstanceState>()
            .set_status_text(text.into());
    }
}

/// Whether the detail screen is up and showing `slug`.
fn shows(window: &AppWindow, slug: &str) -> bool {
    let app = window.global::<App>();
    app.get_screen() == Screen::Instance && app.get_current_slug().as_str() == slug
}

/// The complete lines `contents` grew since `prev_len` bytes, and the new byte count.
///
/// A partial last line is left for the next call, so a line is only shown once it is whole. A
/// file that shrank, or an offset that is not a character boundary, is read again from the
/// start: a truncated log is rare, and re-reading it is better than losing it.
pub fn tail_new_lines(prev_len: usize, contents: &str) -> (Vec<String>, usize) {
    let from = if prev_len <= contents.len() && contents.is_char_boundary(prev_len) {
        prev_len
    } else {
        0
    };
    let fresh = &contents[from..];
    let complete = match fresh.rfind('\n') {
        Some(at) => at + 1,
        None => return (Vec::new(), from),
    };
    let lines = fresh[..complete]
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    (lines, from + complete)
}

/// Whether a launch failed only because no account is selected.
///
/// The offline-name prompt is the answer to this one error, so it is told apart from every
/// other launch failure, which opens the error dialog instead.
pub fn is_no_account(err: &gcl_core::Error) -> bool {
    matches!(err, gcl_core::Error::Auth(gcl_core::auth::Error::NoAccount))
}

#[cfg(test)]
mod tests {
    use super::{is_no_account, tail_new_lines};

    #[test]
    fn tail_new_lines_returns_only_whole_lines() {
        let (lines, read) = tail_new_lines(0, "one\ntwo\nthr");
        assert_eq!(lines, vec!["one".to_string(), "two".to_string()]);
        assert_eq!(read, 8, "the partial third line is left for the next call");

        let (lines, read) = tail_new_lines(read, "one\ntwo\nthree\n");
        assert_eq!(lines, vec!["three".to_string()]);
        assert_eq!(read, 14);

        let (lines, read) = tail_new_lines(read, "one\ntwo\nthree\n");
        assert!(lines.is_empty(), "nothing was appended");
        assert_eq!(read, 14);
    }

    #[test]
    fn tail_new_lines_strips_carriage_returns_and_restarts_on_a_truncated_file() {
        let (lines, _) = tail_new_lines(0, "one\r\ntwo\r\n");
        assert_eq!(lines, vec!["one".to_string(), "two".to_string()]);

        // The file was replaced by a shorter one, so the offset no longer means anything.
        let (lines, read) = tail_new_lines(500, "fresh\n");
        assert_eq!(lines, vec!["fresh".to_string()]);
        assert_eq!(read, 6);
    }

    #[test]
    fn tail_new_lines_reads_from_the_start_when_the_offset_splits_a_character() {
        // "é" is two bytes, so an offset of 1 is inside it.
        let (lines, read) = tail_new_lines(1, "é\n");
        assert_eq!(lines, vec!["é".to_string()]);
        assert_eq!(read, 3);
    }

    #[test]
    fn only_a_missing_account_opens_the_offline_prompt() {
        assert!(is_no_account(&gcl_core::Error::Auth(
            gcl_core::auth::Error::NoAccount
        )));
        assert!(!is_no_account(&gcl_core::Error::Auth(
            gcl_core::auth::Error::NotFound("steve".to_string())
        )));
        assert!(!is_no_account(&std::io::Error::other("boom").into()));
    }
}
