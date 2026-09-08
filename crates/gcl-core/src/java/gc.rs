//! Which garbage collectors a JVM was built with.
//!
//! [`probe`] runs one JVM with `-XX:+PrintFlagsFinal -version` and reads the flag names out of
//! the dump. A flag name being present means that build compiled the collector in; the value
//! only says which collector is active by default. [`ProbeCache`] keeps the answer in memory and
//! in `cache/runtimes/gc-probe.json`, both keyed by the java binary's path and modification time.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::Error;
use super::detect::parse_java_version;
use crate::paths::Root;

/// How long one `java -XX:+PrintFlagsFinal -version` run may take.
///
/// Longer than `detect`'s ten seconds for a bare `-version`: this run prints the whole flag
/// table, which a cold JVM on a slow disk takes noticeably longer to produce.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// How much of a failing JVM's stderr the error message carries.
const STDERR_CHARS: usize = 200;

/// The flag names the probe records. Everything else in the dump is dropped.
const FLAG_NAMES: &[&str] = &[
    "UseSerialGC",
    "UseParallelGC",
    "UseG1GC",
    "UseZGC",
    "ZGenerational",
    "UseShenandoahGC",
];

/// Every JVM flag name a probe keeps, in the order the picker lists collectors.
///
/// `instances::model::supported_presets` reads these names, plus the major version, to decide
/// which `GcPreset` one JVM can run. It is a different list from the collector flags
/// `launch::gc_conflict` refuses in an instance's extra arguments: that one also carries names
/// no modern JVM still offers, which a probe would therefore never report.
pub fn supported_flag_names() -> &'static [&'static str] {
    FLAG_NAMES
}

/// What one JVM reported: its version and the collector flags its build carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcSupport {
    /// Major version: 17, 21, 25.
    pub major: u32,
    /// Full version string as the `-version` banner prints it.
    pub version: String,
    /// Flag names from [`supported_flag_names`] this JVM's dump listed.
    pub flags: BTreeSet<String>,
}

impl GcSupport {
    /// True when the JVM's flag dump listed `flag`.
    pub fn supports(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }
}

/// Runs one JVM's flag dump and reads its version and collector flags.
#[tracing::instrument]
pub async fn probe(java: &Path) -> Result<GcSupport, Error> {
    probe_with_timeout(java, PROBE_TIMEOUT).await
}

/// [`probe`] with the timeout given, so a test does not wait the real twenty seconds.
async fn probe_with_timeout(java: &Path, timeout: Duration) -> Result<GcSupport, Error> {
    let fail = |reason: String| Error::Probe {
        path: java.to_path_buf(),
        reason,
    };
    let run = tokio::process::Command::new(java)
        .arg("-XX:+PrintFlagsFinal")
        .arg("-version")
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| fail("timed out".to_string()))?
        .map_err(|err| fail(err.to_string()))?;

    if !output.status.success() {
        let code = match output.status.code() {
            Some(code) => format!("exit code {code}"),
            None => "no exit code, it was killed by a signal".to_string(),
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(fail(format!(
            "{code}: {}",
            first_chars(stderr.trim(), STDERR_CHARS)
        )));
    }

    // The banner goes to stderr and the flag dump to stdout; the parser reads one text. The
    // newline keeps the last stderr line from running into the first dump line.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    parse(&text).ok_or_else(|| fail("no java version banner in the output".to_string()))
}

/// The first `max` characters of `text`, cut on a character boundary.
fn first_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Reads a whole `-XX:+PrintFlagsFinal -version` text.
fn parse(text: &str) -> Option<GcSupport> {
    let (major, version) = parse_banner(text)?;
    Some(GcSupport {
        major,
        version,
        flags: parse_flags(text),
    })
}

/// Reads the major and full version out of the `openjdk version "21.0.7"` banner line.
fn parse_banner(text: &str) -> Option<(u32, String)> {
    text.lines().find_map(|line| {
        let (_, rest) = line.split_once(" version \"")?;
        let (value, _) = rest.split_once('"')?;
        parse_java_version(value)
    })
}

/// Collects the flag names of the `bool` flags the dump lists, keeping only known ones.
///
/// A dump line is whitespace columns: the C++ type, the name, `=`, the value, then its origin
/// tags. Names are right-aligned to the longest one in the file, so this reads tokens rather
/// than fixed columns.
fn parse_flags(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let mut tokens = line.split_ascii_whitespace();
        if tokens.next() != Some("bool") {
            continue;
        }
        let Some(name) = tokens.next() else {
            continue;
        };
        if FLAG_NAMES.contains(&name) {
            out.insert(name.to_string());
        }
    }
    out
}

/// One probed JVM as the disk cache stores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    /// Canonical path of the java binary.
    path: PathBuf,
    /// Its modification time, nanoseconds since the unix epoch.
    mtime_ns: i64,
    /// What the probe found.
    support: GcSupport,
}

/// Remembers probe results per java binary, in memory and on disk.
///
/// An entry is keyed by the binary's canonical path and its modification time, so a runtime
/// replaced in place is probed again instead of answered from a stale entry. A binary whose
/// metadata cannot be read has no key, and is probed every time rather than cached wrongly.
#[derive(Debug, Default)]
pub struct ProbeCache {
    memory: Mutex<HashMap<Key, GcSupport>>,
    /// One gate per key being probed right now, so two callers run java once between them.
    inflight: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<()>>>>,
}

/// What a cached probe is keyed by: the canonical java path and its modification time.
type Key = (PathBuf, i64);

impl ProbeCache {
    /// An empty cache. The disk file is read on the first miss, not here.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers from memory, then from disk, then by running the JVM.
    pub async fn get_or_probe(&self, root: &Root, java: &Path) -> Result<GcSupport, Error> {
        let path = std::fs::canonicalize(java).unwrap_or_else(|_| java.to_path_buf());
        let key = mtime_ns(&path).map(|mtime| (path.clone(), mtime));
        self.get_or_probe_keyed(root, &path, key).await
    }

    /// [`ProbeCache::get_or_probe`] with the cache key already made, or `None` for no cache.
    async fn get_or_probe_keyed(
        &self,
        root: &Root,
        java: &Path,
        key: Option<Key>,
    ) -> Result<GcSupport, Error> {
        // No modification time means no key that a later replacement could invalidate, so this
        // answer is never remembered: a stale entry would outlive the binary it describes.
        let Some(key) = key else {
            return probe(java).await;
        };
        if let Some(hit) = self.remembered(&key) {
            return Ok(hit);
        }
        let gate = self.gate(&key);
        let result = {
            let _held = gate.lock().await;
            match self.remembered(&key) {
                // Another caller probed the same binary while this one waited on the gate.
                Some(hit) => Ok(hit),
                None => self.load_or_run(root, java, &key).await,
            }
        };
        self.release(&key, gate);
        result
    }

    /// Reads the disk cache, else runs the JVM, and remembers whichever answered.
    async fn load_or_run(&self, root: &Root, java: &Path, key: &Key) -> Result<GcSupport, Error> {
        let file = cache_file(root);
        let mut records = load(&file);
        if let Some(hit) = records
            .iter()
            .find(|r| r.path == key.0 && r.mtime_ns == key.1)
            .map(|r| r.support.clone())
        {
            self.remember(key.clone(), hit.clone());
            return Ok(hit);
        }
        let support = probe(java).await?;
        records.retain(|r| r.path != key.0);
        records.push(Record {
            path: key.0.clone(),
            mtime_ns: key.1,
            support: support.clone(),
        });
        if let Err(err) = save(&file, &records) {
            tracing::warn!(%err, "cannot write the gc probe cache");
        }
        self.remember(key.clone(), support.clone());
        Ok(support)
    }

    /// The gate for one key, making it when this is the first caller to ask.
    fn gate(&self, key: &Key) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.inflight
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .entry(key.clone())
                .or_default(),
        )
    }

    /// Drops the gate for one key once no other caller still holds it.
    fn release(&self, key: &Key, gate: Arc<tokio::sync::Mutex<()>>) {
        let mut map = self.inflight.lock().unwrap_or_else(|err| err.into_inner());
        // Two references left means the map's and this one's: nobody else is waiting.
        if Arc::strong_count(&gate) == 2 {
            map.remove(key);
        }
    }

    /// Reads one entry out of the in-memory map.
    fn remembered(&self, key: &Key) -> Option<GcSupport> {
        self.lock().get(key).cloned()
    }

    /// Puts one entry into the in-memory map.
    fn remember(&self, key: Key, support: GcSupport) {
        self.lock().insert(key, support);
    }

    /// Takes the map lock, keeping the contents when another thread panicked holding it.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Key, GcSupport>> {
        self.memory.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// Where the disk cache lives.
fn cache_file(root: &Root) -> PathBuf {
    root.runtimes_dir().join("gc-probe.json")
}

/// Modification time in nanoseconds since the epoch, or `None` when it cannot be read.
///
/// `None` means the binary gets no cache entry at all: a made-up time would key an answer that
/// no later replacement of that binary could invalidate.
fn mtime_ns(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
}

/// Reads the disk cache. A missing or unreadable file is an empty cache, not an error.
fn load(file: &Path) -> Vec<Record> {
    let Ok(bytes) = std::fs::read(file) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        tracing::debug!(%err, "ignoring an unreadable gc probe cache");
        Vec::new()
    })
}

/// Rewrites the disk cache in one atomic replace.
fn save(file: &Path, records: &[Record]) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(records).map_err(|err| Error::Io {
        path: file.to_path_buf(),
        source: std::io::Error::other(err),
    })?;
    Ok(crate::paths::write_atomic(file, &bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP_17: &str = include_str!("../../../../tests/fixtures/java/printflags-17.txt");
    const DUMP_21: &str = include_str!("../../../../tests/fixtures/java/printflags-21.txt");
    const DUMP_25: &str = include_str!("../../../../tests/fixtures/java/printflags-25.txt");

    fn parsed(dump: &str) -> GcSupport {
        parse(dump).expect("the fixture parses")
    }

    #[test]
    fn reads_the_java_17_dump() {
        let support = parsed(DUMP_17);
        assert_eq!(support.major, 17);
        assert_eq!(support.version, "17.0.15");
        for flag in [
            "UseSerialGC",
            "UseParallelGC",
            "UseG1GC",
            "UseZGC",
            "UseShenandoahGC",
        ] {
            assert!(support.supports(flag), "17 should list {flag}: {support:?}");
        }
        assert!(
            !support.supports("ZGenerational"),
            "17 has no generational ZGC flag"
        );
    }

    #[test]
    fn reads_the_java_21_dump() {
        let support = parsed(DUMP_21);
        assert_eq!(support.major, 21);
        assert_eq!(support.version, "21.0.7");
        assert!(support.supports("UseZGC"));
        assert!(
            support.supports("ZGenerational"),
            "21 carries the generational ZGC switch"
        );
        assert!(support.supports("UseShenandoahGC"));
    }

    #[test]
    fn reads_the_java_25_dump() {
        let support = parsed(DUMP_25);
        assert_eq!(support.major, 25);
        assert_eq!(support.version, "25.0.4.1");
        assert!(support.supports("UseZGC"));
        assert!(
            !support.supports("ZGenerational"),
            "25 dropped the switch: ZGC is generational there"
        );
        assert!(support.supports("UseShenandoahGC"));
    }

    #[test]
    fn keeps_only_the_flag_names_it_was_asked_for() {
        let support = parsed(DUMP_17);
        assert!(!support.supports("UseGCOverheadLimit"), "{support:?}");
        for flag in &support.flags {
            assert!(supported_flag_names().contains(&flag.as_str()), "{flag}");
        }
    }

    #[test]
    fn a_non_bool_flag_line_is_not_a_flag() {
        let text =
            "openjdk version \"21.0.7\" 2025-04-15 LTS\n   ccstr UseZGC = adaptive {product}\n";
        assert!(!parsed(text).supports("UseZGC"));
    }

    #[test]
    fn output_with_no_banner_does_not_parse() {
        assert!(parse("     bool UseZGC = false {product}\n").is_none());
        assert!(parse("").is_none());
        assert!(parse("openjdk version \"banana\"\n").is_none());
    }

    #[cfg(unix)]
    mod process {
        use super::*;
        use std::time::SystemTime;

        /// Writes an executable `sh` script.
        fn script(path: &Path, body: &str) {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(path, body).expect("write the script");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("make the script executable");
        }

        /// A stand-in java that prints `dump` and appends one byte to `counter` per run.
        fn fake_java(dir: &Path, dump: &str) -> (PathBuf, PathBuf) {
            let dump_file = dir.join("dump.txt");
            std::fs::write(&dump_file, dump).expect("write the dump");
            let counter = dir.join("runs");
            let java = dir.join("fake-java");
            script(
                &java,
                &format!(
                    "#!/bin/sh\nprintf x >> {}\ncat {}\n",
                    counter.display(),
                    dump_file.display()
                ),
            );
            (java, counter)
        }

        /// How many times the stand-in java ran.
        fn runs(counter: &Path) -> usize {
            std::fs::read(counter).map(|b| b.len()).unwrap_or(0)
        }

        /// Sets a file's modification time, so a rewrite can keep or move the cache key.
        fn set_mtime(path: &Path, time: SystemTime) {
            std::fs::File::options()
                .write(true)
                .open(path)
                .expect("open for mtime")
                .set_modified(time)
                .expect("set mtime");
        }

        #[tokio::test]
        async fn probes_a_stand_in_java() {
            let dir = tempfile::tempdir().expect("tempdir");
            let (java, counter) = fake_java(dir.path(), DUMP_21);
            let support = probe(&java).await.expect("probe");
            assert_eq!(support.major, 21);
            assert!(support.supports("ZGenerational"));
            assert_eq!(runs(&counter), 1);
        }

        #[tokio::test]
        async fn probing_a_missing_binary_is_an_error() {
            let dir = tempfile::tempdir().expect("tempdir");
            let missing = dir.path().join("no-such-java");
            assert!(matches!(probe(&missing).await, Err(Error::Probe { .. })));
        }

        #[tokio::test]
        async fn a_java_that_never_answers_times_out() {
            let dir = tempfile::tempdir().expect("tempdir");
            let java = dir.path().join("slow-java");
            script(&java, "#!/bin/sh\nsleep 30\n");
            let err = probe_with_timeout(&java, Duration::from_millis(200))
                .await
                .expect_err("a hanging java must not block forever");
            match err {
                Error::Probe { reason, .. } => assert_eq!(reason, "timed out"),
                other => panic!("wrong error: {other:?}"),
            }
        }

        #[tokio::test]
        async fn the_cache_runs_java_once_per_mtime() {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = Root::from_path(dir.path());
            let (java, counter) = fake_java(dir.path(), DUMP_17);
            let cache = ProbeCache::new();

            let first = cache.get_or_probe(&root, &java).await.expect("probe");
            let second = cache.get_or_probe(&root, &java).await.expect("cached");
            assert_eq!(first, second);
            assert_eq!(runs(&counter), 1, "the second ask must not spawn java");

            // A runtime replaced in place must be probed again.
            let moved = SystemTime::now() + Duration::from_secs(60);
            set_mtime(&java, moved);
            let third = cache.get_or_probe(&root, &java).await.expect("reprobe");
            assert_eq!(first, third);
            assert_eq!(runs(&counter), 2, "a new mtime must spawn java again");
        }

        #[tokio::test]
        async fn a_second_cache_reloads_the_answer_from_disk() {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = Root::from_path(dir.path());
            let (java, counter) = fake_java(dir.path(), DUMP_25);

            let first = ProbeCache::new()
                .get_or_probe(&root, &java)
                .await
                .expect("probe");
            assert_eq!(runs(&counter), 1);
            assert!(
                root.runtimes_dir().join("gc-probe.json").is_file(),
                "the probe must be written to cache/runtimes/gc-probe.json"
            );

            // Break the script but keep its mtime: only a disk hit can answer now.
            let mtime = std::fs::metadata(&java)
                .and_then(|m| m.modified())
                .expect("mtime");
            script(&java, "#!/bin/sh\nexit 1\n");
            set_mtime(&java, mtime);

            let reloaded = ProbeCache::new()
                .get_or_probe(&root, &java)
                .await
                .expect("the disk cache answers");
            assert_eq!(reloaded, first);
            assert_eq!(runs(&counter), 1, "nothing new was spawned");
        }

        #[tokio::test]
        async fn a_corrupt_cache_file_is_ignored() {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = Root::from_path(dir.path());
            let (java, counter) = fake_java(dir.path(), DUMP_21);
            let file = root.runtimes_dir().join("gc-probe.json");
            std::fs::create_dir_all(root.runtimes_dir()).expect("mkdir");
            std::fs::write(&file, b"not json").expect("write");

            let support = ProbeCache::new()
                .get_or_probe(&root, &java)
                .await
                .expect("probe");
            assert_eq!(support.major, 21);
            assert_eq!(runs(&counter), 1);
            let bytes = std::fs::read(&file).expect("read back");
            let records: Vec<Record> = serde_json::from_slice(&bytes).expect("rewritten as json");
            assert_eq!(records.len(), 1);
        }

        #[tokio::test]
        async fn a_java_that_exits_non_zero_reports_its_code_and_stderr() {
            let dir = tempfile::tempdir().expect("tempdir");
            let java = dir.path().join("angry-java");
            let long = "a".repeat(200);
            script(
                &java,
                &format!("#!/bin/sh\nprintf '{long}TAIL' >&2\nexit 3\n"),
            );
            let err = probe(&java)
                .await
                .expect_err("a non-zero exit is a failure");
            match err {
                Error::Probe { reason, .. } => {
                    assert!(reason.contains("exit code 3"), "{reason}");
                    assert!(reason.contains(&long), "{reason}");
                    assert!(
                        !reason.contains("TAIL"),
                        "stderr must be cut at 200: {reason}"
                    );
                }
                other => panic!("wrong error: {other:?}"),
            }
        }

        #[tokio::test]
        async fn two_asks_for_one_java_run_it_once() {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = Root::from_path(dir.path());
            let dump_file = dir.path().join("dump.txt");
            std::fs::write(&dump_file, DUMP_21).expect("write the dump");
            let counter = dir.path().join("runs");
            let java = dir.path().join("slow-java");
            script(
                &java,
                &format!(
                    "#!/bin/sh\nprintf x >> {}\nsleep 0.3\ncat {}\n",
                    counter.display(),
                    dump_file.display()
                ),
            );
            let cache = ProbeCache::new();
            let (first, second) = tokio::join!(
                cache.get_or_probe(&root, &java),
                cache.get_or_probe(&root, &java)
            );
            assert_eq!(first.expect("first"), second.expect("second"));
            assert_eq!(runs(&counter), 1, "both asks must share one probe");
        }

        #[tokio::test]
        async fn a_java_whose_metadata_cannot_be_read_is_not_cached() {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = Root::from_path(dir.path());
            let (java, counter) = fake_java(dir.path(), DUMP_17);
            let cache = ProbeCache::new();

            // `None` is what an unreadable metadata call gives: no cache key, so no cache.
            cache
                .get_or_probe_keyed(&root, &java, None)
                .await
                .expect("first");
            cache
                .get_or_probe_keyed(&root, &java, None)
                .await
                .expect("second");
            assert_eq!(runs(&counter), 2, "an unkeyed probe must not be cached");
            assert!(
                !root.runtimes_dir().join("gc-probe.json").exists(),
                "nothing may be written without a key"
            );
        }

        #[test]
        fn a_missing_file_has_no_cache_key() {
            let dir = tempfile::tempdir().expect("tempdir");
            assert_eq!(mtime_ns(&dir.path().join("no-such-java")), None);
        }
    }
}
