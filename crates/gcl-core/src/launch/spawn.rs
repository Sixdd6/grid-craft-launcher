//! Starts the game process and streams its output to the event sink and a log file.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::Error;
use super::command::LaunchCommand;
use super::log4j::{EventParser, LogRecord};
use crate::events::{Event, EventSink, LogLevel};

/// A shared handle to the game's child process.
///
/// [`wait`] and a stop both reach the process through this. It holds `None` once the game
/// has exited and [`wait`] has dropped the child.
pub type ChildHandle = Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>;

/// How often [`wait`] checks whether the game has exited.
///
/// It polls instead of awaiting the child, so the lock on [`ChildHandle`] is free between
/// checks and a stop can take it while the game runs.
const POLL: Duration = Duration::from_millis(100);

/// A running game process and the tasks draining its output.
#[derive(Debug)]
pub struct RunningGame {
    /// The child process, shared with whoever wants to stop the game.
    child: ChildHandle,
    /// Process id of the game, read at spawn. `None` if it exited before it was read.
    pub pid: Option<u32>,
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
        pid: child.id(),
        child: Arc::new(tokio::sync::Mutex::new(Some(child))),
        log_path,
        program: cmd.program.clone(),
        tasks,
        sink,
    })
}

impl RunningGame {
    /// The shared child handle, for a caller that wants to stop the game.
    pub fn child(&self) -> ChildHandle {
        self.child.clone()
    }
}

/// Asks the game to exit.
///
/// Unix sends `SIGTERM` to the child, which the JVM turns into a clean shutdown; a process
/// that has already gone is not an error. Other platforms have no signal, so the child is
/// killed through its handle, the same as [`force_stop`].
///
/// This is [`Error::AlreadyExited`] once [`wait`] has reaped the child, so no signal ever
/// reaches a pid the system has handed to something else.
pub async fn request_stop(child: &ChildHandle, pid: Option<u32>) -> Result<(), Error> {
    #[cfg(unix)]
    {
        signal_live(child, pid, nix::sys::signal::Signal::SIGTERM).await
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        kill_child(child).await
    }
}

/// Kills the game outright: `SIGKILL` on unix, a kill through the handle elsewhere.
///
/// This is [`Error::AlreadyExited`] once [`wait`] has reaped the child, the same as
/// [`request_stop`].
pub async fn force_stop(child: &ChildHandle, pid: Option<u32>) -> Result<(), Error> {
    #[cfg(unix)]
    {
        signal_live(child, pid, nix::sys::signal::Signal::SIGKILL).await
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        kill_child(child).await
    }
}

/// Signals the game only while the child is still ours.
///
/// The handle lock is held across the signal, so [`wait`] cannot reap the child in between.
/// An empty handle means the child is reaped and its pid is free for reuse: nothing is
/// signalled and the call is [`Error::AlreadyExited`].
#[cfg(unix)]
async fn signal_live(
    child: &ChildHandle,
    pid: Option<u32>,
    sig: nix::sys::signal::Signal,
) -> Result<(), Error> {
    let guard = child.lock().await;
    let running = guard.as_ref().ok_or(Error::AlreadyExited)?;
    let pid = running.id().or(pid).ok_or(Error::AlreadyExited)?;
    signal(pid, sig)
}

/// Sends one signal to `pid`. A pid that is already gone (`ESRCH`) is a success: the
/// caller wanted the process stopped, and it is.
#[cfg(unix)]
fn signal(pid: u32, signal: nix::sys::signal::Signal) -> Result<(), Error> {
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), signal) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(errno) => Err(Error::Stop {
            pid,
            source: std::io::Error::from_raw_os_error(errno as i32),
        }),
    }
}

/// Kills the child through its handle, for a platform with no signals.
#[cfg(not(unix))]
async fn kill_child(child: &ChildHandle) -> Result<(), Error> {
    let mut guard = child.lock().await;
    let running = guard.as_mut().ok_or(Error::AlreadyExited)?;
    let pid = running.id().unwrap_or_default();
    running
        .start_kill()
        .map_err(|source| Error::Stop { pid, source })
}

/// The exit code to report for a finished process.
///
/// A process killed by a signal has no exit code of its own. Unix shells report `128 +
/// signal` for one, so a `SIGTERM` reads as 143 and matches what a user sees anywhere else.
/// A platform with no signals, and a status with neither, still reports `-1`.
#[cfg(unix)]
fn exit_code(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(-1)
}

/// The exit code to report for a finished process. No signals here, so it is the code or `-1`.
#[cfg(not(unix))]
fn exit_code(status: &std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

/// Waits for the game to exit, drains the rest of its output, and returns the exit code.
///
/// A process killed by a signal reports `128 + signal`, so a `SIGTERM` is 143. The child is
/// dropped before this returns, so a stop that comes afterwards finds nothing to signal.
pub async fn wait(game: RunningGame) -> Result<i32, Error> {
    let RunningGame {
        child,
        pid: _,
        log_path: _,
        program,
        tasks,
        sink,
    } = game;
    let waited = loop {
        let mut guard = child.lock().await;
        let Some(running) = guard.as_mut() else {
            break Err(std::io::Error::other("the game process is already gone"));
        };
        match running.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(err) => break Err(err),
        }
        drop(guard);
        tokio::time::sleep(POLL).await;
    };
    // The child is dropped here, so a later stop reports "not running" rather than
    // signalling a pid the system has already handed to something else.
    *child.lock().await = None;
    // The reader tasks are joined either way: a failed wait still leaves two tasks holding
    // the output pipes, and dropping them would lose the last lines of the log.
    for task in tasks {
        if let Err(err) = task.await {
            tracing::warn!(%err, "game log task did not finish cleanly");
        }
    }
    let status = waited.map_err(|source| Error::Spawn { program, source })?;
    let code = exit_code(&status);
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
///
/// Every line goes through an [`EventParser`] first, so the game's log4j XML becomes the plain
/// `[HH:MM:SS] [thread/LEVEL]: message` lines the log file, the sink, and
/// [`crate::launch::crash_hint`] all read. A line that is not part of an event is forwarded as
/// it came, at `level`.
fn read_lines<R>(
    stream: R,
    level: LogLevel,
    tx: UnboundedSender<(LogLevel, String)>,
) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut parser = EventParser::new(level);
        let mut lines = BufReader::new(stream).lines();
        // Reused for every line, so a line that is not part of an event allocates no vector.
        let mut records = Vec::new();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    records.clear();
                    parser.push_into(&line, &mut records);
                    if !send_all(&tx, records.drain(..)) {
                        return;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(%err, "could not read game output");
                    break;
                }
            }
        }
        send_all(&tx, parser.flush());
    })
}

/// Sends every record on. `false` means the receiver is gone and reading should stop.
fn send_all(
    tx: &UnboundedSender<(LogLevel, String)>,
    records: impl IntoIterator<Item = LogRecord>,
) -> bool {
    for record in records {
        if tx.send((record.level, record.text)).is_err() {
            return false;
        }
    }
    true
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
    async fn log4j_events_reach_the_file_and_the_sink_as_plain_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("logs/latest.log");
        let event = "  <log4j:Event logger=\"FabricLoader\" timestamp=\"1788800335568\" \
             level=\"WARN\" thread=\"main\">\n\
             <log4j:Message><![CDATA[Mappings not present!]]></log4j:Message>\n\
             </log4j:Event>";
        let cmd = LaunchCommand {
            program: PathBuf::from("sh"),
            args: vec!["-c".to_string(), format!("cat <<'XML'\n{event}\nXML")],
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let game = spawn(&cmd, log_path.clone(), tx).await.expect("spawn");
        assert_eq!(wait(game).await.expect("wait"), 0);

        let mut logs = Vec::new();
        while let Ok(Event::Log { level, message }) = rx.try_recv() {
            logs.push((level, message));
        }
        logs.pop().expect("summary line");
        assert_eq!(logs.len(), 1, "{logs:?}");
        assert_eq!(logs[0].0, LogLevel::Warn, "the event's own level");
        assert!(
            logs[0].1.ends_with("[main/WARN]: Mappings not present!"),
            "{:?}",
            logs[0].1
        );

        let written = std::fs::read_to_string(&log_path).expect("read log");
        assert!(!written.contains("log4j"), "{written}");
        assert!(written.contains("Mappings not present!\n"), "{written}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_after_the_child_is_reaped_signals_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cmd = LaunchCommand {
            program: PathBuf::from("sh"),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
        };
        let game = spawn(
            &cmd,
            dir.path().join("logs/latest.log"),
            crate::events::null_sink(),
        )
        .await
        .expect("spawn");
        let child = game.child();
        // The pid the launcher registry would have kept. The system is free to hand it to
        // another process once `wait` reaps the child.
        let pid = game.pid;
        assert_eq!(wait(game).await.expect("wait"), 0);
        assert!(child.lock().await.is_none(), "wait should drop the child");

        let err = request_stop(&child, pid)
            .await
            .expect_err("nothing to stop");
        assert!(matches!(err, Error::AlreadyExited), "{err:?}");
        let err = force_stop(&child, pid).await.expect_err("nothing to kill");
        assert!(matches!(err, Error::AlreadyExited), "{err:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_killed_by_sigterm_reports_143() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cmd = LaunchCommand {
            // No `trap`, so the shell really dies of the signal and leaves no exit code.
            program: PathBuf::from("sh"),
            args: vec!["-c".to_string(), "sleep 30".to_string()],
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
        };
        let game = spawn(
            &cmd,
            dir.path().join("logs/latest.log"),
            crate::events::null_sink(),
        )
        .await
        .expect("spawn");
        let child = game.child();
        let pid = game.pid;
        let waiting = tokio::spawn(async move { wait(game).await });
        request_stop(&child, pid).await.expect("send SIGTERM");
        let code = waiting.await.expect("join the wait").expect("wait");
        assert_eq!(code, 143, "128 + SIGTERM, not the -1 a missing code gives");
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
