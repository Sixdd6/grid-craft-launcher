//! Running Forge and NeoForge installer processors as child JVM processes.

use std::path::{Path, PathBuf};

use super::Error;
use super::forgelike::{DataMap, InstallProfile, Processor, library_path, runs_on_client};
use crate::download::hash::sha1_file;
use crate::events::{EventSink, TaskHandle};
use crate::mojang::version::MavenCoord;
use crate::paths::Root;

/// Runs one installer processor and reports the exit code it finished with.
#[async_trait::async_trait]
pub trait ProcessRunner: Send + Sync {
    /// Runs `java -cp <classpath> <main> <args>`, writing the process output to `log`.
    async fn run(
        &self,
        java: &Path,
        classpath: &[PathBuf],
        main: &str,
        args: &[String],
        log: &Path,
    ) -> Result<i32, std::io::Error>;
}

/// Runs processors as real child JVMs, appending stdout and stderr to the log file.
pub struct JavaRunner;

#[async_trait::async_trait]
impl ProcessRunner for JavaRunner {
    async fn run(
        &self,
        java: &Path,
        classpath: &[PathBuf],
        main: &str,
        args: &[String],
        log: &Path,
    ) -> Result<i32, std::io::Error> {
        if let Some(parent) = log.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)?;
        let err = out.try_clone()?;
        let status = tokio::process::Command::new(java)
            .arg("-cp")
            .arg(join_classpath(classpath))
            .arg(main)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(out))
            .stderr(std::process::Stdio::from(err))
            .status()
            .await?;
        // A process killed by a signal has no exit code; report it as -1.
        Ok(status.code().unwrap_or(-1))
    }
}

/// Joins classpath entries with the platform's separator.
fn join_classpath(classpath: &[PathBuf]) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    classpath
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(sep)
}

/// Runs every client-side processor of a profile in order.
///
/// A processor whose declared outputs all exist with the right sha1 is skipped, so a repeated
/// install does no work. Each run writes `<index>-<artifact>.log` under `log_dir`.
#[tracing::instrument(skip_all, fields(processors = profile.processors.len()))]
pub async fn run_processors(
    runner: &dyn ProcessRunner,
    java: &Path,
    profile: &InstallProfile,
    data: &DataMap,
    root: &Root,
    log_dir: &Path,
    sink: &EventSink,
) -> Result<(), Error> {
    for (index, processor) in profile.processors.iter().enumerate() {
        if !runs_on_client(&processor.sides) {
            continue;
        }
        let coord = MavenCoord::parse(&processor.jar)?;
        let outputs = resolve_outputs(processor, data, root)?;
        if !outputs.is_empty() && first_bad_output(&outputs).await.is_none() {
            tracing::debug!(jar = %processor.jar, "processor outputs are current, skipping");
            continue;
        }

        let jar = library_path(&processor.jar, root)?;
        let main = main_class(jar.clone(), processor.jar.clone()).await?;
        let mut classpath = Vec::with_capacity(processor.classpath.len() + 1);
        classpath.push(jar);
        for entry in &processor.classpath {
            classpath.push(library_path(entry, root)?);
        }
        let mut args = Vec::with_capacity(processor.args.len());
        for arg in &processor.args {
            args.push(crate::loaders::forgelike::substitute(arg, data, root)?);
        }
        let log = log_dir.join(format!("{index}-{}.log", coord.artifact));

        let task = TaskHandle::start(sink, format!("processor {}", coord.artifact), None);
        let code = match runner.run(java, &classpath, &main, &args, &log).await {
            Ok(code) => code,
            Err(source) => {
                task.fail(source.to_string());
                return Err(Error::Io { path: log, source });
            }
        };
        if code != 0 {
            task.fail(format!("exit {code}"));
            return Err(Error::ProcessorFailed { main, code, log });
        }
        if let Some(path) = first_bad_output(&outputs).await {
            task.fail(format!("missing output {}", path.display()));
            return Err(Error::ProcessorOutput { path });
        }
        task.finish();
    }
    Ok(())
}

/// Resolves a processor's `outputs` into `(path, expected sha1)` pairs.
fn resolve_outputs(
    processor: &Processor,
    data: &DataMap,
    root: &Root,
) -> Result<Vec<(PathBuf, String)>, Error> {
    let mut outputs = Vec::with_capacity(processor.outputs.len());
    for (path, sha1) in &processor.outputs {
        outputs.push((
            PathBuf::from(crate::loaders::forgelike::substitute(path, data, root)?),
            crate::loaders::forgelike::substitute(sha1, data, root)?,
        ));
    }
    Ok(outputs)
}

/// Returns the first output that is missing or whose sha1 does not match.
async fn first_bad_output(outputs: &[(PathBuf, String)]) -> Option<PathBuf> {
    let owned = outputs.to_vec();
    tokio::task::spawn_blocking(move || {
        owned
            .into_iter()
            .find(|(path, want)| !matches_sha1(path, want))
            .map(|(path, _)| path)
    })
    .await
    .unwrap_or_else(|_| outputs.first().map(|(path, _)| path.clone()))
}

/// True when the file at `path` hashes to `want`. A missing or unreadable file is false.
fn matches_sha1(path: &Path, want: &str) -> bool {
    sha1_file(path).is_ok_and(|got| got.eq_ignore_ascii_case(want))
}

/// Reads a processor jar's `Main-Class`, off the async thread.
async fn main_class(jar: PathBuf, coord: String) -> Result<String, Error> {
    let found = tokio::task::spawn_blocking(move || {
        super::forgelike::InstallerJar::open(jar)?.manifest_main_class()
    })
    .await
    .map_err(|source| Error::Io {
        path: PathBuf::from(&coord),
        source: std::io::Error::other(source),
    })??;
    found.ok_or(Error::ProcessorMain(coord))
}

#[cfg(test)]
#[path = "processors_tests.rs"]
mod tests;
