//! State shared by more than one screen.
//!
//! The instances list and the instance detail screen both show whether a game is up, and a
//! launch started from either has to reach both. That set lives here, behind a handle both
//! screens hold.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// The slugs whose game is still running.
///
/// Cloning shares one set, so every screen sees the same launches. A poisoned lock is taken
/// anyway: the set is plain data, and losing it would only stick a row on "Running".
#[derive(Clone, Default)]
pub struct RunState(Arc<Mutex<HashSet<String>>>);

impl RunState {
    /// An empty set, with nothing running.
    pub fn new() -> Self {
        RunState::default()
    }

    /// Marks a slug as running. Returns false when it already was, so a duplicate launch
    /// started from a stale row can be dropped by the caller.
    pub fn start(&self, slug: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(slug.to_string())
    }

    /// Marks a slug as no longer running.
    pub fn finish(&self, slug: &str) {
        self.0
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(slug);
    }

    /// Whether this slug's game is up.
    pub fn is_running(&self, slug: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .contains(slug)
    }

    /// A copy of the whole set, for rewriting a list of rows in one pass.
    pub fn snapshot(&self) -> HashSet<String> {
        self.0.lock().unwrap_or_else(|err| err.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::RunState;

    #[test]
    fn start_reports_only_the_first_launch_of_a_slug() {
        let run = RunState::new();
        assert!(run.start("vanilla"));
        assert!(!run.start("vanilla"), "a second launch is not fresh");
        assert!(run.is_running("vanilla"));
    }

    #[test]
    fn finish_clears_the_slug_and_clones_share_the_set() {
        let run = RunState::new();
        let other = run.clone();
        assert!(run.start("vanilla"));
        assert!(other.is_running("vanilla"), "clones share one set");
        other.finish("vanilla");
        assert!(!run.is_running("vanilla"));
        assert!(run.snapshot().is_empty());
    }
}
