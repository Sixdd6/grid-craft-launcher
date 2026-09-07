//! Forwards core [`Event`]s onto the UI thread.
//!
//! One thread owns the event receiver. It blocks for the first event of a batch, collects
//! everything that arrives within [`BATCH`], and posts one closure to the UI thread. The
//! task list and the log live in the Slint models, so the UI thread is their only owner.

use std::collections::VecDeque;
use std::thread::JoinHandle;
use std::time::Duration;

use gcl_core::events::{Event, LogLevel};
use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::models::format_bytes;
use crate::{App, AppWindow, LogLine, TaskRow};

/// How long events are collected before a batch is posted to the UI thread.
const BATCH: Duration = Duration::from_millis(50);

/// How long a finished task stays in the list before it is pruned.
const KEEP_DONE: Duration = Duration::from_secs(5);

/// Most log lines kept in memory. Older lines are dropped from the front.
pub const LOG_LIMIT: usize = 2000;

/// Status of a task that is still running.
const RUNNING: &str = "running";

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

    apply(&mut tasks, &mut log, events);

    let has_finished = tasks.iter().any(|t| t.status.as_str() != RUNNING);
    app.set_tasks(ModelRc::new(VecModel::from(tasks)));
    app.set_app_log(ModelRc::new(VecModel::from(Vec::from(log))));
    if has_finished {
        arm_prune(window.as_weak());
    }
}

/// Drops finished and failed tasks from the list once they have been visible for `KEEP_DONE`.
fn arm_prune(weak: Weak<AppWindow>) {
    slint::Timer::single_shot(KEEP_DONE, move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let app = window.global::<App>();
        let kept: Vec<TaskRow> = app
            .get_tasks()
            .iter()
            .filter(|t| t.status.as_str() == RUNNING)
            .collect();
        app.set_tasks(ModelRc::new(VecModel::from(kept)));
    });
}

/// Folds a batch of events into the task list and the log ring.
///
/// Pure: no Slint instance and no UI thread needed. Returns the warnings in the batch, in
/// arrival order, for a caller that wants to surface them somewhere other than the log.
pub fn apply(
    tasks: &mut Vec<TaskRow>,
    log: &mut VecDeque<LogLine>,
    events: &[Event],
) -> Vec<String> {
    let mut warnings = Vec::new();
    for event in events {
        match event {
            Event::TaskStarted {
                id,
                label,
                total_bytes,
            } => {
                let row = TaskRow {
                    id: *id as i32,
                    label: label.as_str().into(),
                    fraction: 0.0,
                    status: RUNNING.into(),
                    detail: total_bytes.map(format_bytes).unwrap_or_default().into(),
                };
                match find(tasks, *id) {
                    Some(existing) => *existing = row,
                    None => tasks.push(row),
                }
            }
            Event::TaskProgress {
                id,
                done_bytes,
                total_bytes,
            } => {
                if let Some(row) = find(tasks, *id) {
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
                if let Some(row) = find(tasks, *id) {
                    row.fraction = 1.0;
                    row.status = "done".into();
                }
            }
            Event::TaskFailed { id, error } => {
                if let Some(row) = find(tasks, *id) {
                    row.status = "failed".into();
                    row.detail = error.as_str().into();
                }
            }
            Event::Log { level, message } => push_log(log, level_name(*level), message),
            Event::Warning(message) => {
                push_log(log, "warning", message);
                warnings.push(message.clone());
            }
        }
    }
    warnings
}

/// The task row with this id, if the list still holds it.
fn find(tasks: &mut [TaskRow], id: u64) -> Option<&mut TaskRow> {
    tasks.iter_mut().find(|t| t.id == id as i32)
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
