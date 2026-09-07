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
        Ok(exit_code(&status))
    }
}

/// Exit code reported for a process a signal killed, which has no code of its own.
const SIGNALLED: i32 = -1;

/// Reads a finished process's exit code, mapping a signal death to [`SIGNALLED`].
fn exit_code(status: &std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(SIGNALLED)
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
/// A processor whose declared outputs all exist with the right sha1 — the profile's, or the one
/// recorded in an `<output>.sha1` sidecar by an earlier run — is skipped, so a repeated install
/// does no work. Each run writes `<index>-<artifact>.log` under `log_dir`.
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
        if !outputs.is_empty() {
            match first_bad_output(&outputs).await {
                None => {
                    tracing::debug!(jar = %processor.jar, "processor outputs are current, skipping");
                    continue;
                }
                Some(bad) => tracing::debug!(
                    jar = %processor.jar,
                    output = %bad.path().display(),
                    "processor output is missing or stale, running it"
                ),
            }
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
        if let Err(error) = verify_outputs(&outputs).await {
            task.fail(error.to_string());
            return Err(error);
        }
        task.finish();
    }
    Ok(())
}

/// True when every client-side processor's declared outputs are present with the right sha1.
///
/// A processor that declares no outputs is not checkable, so it does not count either way.
pub async fn outputs_current(
    profile: &InstallProfile,
    data: &DataMap,
    root: &Root,
) -> Result<bool, Error> {
    for processor in &profile.processors {
        if !runs_on_client(&processor.sides) {
            continue;
        }
        let outputs = resolve_outputs(processor, data, root)?;
        if outputs.is_empty() {
            continue;
        }
        if first_bad_output(&outputs).await.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
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

/// Why one declared output is not the file the install profile named.
enum BadOutput {
    /// Nothing readable is at the path.
    Missing(PathBuf),
    /// The file is there but hashes to something else.
    Mismatch {
        /// The output.
        path: PathBuf,
        /// The sha1 the install profile names.
        expected: String,
        /// The sha1 the file on disk has.
        actual: String,
    },
}

impl BadOutput {
    /// The output this verdict is about.
    fn path(&self) -> &Path {
        match self {
            BadOutput::Missing(path) => path,
            BadOutput::Mismatch { path, .. } => path,
        }
    }
}

/// Returns the first output that is missing or whose sha1 is neither the profile's nor accepted.
async fn first_bad_output(outputs: &[(PathBuf, String)]) -> Option<BadOutput> {
    let owned = outputs.to_vec();
    tokio::task::spawn_blocking(move || {
        owned
            .iter()
            .find_map(|(path, want)| check_output(path, want).err())
    })
    .await
    .unwrap_or_else(|_| {
        outputs
            .first()
            .map(|(path, _)| BadOutput::Missing(path.clone()))
    })
}

/// Checks one output against the profile's sha1, then against its accepted-hash sidecar.
fn check_output(path: &Path, want: &str) -> Result<(), BadOutput> {
    let Ok(actual) = sha1_file(path) else {
        return Err(BadOutput::Missing(path.to_path_buf()));
    };
    if actual.eq_ignore_ascii_case(want) || sidecar_holds(path, want, &actual) {
        return Ok(());
    }
    Err(BadOutput::Mismatch {
        path: path.to_path_buf(),
        expected: want.to_string(),
        actual,
    })
}

/// Path of the sidecar holding the sha1 an output was accepted with: `<output>.sha1`.
fn sidecar_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".sha1");
    PathBuf::from(name)
}

/// True when `<output>.sha1` binds this profile's hash to the hash the file has now.
///
/// The sidecar holds two lines, `{expected}\n{actual}\n`. Both have to match: the first
/// against the sha1 the install profile names right now, the second against the file on
/// disk. A sidecar written for an earlier profile — a new Forge or NeoForge build with
/// another expected hash for the same path — says nothing about this install, so it is
/// ignored and the processor runs again.
fn sidecar_holds(path: &Path, want: &str, actual: &str) -> bool {
    let Ok(noted) = std::fs::read_to_string(sidecar_path(path)) else {
        return false;
    };
    let mut lines = noted.lines();
    let (Some(expected_line), Some(actual_line)) = (lines.next(), lines.next()) else {
        return false;
    };
    expected_line.trim().eq_ignore_ascii_case(want)
        && actual_line.trim().eq_ignore_ascii_case(actual)
}

/// True when an output is an archive the CRC check can read back entry by entry.
fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jar") || e.eq_ignore_ascii_case("zip"))
}

/// Verifies a finished processor's outputs, accepting a jar the host zlib recompressed.
///
/// A missing output is fatal. A hash mismatch is accepted when the archive still reads back
/// entry by entry with every CRC32 intact: Mojang's bundled `libzip` links against the host
/// `libz.so.1`, and zlib-ng deflates to a different byte stream than the one Forge hashed into
/// the install profile. The accepted hash is written to `<output>.sha1` so the next install
/// skips the processor instead of running it again.
async fn verify_outputs(outputs: &[(PathBuf, String)]) -> Result<(), Error> {
    let owned = outputs.to_vec();
    let first = outputs
        .first()
        .map(|(path, _)| path.clone())
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || verify_outputs_blocking(&owned))
        .await
        .map_err(|source| Error::Io {
            path: first,
            source: std::io::Error::other(source),
        })?
}

/// The blocking half of [`verify_outputs`]: hashing, reading archives, writing sidecars.
fn verify_outputs_blocking(outputs: &[(PathBuf, String)]) -> Result<(), Error> {
    for (path, want) in outputs {
        match check_output(path, want) {
            Ok(()) => {}
            Err(BadOutput::Missing(path)) => return Err(Error::ProcessorOutput { path }),
            Err(BadOutput::Mismatch {
                path,
                expected,
                actual,
            }) => {
                if !is_archive(&path) {
                    // Only a jar or a zip can be recompressed into other bytes with the
                    // same content. Anything else with another hash is simply wrong.
                    return Err(Error::ProcessorOutputHash {
                        path,
                        expected,
                        actual,
                    });
                }
                if let Err(source) = archive_reads_cleanly(&path) {
                    return Err(Error::ProcessorOutputDamaged {
                        path,
                        expected,
                        actual,
                        source,
                    });
                }
                tracing::warn!(
                    output = %path.display(),
                    %expected,
                    %actual,
                    "processor output has a different sha1 than the install profile, but every \
                     zip entry reads back with its CRC32 intact: the host zlib (zlib-ng on \
                     Fedora and Arch) deflates to different bytes than the stream Forge hashed. \
                     Accepting it"
                );
                write_sidecar(&path, &expected, &actual)?;
            }
        }
    }
    Ok(())
}

/// Reads every entry of a zip end to end, so a truncated or corrupt archive is an error.
///
/// The reader checks each entry's CRC32 against its header when the entry ends.
fn archive_reads_cleanly(path: &Path) -> Result<(), std::io::Error> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|source| std::io::Error::other(source.to_string()))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|source| std::io::Error::other(source.to_string()))?;
        std::io::copy(&mut entry, &mut std::io::sink())?;
    }
    Ok(())
}

/// Records the profile hash an output was accepted against, and the hash it really has.
///
/// Two lines, `{expected}\n{actual}\n`, written atomically so a killed install leaves no
/// half-written sidecar behind.
fn write_sidecar(path: &Path, expected: &str, actual: &str) -> Result<(), Error> {
    let sidecar = sidecar_path(path);
    crate::paths::write_atomic(&sidecar, format!("{expected}\n{actual}\n").as_bytes()).map_err(
        |source| Error::Io {
            path: sidecar,
            source: std::io::Error::other(source.to_string()),
        },
    )
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
