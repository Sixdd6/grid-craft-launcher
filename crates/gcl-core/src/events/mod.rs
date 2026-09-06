//! Progress and log events emitted by long-running core operations.

use std::sync::atomic::{AtomicU64, Ordering};

/// Identifier for a single tracked task, unique within a process run.
pub type TaskId = u64;

/// Channel used to publish events from core operations to a UI or CLI.
pub type EventSink = tokio::sync::mpsc::UnboundedSender<Event>;

/// One thing that happened during a launcher operation.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A task started. `total_bytes` is `None` when the size is not known yet.
    TaskStarted {
        /// The task's id.
        id: TaskId,
        /// Human-readable description of the task.
        label: String,
        /// Total size in bytes, if known.
        total_bytes: Option<u64>,
    },
    /// A task made progress.
    TaskProgress {
        /// The task's id.
        id: TaskId,
        /// Bytes completed so far.
        done_bytes: u64,
        /// Total size in bytes, if known.
        total_bytes: Option<u64>,
    },
    /// A task finished successfully.
    TaskFinished {
        /// The task's id.
        id: TaskId,
    },
    /// A task failed.
    TaskFailed {
        /// The task's id.
        id: TaskId,
        /// Description of the failure.
        error: String,
    },
    /// A log line at a given level.
    Log {
        /// Severity of the log line.
        level: LogLevel,
        /// The log message.
        message: String,
    },
    /// A non-fatal warning to surface to the user.
    Warning(String),
}

/// Severity of a `Log` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Verbose diagnostic detail.
    Debug,
    /// Normal operational message.
    Info,
    /// Something unexpected but not fatal.
    Warn,
    /// Something failed.
    Error,
}

/// Creates a sink that discards every event it receives. For tests.
pub fn null_sink() -> EventSink {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    tx
}

static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

/// Returns a fresh, process-unique task id.
pub fn next_task_id() -> TaskId {
    NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed)
}

/// Handle to a single in-flight task, used to emit its progress events.
pub struct TaskHandle {
    id: TaskId,
    sink: EventSink,
    total_bytes: Option<u64>,
}

impl TaskHandle {
    /// Starts a new task: allocates an id, emits `TaskStarted`, and returns the handle.
    pub fn start(sink: &EventSink, label: impl Into<String>, total_bytes: Option<u64>) -> Self {
        let id = next_task_id();
        let handle = TaskHandle {
            id,
            sink: sink.clone(),
            total_bytes,
        };
        let _ = handle.sink.send(Event::TaskStarted {
            id,
            label: label.into(),
            total_bytes,
        });
        handle
    }

    /// Emits a `TaskProgress` event carrying the bytes done so far.
    pub fn progress(&self, done_bytes: u64) {
        let _ = self.sink.send(Event::TaskProgress {
            id: self.id,
            done_bytes,
            total_bytes: self.total_bytes,
        });
    }

    /// Emits `TaskFinished` and consumes the handle.
    pub fn finish(self) {
        let _ = self.sink.send(Event::TaskFinished { id: self.id });
    }

    /// Emits `TaskFailed` with the given error description and consumes the handle.
    pub fn fail(self, error: impl Into<String>) {
        let _ = self.sink.send(Event::TaskFailed {
            id: self.id,
            error: error.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_handle_emits_started_progress_finished() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let t = TaskHandle::start(&tx, "download foo", Some(10));
        t.progress(4);
        t.finish();
        let a = rx.recv().await.unwrap();
        assert!(
            matches!(a, Event::TaskStarted { label, total_bytes: Some(10), .. } if label == "download foo")
        );
        assert!(matches!(
            rx.recv().await.unwrap(),
            Event::TaskProgress { done_bytes: 4, .. }
        ));
        assert!(matches!(
            rx.recv().await.unwrap(),
            Event::TaskFinished { .. }
        ));
    }

    #[test]
    fn task_ids_increase() {
        assert!(next_task_id() < next_task_id());
    }
}
