//! Starts the game process and streams its output to the event sink and a log file.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::Error;
use super::command::LaunchCommand;
use crate::events::{Event, EventSink, LogLevel};

/// A running game process and the tasks draining its output.
#[derive(Debug)]
pub struct RunningGame {
    /// The child process. Take it to kill the game.
    pub child: tokio::process::Child,
    /// The file both output streams are appended to.
    pub log_path: PathBuf,
    /// The program that was started, named in a [`Error::Spawn`] if waiting fails.
    pub program: PathBuf,
    /// Two readers and one writer task, awaited by [`wait`] after the child exits.
    tasks: Vec<JoinHandle<()>>,
    /// Sink [`wait`] emits its summary line on.
    sink: EventSink,
}

/// Starts the game.
///
/// Both output pipes are read line by line. Every line becomes an [`Event::Log`] and is appended
/// to `log_path`, whose parent directories are created first. The returned handle does not block
/// on the child; call [`wait`] for the exit code.
#[tracing::instrument(skip(cmd, sink), fields(program = %cmd.program.display()))]
pub async fn spawn(
    cmd: &LaunchCommand,
    log_path: PathBuf,
    sink: EventSink,
) -> Result<RunningGame, Error> {
    if let Some(parent) = log_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .await
        .map_err(|source| Error::Io {
            path: log_path.clone(),
            source,
        })?;

    let mut child = tokio::process::Command::new(&cmd.program)
        .args(&cmd.args)
        .current_dir(&cmd.cwd)
        .envs(cmd.env.iter().map(|(k, v)| (k.clone(), v.clone())))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false)
        .spawn()
        .map_err(|source| Error::Spawn {
            program: cmd.program.clone(),
            source,
        })?;

    // One writer owns the file, so log lines reach the sink and the file in the same order.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(LogLevel, String)>();
    let mut tasks = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        tasks.push(read_lines(stdout, LogLevel::Info, tx.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tasks.push(read_lines(stderr, LogLevel::Error, tx.clone()));
    }
    drop(tx);
    let writer_sink = sink.clone();
    tasks.push(tokio::spawn(async move {
        let mut file = file;
        while let Some((level, line)) = rx.recv().await {
            if let Err(err) = file.write_all(format!("{line}\n").as_bytes()).await {
                tracing::warn!(%err, "could not write to the game log");
            }
            let _ = writer_sink.send(Event::Log {
                level,
                message: line,
            });
        }
        if let Err(err) = file.flush().await {
            tracing::warn!(%err, "could not flush the game log");
        }
    }));

    Ok(RunningGame {
        child,
        log_path,
        program: cmd.program.clone(),
        tasks,
        sink,
    })
}

/// Waits for the game to exit, drains the rest of its output, and returns the exit code.
///
/// A process killed by a signal reports `-1`.
pub async fn wait(game: RunningGame) -> Result<i32, Error> {
    let RunningGame {
        mut child,
        log_path: _,
        program,
        tasks,
        sink,
    } = game;
    let waited = child.wait().await;
    // The reader tasks are joined either way: a failed wait still leaves two tasks holding
    // the output pipes, and dropping them would lose the last lines of the log.
    for task in tasks {
        if let Err(err) = task.await {
            tracing::warn!(%err, "game log task did not finish cleanly");
        }
    }
    let status = waited.map_err(|source| Error::Spawn { program, source })?;
    let code = status.code().unwrap_or(-1);
    let level = if code == 0 {
        LogLevel::Info
    } else {
        LogLevel::Error
    };
    let _ = sink.send(Event::Log {
        level,
        message: format!("game exited with code {code}"),
    });
    Ok(code)
}

/// Spawns a task that turns every line of `stream` into a `(level, line)` message.
fn read_lines<R>(
    stream: R,
    level: LogLevel,
    tx: UnboundedSender<(LogLevel, String)>,
) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if tx.send((level, line)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(%err, "could not read game output");
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_streams_both_pipes_and_wait_returns_the_exit_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("logs/latest.log");
        let cmd = LaunchCommand {
            program: PathBuf::from("sh"),
            args: vec![
                "-c".to_string(),
                "echo out; echo err 1>&2; exit 3".to_string(),
            ],
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let game = spawn(&cmd, log_path.clone(), tx).await.expect("spawn");
        assert_eq!(game.log_path, log_path);
        assert_eq!(game.program, cmd.program);
        let code = wait(game).await.expect("wait");
        assert_eq!(code, 3);

        let mut logs = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let Event::Log { level, message } = event {
                logs.push((level, message));
            }
        }
        let summary = logs.pop().expect("summary line");
        assert_eq!(summary.0, LogLevel::Error);
        assert!(summary.1.contains("code 3"), "{}", summary.1);
        assert_eq!(logs.len(), 2, "{logs:?}");
        assert!(
            logs.contains(&(LogLevel::Info, "out".to_string())),
            "{logs:?}"
        );
        assert!(
            logs.contains(&(LogLevel::Error, "err".to_string())),
            "{logs:?}"
        );

        let written = std::fs::read_to_string(&log_path).expect("read log");
        assert!(written.contains("out\n"), "{written}");
        assert!(written.contains("err\n"), "{written}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_missing_program_is_a_spawn_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cmd = LaunchCommand {
            program: dir.path().join("no-such-java"),
            args: Vec::new(),
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
        };
        let err = spawn(
            &cmd,
            dir.path().join("logs/latest.log"),
            crate::events::null_sink(),
        )
        .await
        .expect_err("no such program");
        assert!(matches!(err, Error::Spawn { .. }), "{err:?}");
    }
}
