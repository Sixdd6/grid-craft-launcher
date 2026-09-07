//! Unit tests for the pure event fold. No Slint instance and no display needed.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use gcl_core::events::{Event, LogLevel};

use super::{FAILED_TTL, KEEP_DONE, LOG_LIMIT, any_running, apply, prune_finished};
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
fn a_failure_is_worded_once_for_the_log_and_the_toast() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    apply(&mut tasks, &mut log, &[started(2, Some(10))]);
    let applied = apply(
        &mut tasks,
        &mut log,
        &[Event::TaskFailed {
            id: 2,
            error: "connection reset".into(),
        }],
    );
    assert_eq!(applied.failures, vec!["task 2 failed: connection reset"]);
    let last = log.back().expect("a line was appended");
    assert_eq!(last.level.as_str(), "error");
    assert_eq!(last.text.as_str(), "task 2 failed: connection reset");
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
    let applied = apply(
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
    assert_eq!(applied.warnings, vec!["no CurseForge key".to_string()]);
    assert!(applied.finished.is_empty());
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

#[test]
fn finishing_and_failing_are_both_reported_as_ended() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    apply(&mut tasks, &mut log, &[started(1, None), started(2, None)]);
    let applied = apply(
        &mut tasks,
        &mut log,
        &[
            Event::TaskFinished { id: 1 },
            Event::TaskFailed {
                id: 2,
                error: "timed out".into(),
            },
        ],
    );
    assert_eq!(applied.finished, vec![1, 2]);
}

#[test]
fn an_id_too_large_for_a_slint_int_is_dropped() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    let huge = i32::MAX as u64 + 1;
    apply(&mut tasks, &mut log, &[started(huge, Some(10))]);
    assert!(
        tasks.is_empty(),
        "a row that would alias another is not made"
    );

    let applied = apply(&mut tasks, &mut log, &[Event::TaskFinished { id: huge }]);
    assert!(applied.finished.is_empty());
}

#[test]
fn the_largest_usable_id_still_makes_a_row() {
    let mut tasks = Vec::new();
    let mut log = VecDeque::new();
    apply(&mut tasks, &mut log, &[started(i32::MAX as u64, Some(10))]);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, i32::MAX);
}

/// One row, with only the fields the prune reads set to anything meaningful.
fn row(id: i32, status: &str) -> TaskRow {
    TaskRow {
        id,
        label: format!("task {id}").into(),
        fraction: 0.0,
        status: status.into(),
        detail: "".into(),
    }
}

#[test]
fn prune_drops_a_row_five_seconds_after_its_own_end() {
    let start = Instant::now();
    let mut rows = vec![row(1, "done"), row(2, "done")];
    let mut ages = HashMap::from([(1, start), (2, start + Duration::from_secs(4))]);

    let now = start + KEEP_DONE + Duration::from_millis(1);
    assert!(prune_finished(
        &mut rows, &mut ages, now, KEEP_DONE, FAILED_TTL
    ));
    assert_eq!(rows.len(), 1, "only the older row is old enough");
    assert_eq!(rows[0].id, 2);
    assert!(!ages.contains_key(&1), "its stamp went with it");
    assert!(ages.contains_key(&2));
}

#[test]
fn prune_gives_a_failed_row_six_times_as_long_to_be_read() {
    let start = Instant::now();
    let mut rows = vec![row(1, "failed")];
    let mut ages = HashMap::from([(1, start)]);

    let five = start + KEEP_DONE + Duration::from_millis(1);
    assert!(!prune_finished(
        &mut rows, &mut ages, five, KEEP_DONE, FAILED_TTL
    ));
    assert_eq!(rows.len(), 1, "a done row would be gone by now");

    let thirty_one = start + FAILED_TTL + Duration::from_secs(1);
    assert!(prune_finished(
        &mut rows, &mut ages, thirty_one, KEEP_DONE, FAILED_TTL
    ));
    assert!(rows.is_empty());
    assert!(ages.is_empty(), "its stamp went with it");
}

#[test]
fn prune_never_drops_a_running_row() {
    let start = Instant::now();
    let mut rows = vec![row(1, "running")];
    let mut ages = HashMap::new();
    let now = start + Duration::from_secs(60);
    assert!(!prune_finished(
        &mut rows, &mut ages, now, KEEP_DONE, FAILED_TTL
    ));
    assert_eq!(rows.len(), 1);
}

#[test]
fn prune_keeps_a_finished_row_that_has_no_stamp_yet() {
    let mut rows = vec![row(1, "failed")];
    let mut ages = HashMap::new();
    let now = Instant::now() + Duration::from_secs(60);
    assert!(!prune_finished(
        &mut rows, &mut ages, now, KEEP_DONE, FAILED_TTL
    ));
    assert_eq!(rows.len(), 1);
}

#[test]
fn prune_forgets_stamps_for_rows_that_are_gone() {
    let start = Instant::now();
    let mut rows: Vec<TaskRow> = Vec::new();
    let mut ages = HashMap::from([(7, start)]);
    prune_finished(&mut rows, &mut ages, start, KEEP_DONE, FAILED_TTL);
    assert!(ages.is_empty());
}

#[test]
fn busy_follows_the_running_rows() {
    assert!(!any_running(&[]));
    assert!(!any_running(&[row(1, "done"), row(2, "failed")]));
    assert!(any_running(&[row(1, "done"), row(2, "running")]));
}
