//! Runs `Launcher` calls off the UI thread and posts their results back.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use gcl_core::Launcher;
use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};

use crate::{App, AppWindow, LogLine};

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

    /// The GUI log file this process writes to, named in every error dialog.
    pub fn log_path(&self) -> std::path::PathBuf {
        crate::logging::log_file(self.launcher.root())
    }

    /// Runs `job` on its own thread. On success `done` runs on the UI thread; on failure the
    /// error dialog opens with `label` as its title.
    pub fn run<T: Send + 'static>(
        &self,
        label: &'static str,
        job: impl FnOnce(&Launcher) -> Result<T, gcl_core::Error> + Send + 'static,
        done: impl FnOnce(&AppWindow, T) + Send + 'static,
    ) {
        self.run_logged(label, true, job, done);
    }

    /// [`Bridge::run`], with the outcome log line under a switch.
    ///
    /// [`Bridge::run_with_error`] hands the outer job a `Result` that is always `Ok`, so the
    /// outer line would say every job succeeded. It passes `false` here and writes its own
    /// line from the result it can actually see.
    fn run_logged<T: Send + 'static>(
        &self,
        label: &'static str,
        log_outcome: bool,
        job: impl FnOnce(&Launcher) -> Result<T, gcl_core::Error> + Send + 'static,
        done: impl FnOnce(&AppWindow, T) + Send + 'static,
    ) {
        let launcher = Arc::clone(&self.launcher);
        let weak = self.weak.clone();
        let log_path = self.log_path();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            tracing::debug!(label, "job start");
            let result = spawn_job(move || job(&launcher))
                .recv()
                .unwrap_or_else(|_| Err(worker_gone()));
            let elapsed_ms = elapsed_ms(started);
            if log_outcome {
                match &result {
                    Ok(_) => tracing::info!(label, elapsed_ms, "job ok"),
                    Err(err) => {
                        tracing::error!(label, elapsed_ms, error = %error_chain(err), "job failed");
                    }
                }
            }
            let _ = weak.upgrade_in_event_loop(move |window| match result {
                Ok(value) => done(&window, value),
                Err(err) => show_error_with_log(&window, label, &err, Some(&log_path)),
            });
        });
    }

    /// Runs `job` on its own thread and hands its whole result to `done` on the UI thread.
    ///
    /// [`Bridge::run`] drops `done` when the job fails, which leaves a screen that set a
    /// "loading" flag stuck on it. Here `done` always runs, so every path can clear that
    /// flag, and the error dialog still opens with `label` as its title before it does.
    ///
    /// A job that panics is the one exception: the worker thread dies without sending a
    /// result, so the call takes the generic "the background job ended without a result"
    /// error path and `done` is not called at all. Do not panic in a job; return an error.
    pub fn run_with_error<T: Send + 'static>(
        &self,
        label: &'static str,
        job: impl FnOnce(&Launcher) -> Result<T, gcl_core::Error> + Send + 'static,
        done: impl FnOnce(&AppWindow, Result<T, gcl_core::Error>) + Send + 'static,
    ) {
        let log_path = self.log_path();
        self.run_logged(
            label,
            false,
            move |launcher| {
                let started = std::time::Instant::now();
                let result = job(launcher);
                Ok((result, elapsed_ms(started)))
            },
            move |window, (result, elapsed_ms)| {
                match &result {
                    Ok(_) => tracing::info!(label, elapsed_ms, "job ok"),
                    Err(err) => {
                        tracing::error!(label, elapsed_ms, error = %error_chain(err), "job failed");
                        show_error_with_log(window, label, err, Some(&log_path));
                    }
                }
                done(window, result);
            },
        );
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
///
/// The body ends with the path of the GUI log, so a user reading the dialog knows where the
/// rest of the story is. `logs_path` is the whole reason `show_error` needs the bridge.
pub fn show_error(window: &AppWindow, label: &str, err: &gcl_core::Error) {
    show_error_with_log(window, label, err, None);
}

/// [`show_error`], with the log path a caller already knows.
///
/// `Bridge` has the launcher and so the root; a bare `show_error` call from a screen does not,
/// and passes `None`.
pub fn show_error_with_log(
    window: &AppWindow,
    label: &str,
    err: &gcl_core::Error,
    log_path: Option<&std::path::Path>,
) {
    let app = window.global::<App>();
    let mut text = error_chain(err);
    if let Some(path) = log_path {
        text.push_str("\n\nDetails: ");
        text.push_str(&path.display().to_string());
    }
    app.set_error_title(label.into());
    app.set_error_text(text.into());
    app.set_error_open(true);
}

/// Appends a warning to the app log and stacks it as a toast.
///
/// Every warning a screen raises goes through here, so the browser's manual-download note and
/// a bad launch outcome both reach the toast host without either caller knowing about it.
pub fn warn(window: &AppWindow, text: &str) {
    let app = window.global::<App>();
    let mut log: Vec<LogLine> = app.get_app_log().iter().collect();
    log.push(LogLine {
        level: "warning".into(),
        text: text.into(),
    });
    app.set_app_log(ModelRc::new(VecModel::from(log)));
    crate::toasts::show(window, text, "warning");
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

/// Milliseconds since `started`, saturating rather than wrapping.
fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The error reported when a job thread died without sending a result.
fn worker_gone() -> gcl_core::Error {
    std::io::Error::other("the background job ended without a result").into()
}

#[cfg(test)]
mod tests;
