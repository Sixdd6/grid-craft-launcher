//! Forwards core [`Event`]s onto the UI thread.
//!
//! One thread owns the event receiver. It blocks for the first event of a batch, collects
//! everything that arrives within [`BATCH`], and posts one closure to the UI thread. The
//! task list and the log live in the Slint models, so the UI thread is their only owner.
//!
//! Finished rows are not dropped by the batch that finished them: each one is stamped in
//! [`AGES`] and the one-second timer in `src/app.rs` removes it [`KEEP_DONE`] after its own
//! end, or [`FAILED_TTL`] when it failed. That map is a `thread_local!` because both writers,
//! this module's `push` and that timer, already run on the UI thread.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gcl_core::events::{Event, LogLevel};
use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::models::format_bytes;
use crate::toasts;
use crate::{App, AppWindow, LogLine, TaskRow};

/// How long events are collected before a batch is posted to the UI thread.
const BATCH: Duration = Duration::from_millis(50);

/// How long a task that finished cleanly stays in the list after it ended.
pub const KEEP_DONE: Duration = Duration::from_secs(5);

/// How long a task that failed stays in the list.
///
/// Six times [`KEEP_DONE`], because a failure is the one row a user has to read. The panel
/// opens itself when a task fails, so the row is on screen for that whole time.
pub const FAILED_TTL: Duration = Duration::from_secs(30);

/// Most log lines kept in memory. Older lines are dropped from the front.
pub const LOG_LIMIT: usize = 2000;

/// Status of a task that is still running.
const RUNNING: &str = "running";

/// Status of a task that ended badly.
const FAILED: &str = "failed";

thread_local! {
    /// When each finished task ended, by row id. Rows still running are absent.
    static AGES: RefCell<HashMap<i32, Instant>> = RefCell::new(HashMap::new());
}

/// What one batch changed, for the caller that has to act on it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Warning text in the batch, in arrival order.
    pub warnings: Vec<String>,
    /// Ids of the rows this batch moved to done or failed.
    pub finished: Vec<i32>,
    /// One line per task this batch failed, already worded for a reader.
    pub failures: Vec<String>,
}

/// Starts the forwarder thread. It ends when the channel closes or the window is gone.
pub fn start_forwarder(mut rx: UnboundedReceiver<Event>, weak: Weak<AppWindow>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while let Some(first) = rx.blocking_recv() {
            let mut batch = vec![first];
            std::thread::sleep(BATCH);
            while let Ok(next) = rx.try_recv() {
                batch.push(next);
            }
            let posted = weak.upgrade_in_event_loop(move |window| push(&window, &batch));
            if posted.is_err() {
                break;
            }
        }
    })
}

/// Applies one batch to the models. Runs on the UI thread.
fn push(window: &AppWindow, events: &[Event]) {
    let app = window.global::<App>();
    let mut tasks: Vec<TaskRow> = app.get_tasks().iter().collect();
    let mut log: VecDeque<LogLine> = app.get_app_log().iter().collect();

    let applied = apply(&mut tasks, &mut log, events);

    // Stamp each row the moment it ended. `or_insert` keeps the first stamp, so a repeated
    // finish event cannot keep a row alive.
    let now = Instant::now();
    AGES.with_borrow_mut(|ages| {
        for id in &applied.finished {
            ages.entry(*id).or_insert(now);
        }
    });

    app.set_busy(any_running(&tasks));
    app.set_tasks(ModelRc::new(VecModel::from(tasks)));
    app.set_app_log(ModelRc::new(VecModel::from(Vec::from(log))));

    for warning in &applied.warnings {
        toasts::show(window, warning, "warning");
    }
    // A failure is the one thing the user has to read, so it is said three ways: a toast, an
    // error line in the app log, and the panel opening itself on the row that carries it.
    for failure in &applied.failures {
        toasts::show(window, failure, "error");
    }
    if !applied.failures.is_empty() {
        app.set_panel_expanded(true);
    }
}

/// Ages finished rows out of the task list. Runs on the UI thread, once a second.
///
/// Returns whether the list changed, so the caller can skip the redraw.
pub fn tick(window: &AppWindow, now: Instant) -> bool {
    let app = window.global::<App>();
    let mut rows: Vec<TaskRow> = app.get_tasks().iter().collect();
    let changed =
        AGES.with_borrow_mut(|ages| prune_finished(&mut rows, ages, now, KEEP_DONE, FAILED_TTL));
    if changed {
        app.set_busy(any_running(&rows));
        app.set_tasks(ModelRc::new(VecModel::from(rows)));
    }
    changed
}

/// Drops every finished row that ended long enough ago, and forgets its stamp.
///
/// A row that ended cleanly is kept for `keep`, one that failed for `failed_keep`, measured
/// from its own end. Pure: the caller owns both the rows and the stamps. A running row is
/// never touched, and a finished row with no stamp is kept until the next batch stamps it.
pub fn prune_finished(
    rows: &mut Vec<TaskRow>,
    ages: &mut HashMap<i32, Instant>,
    now: Instant,
    keep: Duration,
    failed_keep: Duration,
) -> bool {
    let before = rows.len();
    rows.retain(|row| {
        if row.status.as_str() == RUNNING {
            return true;
        }
        let life = if row.status.as_str() == FAILED {
            failed_keep
        } else {
            keep
        };
        match ages.get(&row.id) {
            // `duration_since` saturates, so a stamp from the future reads as zero age.
            Some(at) => now.duration_since(*at) < life,
            None => true,
        }
    });
    // Stamps outlive nothing: anything the list no longer holds is forgotten here.
    ages.retain(|id, _| rows.iter().any(|row| row.id == *id));
    rows.len() != before
}

/// Whether any row is still running, which is what `App.busy` shows in the strip.
pub fn any_running(rows: &[TaskRow]) -> bool {
    rows.iter().any(|row| row.status.as_str() == RUNNING)
}

/// Folds a batch of events into the task list and the log ring.
///
/// Pure: no Slint instance and no UI thread needed. Returns the warnings in the batch and the
/// rows it ended, for a caller that has to surface or stamp them.
pub fn apply(tasks: &mut Vec<TaskRow>, log: &mut VecDeque<LogLine>, events: &[Event]) -> Applied {
    let mut applied = Applied::default();
    for event in events {
        match event {
            Event::TaskStarted {
                id,
                label,
                total_bytes,
            } => {
                // `TaskId` is a `u64` and `TaskRow.id` is a Slint `int`, which is an `i32`.
                // An id past `i32::MAX` would alias another row, so the event is dropped
                // instead: losing a progress line beats writing over the wrong task.
                let Some(id) = row_id(*id) else { continue };
                let row = TaskRow {
                    id,
                    label: label.as_str().into(),
                    fraction: 0.0,
                    status: RUNNING.into(),
                    detail: total_bytes.map(format_bytes).unwrap_or_default().into(),
                };
                match find(tasks, id) {
                    Some(existing) => *existing = row,
                    None => tasks.push(row),
                }
            }
            Event::TaskProgress {
                id,
                done_bytes,
                total_bytes,
            } => {
                let Some(id) = row_id(*id) else { continue };
                if let Some(row) = find(tasks, id) {
                    row.fraction = match total_bytes {
                        Some(total) if *total > 0 => *done_bytes as f32 / *total as f32,
                        _ => 0.0,
                    };
                    row.detail = match total_bytes {
                        Some(total) => {
                            format!("{} / {}", format_bytes(*done_bytes), format_bytes(*total))
                        }
                        None => format_bytes(*done_bytes),
                    }
                    .into();
                }
            }
            Event::TaskFinished { id } => {
                let Some(id) = row_id(*id) else { continue };
                if let Some(row) = find(tasks, id) {
                    row.fraction = 1.0;
                    row.status = "done".into();
                    applied.finished.push(id);
                }
            }
            Event::TaskFailed { id, error } => {
                let Some(id) = row_id(*id) else { continue };
                if let Some(row) = find(tasks, id) {
                    row.status = FAILED.into();
                    row.detail = error.as_str().into();
                    applied.finished.push(id);
                    // The row's own `detail` is easy to miss: the panel may be shut and the
                    // row goes eventually. The same words are kept in the log and shown as a
                    // toast, so the failure survives both.
                    let message = format!("{} failed: {}", row.label, error);
                    push_log(log, "error", &message);
                    applied.failures.push(message);
                }
            }
            Event::Log { level, message } => push_log(log, level_name(*level), message),
            Event::Warning(message) => {
                push_log(log, "warning", message);
                applied.warnings.push(message.clone());
            }
        }
    }
    applied
}

/// The row id for a core task id, or `None` when it does not fit a Slint `int`.
fn row_id(id: u64) -> Option<i32> {
    i32::try_from(id).ok()
}

/// The task row with this id, if the list still holds it.
fn find(tasks: &mut [TaskRow], id: i32) -> Option<&mut TaskRow> {
    tasks.iter_mut().find(|t| t.id == id)
}

/// Appends a log line, dropping the oldest once the ring is full.
fn push_log(log: &mut VecDeque<LogLine>, level: &str, text: &str) {
    log.push_back(LogLine {
        level: level.into(),
        text: text.into(),
    });
    while log.len() > LOG_LIMIT {
        log.pop_front();
    }
}

/// Lowercase name of a log level, as the `.slint` side matches on it.
fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

#[cfg(test)]
mod tests;
