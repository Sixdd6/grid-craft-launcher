//! The toast queue: short-lived messages stacked over the shell.
//!
//! A toast is a copy of something that is already in the app log, shown for [`TTL`] so the
//! user notices it without opening the log. The queue lives on the UI thread in a
//! `thread_local!`, because every writer is already there: warnings arrive in the forwarder's
//! model-mutating closure, screens push through `Shell.toast`, and the one-second timer in
//! `src/app.rs` ages entries out. Nothing here locks, and nothing here is `Send`.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{App, AppWindow, ToastRow};

/// How long one toast stays on screen.
pub const TTL: Duration = Duration::from_secs(6);

/// Most toasts shown at once. An older one is dropped to make room.
pub const MAX: usize = 3;

/// One queued message and the moment it was pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What the toast says.
    pub text: String,
    /// warning, error, or anything else, which the host draws as a plain note.
    pub kind: String,
    /// When it was pushed. [`prune`] measures its age from here.
    pub at: Instant,
}

thread_local! {
    /// The live queue, oldest first. Only the UI thread ever touches it.
    static QUEUE: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
}

/// Appends a message and drops the oldest once the stack is full.
///
/// Pure: the caller owns the queue, so this is the half that tests can drive.
pub fn push(queue: &mut Vec<Entry>, text: &str, kind: &str, now: Instant, max: usize) {
    queue.push(Entry {
        text: text.to_string(),
        kind: kind.to_string(),
        at: now,
    });
    while queue.len() > max {
        queue.remove(0);
    }
}

/// Drops every message older than `ttl`. Returns whether anything went.
///
/// Pure, for the same reason [`push`] is.
pub fn prune(queue: &mut Vec<Entry>, now: Instant, ttl: Duration) -> bool {
    let before = queue.len();
    // `duration_since` saturates, so an entry stamped in the same instant reads as zero.
    queue.retain(|entry| now.duration_since(entry.at) < ttl);
    queue.len() != before
}

/// Pushes one message onto the live queue and redraws the stack. UI thread only.
pub fn show(window: &AppWindow, text: &str, kind: &str) {
    QUEUE.with_borrow_mut(|queue| push(queue, text, kind, Instant::now(), MAX));
    sync(window);
}

/// Ages the live queue out and redraws the stack when something went. UI thread only.
///
/// Returns whether the stack changed, so the caller can skip the redraw.
pub fn tick(window: &AppWindow, now: Instant) -> bool {
    let changed = QUEUE.with_borrow_mut(|queue| prune(queue, now, TTL));
    if changed {
        sync(window);
    }
    changed
}

/// Copies the live queue into the `App.toasts` model.
fn sync(window: &AppWindow) {
    let rows: Vec<ToastRow> = QUEUE.with_borrow(|queue| {
        queue
            .iter()
            .map(|entry| ToastRow {
                text: entry.text.as_str().into(),
                kind: entry.kind.as_str().into(),
            })
            .collect()
    });
    window
        .global::<App>()
        .set_toasts(ModelRc::new(VecModel::from(rows)));
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Entry, MAX, TTL, prune, push};

    fn texts(queue: &[Entry]) -> Vec<&str> {
        queue.iter().map(|entry| entry.text.as_str()).collect()
    }

    #[test]
    fn push_keeps_the_newest_messages_only() {
        let now = Instant::now();
        let mut queue = Vec::new();
        for i in 0..MAX + 2 {
            push(&mut queue, &format!("m{i}"), "warning", now, MAX);
        }
        assert_eq!(queue.len(), MAX);
        assert_eq!(texts(&queue), vec!["m2", "m3", "m4"]);
    }

    #[test]
    fn push_records_the_kind() {
        let mut queue = Vec::new();
        push(&mut queue, "boom", "error", Instant::now(), MAX);
        assert_eq!(queue[0].kind, "error");
    }

    #[test]
    fn prune_drops_only_what_outlived_the_ttl() {
        let start = Instant::now();
        let mut queue = Vec::new();
        push(&mut queue, "old", "warning", start, MAX);
        push(&mut queue, "new", "warning", start + TTL, MAX);

        let now = start + TTL + Duration::from_millis(1);
        assert!(prune(&mut queue, now, TTL), "the old one went");
        assert_eq!(texts(&queue), vec!["new"]);
    }

    #[test]
    fn prune_reports_nothing_when_every_message_is_young() {
        let start = Instant::now();
        let mut queue = Vec::new();
        push(&mut queue, "fresh", "warning", start, MAX);
        assert!(!prune(&mut queue, start + Duration::from_secs(1), TTL));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn prune_on_an_empty_queue_changes_nothing() {
        let mut queue = Vec::new();
        assert!(!prune(&mut queue, Instant::now(), TTL));
    }
}
