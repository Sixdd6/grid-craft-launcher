//! File and stderr logging for the GUI.
//!
//! The GUI writes every job outcome to `<root>/logs/gui.log`, rotated daily, so a user who
//! sees a dead button has something to send. Stderr stays quiet unless `GCL_LOG` is set,
//! which then takes an `EnvFilter` directive string such as `GCL_LOG=debug` or
//! `GCL_LOG=gcl_core::download=trace`.

use gcl_core::paths::Root;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// The environment variable that turns stderr logging on and sets its filter.
const ENV: &str = "GCL_LOG";

/// The file the GUI logs to, inside the root's `logs/` directory.
///
/// `tracing_appender` appends the date to it, so the file on disk is `gui.log.2026-09-07`.
const FILE: &str = "gui.log";

/// Keeps the log writer's worker thread alive.
///
/// `tracing_appender`'s non-blocking writer flushes on a worker thread that stops when this
/// guard drops, so `main` holds it until the event loop returns. Dropping it early loses the
/// tail of the log.
pub struct LogGuard(#[allow(dead_code)] WorkerGuard);

/// The log file this process is writing to right now.
///
/// `tracing_appender::rolling::daily` names its file `<prefix>.<date>` in UTC, so the name has
/// to be rebuilt from today's date rather than guessed. The error dialog shows this path.
pub fn log_file(root: &Root) -> std::path::PathBuf {
    let today = time::OffsetDateTime::now_utc().date();
    let name = match today.format(time::macros::format_description!("[year]-[month]-[day]")) {
        Ok(date) => format!("{FILE}.{date}"),
        // A date that will not format is not worth an error; the directory still helps.
        Err(_) => FILE.to_string(),
    };
    root.logs_dir().join(name)
}

/// Starts the global subscriber: a daily file in `root.logs_dir()` at `info`, plus stderr when
/// `GCL_LOG` is set.
///
/// Returns the guard that keeps the file writer alive. Calling this twice in one process
/// leaves the first subscriber in place; the second call's layers are dropped.
pub fn init(root: &Root) -> LogGuard {
    let appender = tracing_appender::rolling::daily(root.logs_dir(), FILE);
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .with_filter(EnvFilter::new("info"));

    let stderr_layer = std::env::var(ENV).ok().map(|_| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(EnvFilter::from_env(ENV))
    });

    // `try_init` rather than `init`: a second call (a test, a re-entry) must not panic.
    let _ = tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .try_init();

    LogGuard(guard)
}
