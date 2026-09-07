//! Rendering: aligned text columns, JSON documents, and the background event printer.
//!
//! Command results go to stdout so `--json` output stays parseable. Progress events go to
//! stderr, in the same format the command was asked for. The one exception is a game log
//! line in text mode: it is the launch's output, so it goes to stdout.

use std::collections::HashMap;
use std::thread::JoinHandle;

use gcl_core::events::{Event, LogLevel, TaskId};
use serde::Serialize;
use tokio::sync::mpsc::UnboundedReceiver;

/// How the CLI renders what a command produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Human-readable columns and sentences.
    Text,
    /// One JSON document on stdout, one JSON object per line on stderr.
    Json,
}

impl Format {
    /// Picks a format from the global `--json` flag.
    pub fn from_flag(json: bool) -> Format {
        if json { Format::Json } else { Format::Text }
    }
}

/// Prints a value as pretty JSON on stdout.
pub fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Prints a value as one line of compact JSON on stdout.
///
/// Used where a command writes more than one JSON document, so a caller reads them line by
/// line: `account add-msa --json` prints the login code, then the account.
pub fn print_json_line<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

/// Prints headers and rows on stdout, each column padded to its widest cell.
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }
    print_row(&widths, headers.iter().map(|h| h.to_string()));
    for row in rows {
        print_row(&widths, row.iter().cloned());
    }
}

/// Prints one padded row, with no trailing spaces after the last cell.
fn print_row(widths: &[usize], cells: impl Iterator<Item = String>) {
    let cells: Vec<String> = cells.collect();
    let mut line = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if i + 1 == cells.len() {
            line.push_str(cell);
        } else {
            let pad = widths
                .get(i)
                .copied()
                .unwrap_or(0)
                .saturating_sub(cell.chars().count());
            line.push_str(cell);
            line.push_str(&" ".repeat(pad + 2));
        }
    }
    println!("{line}");
}

/// Starts the thread that drains the event channel and prints what it receives.
///
/// Drop the [`gcl_core::Launcher`] before joining the handle, so the sender closes.
pub fn spawn_event_printer(rx: UnboundedReceiver<Event>, format: Format) -> JoinHandle<()> {
    std::thread::spawn(move || print_events(rx, format))
}

/// Drains the event channel until it closes, rendering each event to stderr.
pub fn print_events(mut rx: UnboundedReceiver<Event>, format: Format) {
    let mut labels: HashMap<TaskId, String> = HashMap::new();
    while let Some(event) = rx.blocking_recv() {
        match format {
            Format::Text => print_event_text(&mut labels, &event),
            Format::Json => print_event_json(&event),
        }
    }
}

/// Renders one event as a human-readable line.
fn print_event_text(labels: &mut HashMap<TaskId, String>, event: &Event) {
    match event {
        Event::TaskStarted { id, label, .. } => {
            labels.insert(*id, label.clone());
            eprintln!("[{label}] 0%");
        }
        Event::TaskProgress { .. } => {}
        Event::TaskFinished { id } => {
            let label = labels.remove(id).unwrap_or_else(|| id.to_string());
            eprintln!("[{label}] 100%");
        }
        Event::TaskFailed { id, error } => {
            let label = labels.remove(id).unwrap_or_else(|| id.to_string());
            eprintln!("[{label}] failed: {error}");
        }
        // Only the game process logs, so every line is one line of its output. It goes to
        // stdout, where a caller watching a launch expects it.
        Event::Log { message, .. } => println!("[game] {message}"),
        Event::Warning(message) => eprintln!("warning: {message}"),
    }
}

/// Renders one event as a single-line JSON object.
fn print_event_json(event: &Event) {
    let value = match event {
        Event::TaskStarted {
            id,
            label,
            total_bytes,
        } => serde_json::json!({
            "event": "task_started", "id": id, "label": label, "total_bytes": total_bytes,
        }),
        Event::TaskProgress {
            id,
            done_bytes,
            total_bytes,
        } => serde_json::json!({
            "event": "task_progress", "id": id,
            "done_bytes": done_bytes, "total_bytes": total_bytes,
        }),
        Event::TaskFinished { id } => serde_json::json!({ "event": "task_finished", "id": id }),
        Event::TaskFailed { id, error } => {
            serde_json::json!({ "event": "task_failed", "id": id, "error": error })
        }
        Event::Log { level, message } => serde_json::json!({
            "event": "log", "level": log_level_name(*level), "message": message,
        }),
        Event::Warning(message) => {
            serde_json::json!({ "event": "warning", "message": message })
        }
    };
    eprintln!("{value}");
}

/// Lowercase name of a log level, for JSON output.
fn log_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_follows_the_json_flag() {
        assert_eq!(Format::from_flag(true), Format::Json);
        assert_eq!(Format::from_flag(false), Format::Text);
    }

    #[test]
    fn a_finished_task_reuses_the_label_from_its_start() {
        let mut labels = HashMap::new();
        print_event_text(
            &mut labels,
            &Event::TaskStarted {
                id: 7,
                label: "download alpha".to_string(),
                total_bytes: None,
            },
        );
        assert_eq!(labels.get(&7).map(String::as_str), Some("download alpha"));
        print_event_text(&mut labels, &Event::TaskFinished { id: 7 });
        assert!(labels.is_empty());
    }

    #[test]
    fn the_printer_thread_stops_when_the_sender_drops() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = spawn_event_printer(rx, Format::Json);
        tx.send(Event::Warning("hi".to_string())).expect("send");
        drop(tx);
        handle.join().expect("printer thread joins");
    }
}
