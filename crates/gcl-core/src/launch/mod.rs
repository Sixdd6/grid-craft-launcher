//! Builds the java command line for a version and runs the game process.
//!
//! [`command::build`] turns an [`InstallPlan`](crate::mojang::install::InstallPlan) plus an
//! account, an instance, and JVM settings into a [`LaunchCommand`]. [`spawn::spawn`] starts it,
//! streams both output pipes to the event sink and to a log file, and [`spawn::wait`] returns
//! the exit code. [`crash::crash_hint`] reads that log back into a one-line reason when the
//! game exits non-zero. [`log4j::EventParser`] turns the game's log4j XML output into plain
//! lines first, so the file, the sink, and the hint never see markup.

pub mod command;
pub mod crash;
pub mod log4j;
pub mod spawn;

pub use command::{JvmSettings, LaunchCommand, LaunchInputs, build};
pub use crash::crash_hint;
pub use log4j::{EventParser, LogRecord, init_local_offset};
pub use spawn::{ChildHandle, RunningGame, force_stop, request_stop, spawn, wait};

/// Errors building or running a launch command.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The resolved version JSON names no `mainClass`, so there is nothing to start.
    #[error("version json has no mainClass")]
    MissingMainClass,
    /// An I/O operation on the log file or the asset index failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The java binary could not be started.
    #[error("could not start {program}: {source}")]
    Spawn {
        /// The program that failed to start.
        program: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The game could not be signalled.
    #[error("could not stop pid {pid}: {source}")]
    Stop {
        /// Process id the stop was aimed at.
        pid: u32,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A stop reached a game that had already exited.
    #[error("the game has already exited")]
    AlreadyExited,
    /// No game this launcher started is running for this instance.
    #[error("no game is running for {0}")]
    NotRunning(String),
    /// Laying out the legacy asset tree failed.
    #[error(transparent)]
    Legacy(#[from] crate::mojang::assets::LegacyError),
    /// A cached asset index could not be parsed.
    #[error("could not parse json at {path}: {source}")]
    Json {
        /// The path being parsed.
        path: std::path::PathBuf,
        /// The underlying JSON error.
        source: serde_json::Error,
    },
}
