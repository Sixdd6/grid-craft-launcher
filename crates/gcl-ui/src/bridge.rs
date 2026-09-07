//! Runs `Launcher` calls off the UI thread and posts their results back.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use gcl_core::Launcher;
use slint::{ComponentHandle, Weak};

use crate::{App, AppWindow};

/// Holds the launcher and a weak window handle, so any callback can start background work.
///
/// Cloning is cheap and shares the same launcher, so every screen can keep its own handle.
#[derive(Clone)]
pub struct Bridge {
    launcher: Arc<Launcher>,
    weak: Weak<AppWindow>,
}

impl Bridge {
    /// Builds a bridge over a shared launcher and the window its results go back to.
    pub fn new(launcher: Arc<Launcher>, weak: Weak<AppWindow>) -> Self {
        Bridge { launcher, weak }
    }

    /// The shared launcher, for a caller that needs it outside a job.
    pub fn launcher(&self) -> &Arc<Launcher> {
        &self.launcher
    }

    /// The weak window handle, for a caller that posts its own results back.
    pub fn weak(&self) -> &Weak<AppWindow> {
        &self.weak
    }

    /// Runs `job` on its own thread. On success `done` runs on the UI thread; on failure the
    /// error dialog opens with `label` as its title.
    pub fn run<T: Send + 'static>(
        &self,
        label: &'static str,
        job: impl FnOnce(&Launcher) -> Result<T, gcl_core::Error> + Send + 'static,
        done: impl FnOnce(&AppWindow, T) + Send + 'static,
    ) {
        let launcher = Arc::clone(&self.launcher);
        let weak = self.weak.clone();
        std::thread::spawn(move || {
            let result = spawn_job(move || job(&launcher))
                .recv()
                .unwrap_or_else(|_| Err(worker_gone()));
            let _ = weak.upgrade_in_event_loop(move |window| match result {
                Ok(value) => done(&window, value),
                Err(err) => show_error(&window, label, &err),
            });
        });
    }
}

/// Runs `job` on a new thread and hands back the channel its one result arrives on.
///
/// The whole of [`Bridge::run`]'s threading, with no window in it, so it can be tested.
pub fn spawn_job<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, gcl_core::Error> + Send + 'static,
) -> Receiver<Result<T, gcl_core::Error>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    rx
}

/// Opens the error dialog with `label` as the title and the error's full chain as the body.
pub fn show_error(window: &AppWindow, label: &str, err: &gcl_core::Error) {
    let app = window.global::<App>();
    app.set_error_title(label.into());
    app.set_error_text(error_chain(err).into());
    app.set_error_open(true);
}

/// Joins an error and every source under it with `: `, the way the CLI prints `{:#}`.
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(next) = source {
        let line = next.to_string();
        if !text.ends_with(&line) {
            text.push_str(": ");
            text.push_str(&line);
        }
        source = next.source();
    }
    text
}

/// The error reported when a job thread died without sending a result.
fn worker_gone() -> gcl_core::Error {
    std::io::Error::other("the background job ended without a result").into()
}

#[cfg(test)]
mod tests;
