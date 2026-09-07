//! Unit tests for the pure event fold. No Slint instance and no display needed.

use std::collections::VecDeque;

use gcl_core::events::{Event, LogLevel};

use super::{LOG_LIMIT, apply};
use crate::{LogLine, TaskRow};

fn started(id: u64, total: Option<u64>) -> Event {
    Event::TaskStarted {
        id,
        label: format!("task {id}"),
        total_bytes: total,
    }
}

#[test]
fn start_progress_finish_updates_one_row() {
    let mut tasks: Vec<TaskRow> = Vec::new();
    let mut log: VecDeque<LogLine> = VecDeque::new();

    apply(&mut tasks, &mut log, &[started(7, Some(1000))]);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, 7);
    assert_eq!(tasks[0].label.as_str(), "task 7");
    assert_eq!(tasks[0].status.as_str(), "running");

    apply(
        &mut tasks,
        &mut log,
        &[Event::TaskProgress {
            id: 7,
            done_bytes: 500,
            total_bytes: Some(1000),
        }],
    );
    assert_eq!(tasks.len(), 1);
    assert!((tasks[0].fraction - 0.5).abs() < 1e-6);
    assert!(tasks[0].detail.as_str().contains('/'));

    apply(&mut tasks, &mut log, &[Event::TaskFinished { id: 7 }]);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status.as_str(), "done");
    assert!((tasks[0].fraction - 1.0).abs() < 1e-6);
}

#[test]
fn progress_without_a_total_leaves_the_fraction_at_zero() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    apply(&mut tasks, &mut log, &[started(1, None)]);
    apply(
        &mut tasks,
        &mut log,
        &[Event::TaskProgress {
            id: 1,
            done_bytes: 4096,
            total_bytes: None,
        }],
    );
    assert_eq!(tasks[0].fraction, 0.0);
    assert_eq!(tasks[0].detail.as_str(), "4.0 KiB");
}

#[test]
fn failed_marks_the_row_and_keeps_the_error() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    apply(&mut tasks, &mut log, &[started(2, Some(10))]);
    apply(
        &mut tasks,
        &mut log,
        &[Event::TaskFailed {
            id: 2,
            error: "connection reset".into(),
        }],
    );
    assert_eq!(tasks[0].status.as_str(), "failed");
    assert_eq!(tasks[0].detail.as_str(), "connection reset");
}

#[test]
fn progress_for_an_unknown_task_is_ignored() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    apply(
        &mut tasks,
        &mut log,
        &[Event::TaskProgress {
            id: 99,
            done_bytes: 1,
            total_bytes: Some(2),
        }],
    );
    assert!(tasks.is_empty());
}

#[test]
fn warnings_are_collected_and_logged() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    let warnings = apply(
        &mut tasks,
        &mut log,
        &[
            Event::Log {
                level: LogLevel::Info,
                message: "installing".into(),
            },
            Event::Warning("no CurseForge key".into()),
        ],
    );
    assert_eq!(warnings, vec!["no CurseForge key".to_string()]);
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].level.as_str(), "info");
    assert_eq!(log[1].level.as_str(), "warning");
    assert_eq!(log[1].text.as_str(), "no CurseForge key");
}

#[test]
fn the_log_ring_keeps_the_newest_lines_only() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    let events: Vec<Event> = (0..LOG_LIMIT + 10)
        .map(|i| Event::Log {
            level: LogLevel::Debug,
            message: format!("line {i}"),
        })
        .collect();
    apply(&mut tasks, &mut log, &events);
    assert_eq!(log.len(), LOG_LIMIT);
    assert_eq!(log[0].text.as_str(), "line 10");
    assert_eq!(
        log[LOG_LIMIT - 1].text.as_str(),
        format!("line {}", LOG_LIMIT + 9)
    );
}
