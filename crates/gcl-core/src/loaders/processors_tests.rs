//! Tests for the processor loop: skipping, failure, and output verification.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;

use super::*;
use crate::download::hash::sha1_hex;
use crate::events::{Event, null_sink};
use crate::loaders::forgelike::{DataMap, InstallProfile};
use crate::loaders::test_support::write_jar_with_main;

const ALPHA: &[u8] = b"alpha-output";
const BETA: &[u8] = b"beta-output";

/// One recorded `ProcessRunner::run` call.
#[derive(Debug, Clone)]
struct Call {
    main: String,
    classpath: Vec<PathBuf>,
    args: Vec<String>,
    log: PathBuf,
}

/// A runner that records its calls and writes the outputs a real processor would.
struct FakeRunner {
    calls: Mutex<Vec<Call>>,
    /// Files written when a processor with this main class runs.
    writes: BTreeMap<String, Vec<(PathBuf, Vec<u8>)>>,
    code: i32,
}

impl FakeRunner {
    fn new(writes: BTreeMap<String, Vec<(PathBuf, Vec<u8>)>>, code: i32) -> FakeRunner {
        FakeRunner {
            calls: Mutex::new(Vec::new()),
            writes,
            code,
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait::async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(
        &self,
        _java: &Path,
        classpath: &[PathBuf],
        main: &str,
        args: &[String],
        log: &Path,
    ) -> Result<i32, std::io::Error> {
        self.calls.lock().expect("lock").push(Call {
            main: main.to_string(),
            classpath: classpath.to_vec(),
            args: args.to_vec(),
            log: log.to_path_buf(),
        });
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(log, format!("ran {main}\n"))?;
        if self.code == 0 {
            for (path, bytes) in self.writes.get(main).into_iter().flatten() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, bytes)?;
            }
        }
        Ok(self.code)
    }
}

/// A root, its temp dir, and the two processor jars the fixtures below name.
struct Harness {
    _dir: TempDir,
    root: Root,
    profile: InstallProfile,
    data: DataMap,
}

impl Harness {
    fn new() -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        let libs = root.libraries_dir();
        write_jar_with_main(&libs.join("net/test/one/1/one-1.jar"), "net.test.One");
        write_jar_with_main(&libs.join("net/test/two/1/two-1.jar"), "net.test.Two");

        let profile: InstallProfile = serde_json::from_str(
            r#"{
                "spec": 1,
                "processors": [
                    {
                        "jar": "net.test:one:1",
                        "classpath": ["net.test:two:1"],
                        "args": ["--output", "{OUT_A}", "--side", "{SIDE}"],
                        "outputs": { "{OUT_A}": "{OUT_A_SHA}" }
                    },
                    {
                        "sides": ["client"],
                        "jar": "net.test:two:1",
                        "classpath": [],
                        "args": ["--output", "{OUT_B}"],
                        "outputs": { "{OUT_B}": "{OUT_B_SHA}" }
                    },
                    {
                        "sides": ["server"],
                        "jar": "net.test:one:1",
                        "args": ["--server-only"]
                    }
                ]
            }"#,
        )
        .expect("parse profile");

        let data = DataMap(BTreeMap::from([
            (
                "OUT_A".to_string(),
                libs.join("out/a.jar").display().to_string(),
            ),
            ("OUT_A_SHA".to_string(), sha1_hex(ALPHA)),
            (
                "OUT_B".to_string(),
                libs.join("out/b.jar").display().to_string(),
            ),
            ("OUT_B_SHA".to_string(), sha1_hex(BETA)),
            ("SIDE".to_string(), "client".to_string()),
        ]));

        Harness {
            _dir: dir,
            root,
            profile,
            data,
        }
    }

    fn out_a(&self) -> PathBuf {
        self.root.libraries_dir().join("out/a.jar")
    }

    fn out_b(&self) -> PathBuf {
        self.root.libraries_dir().join("out/b.jar")
    }

    fn log_dir(&self) -> PathBuf {
        self.root.logs_dir().join("forge-install")
    }

    fn writes(&self) -> BTreeMap<String, Vec<(PathBuf, Vec<u8>)>> {
        BTreeMap::from([
            (
                "net.test.One".to_string(),
                vec![(self.out_a(), ALPHA.to_vec())],
            ),
            (
                "net.test.Two".to_string(),
                vec![(self.out_b(), BETA.to_vec())],
            ),
        ])
    }

    async fn run(
        &self,
        runner: &dyn ProcessRunner,
        sink: &crate::events::EventSink,
    ) -> Result<(), Error> {
        run_processors(
            runner,
            Path::new("/usr/bin/java"),
            &self.profile,
            &self.data,
            &self.root,
            &self.log_dir(),
            sink,
        )
        .await
    }
}

#[tokio::test]
async fn the_first_run_executes_every_client_processor() {
    let h = Harness::new();
    let runner = FakeRunner::new(h.writes(), 0);

    h.run(&runner, &null_sink()).await.expect("run processors");

    let calls = runner.calls();
    assert_eq!(calls.len(), 2, "server-only processor must be skipped");
    assert_eq!(calls[0].main, "net.test.One");
    assert_eq!(
        calls[0].classpath,
        vec![
            h.root.libraries_dir().join("net/test/one/1/one-1.jar"),
            h.root.libraries_dir().join("net/test/two/1/two-1.jar"),
        ]
    );
    assert_eq!(
        calls[0].args,
        vec![
            "--output".to_string(),
            h.out_a().display().to_string(),
            "--side".to_string(),
            "client".to_string(),
        ]
    );
    assert_eq!(calls[0].log, h.log_dir().join("0-one.log"));
    assert_eq!(calls[1].main, "net.test.Two");
    assert_eq!(calls[1].log, h.log_dir().join("1-two.log"));
    assert!(h.out_a().exists() && h.out_b().exists());
}

#[tokio::test]
async fn a_second_run_skips_processors_whose_outputs_already_match() {
    let h = Harness::new();
    let first = FakeRunner::new(h.writes(), 0);
    h.run(&first, &null_sink()).await.expect("first run");
    assert_eq!(first.calls().len(), 2);

    let second = FakeRunner::new(h.writes(), 0);
    h.run(&second, &null_sink()).await.expect("second run");

    assert!(
        second.calls().is_empty(),
        "outputs are present and hashed right: {:?}",
        second.calls()
    );
}

#[tokio::test]
async fn a_stale_output_hash_makes_the_processor_run_again() {
    let h = Harness::new();
    let first = FakeRunner::new(h.writes(), 0);
    h.run(&first, &null_sink()).await.expect("first run");
    std::fs::write(h.out_b(), b"tampered").expect("tamper");

    let second = FakeRunner::new(h.writes(), 0);
    h.run(&second, &null_sink()).await.expect("second run");

    let mains: Vec<String> = second.calls().into_iter().map(|c| c.main).collect();
    assert_eq!(mains, vec!["net.test.Two".to_string()]);
}

#[tokio::test]
async fn a_non_zero_exit_reports_the_log_path() {
    let h = Harness::new();
    let runner = FakeRunner::new(h.writes(), 1);

    let err = h
        .run(&runner, &null_sink())
        .await
        .expect_err("exit 1 must fail the install");

    match err {
        Error::ProcessorFailed { main, code, log } => {
            assert_eq!(main, "net.test.One");
            assert_eq!(code, 1);
            assert_eq!(log, h.log_dir().join("0-one.log"));
            assert!(log.exists(), "the log the error names must be on disk");
        }
        other => panic!("expected ProcessorFailed, got {other:?}"),
    }
    assert_eq!(runner.calls().len(), 1, "the loop stops at the failure");
}

#[tokio::test]
async fn a_processor_that_writes_nothing_is_a_missing_output() {
    let h = Harness::new();
    let runner = FakeRunner::new(BTreeMap::new(), 0);

    let err = h
        .run(&runner, &null_sink())
        .await
        .expect_err("a silent processor must fail the install");

    match err {
        Error::ProcessorOutput { path } => assert_eq!(path, h.out_a()),
        other => panic!("expected ProcessorOutput, got {other:?}"),
    }
}

#[tokio::test]
async fn a_processor_jar_without_a_main_class_is_reported() {
    let h = Harness::new();
    crate::loaders::test_support::write_zip(
        &h.root.libraries_dir().join("net/test/one/1/one-1.jar"),
        &[("a.txt", b"x")],
    );
    let runner = FakeRunner::new(h.writes(), 0);

    let err = h
        .run(&runner, &null_sink())
        .await
        .expect_err("no main class");

    assert!(
        matches!(err, Error::ProcessorMain(ref j) if j == "net.test:one:1"),
        "{err:?}"
    );
}

#[tokio::test]
async fn each_processor_emits_a_started_and_finished_event() {
    let h = Harness::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner = FakeRunner::new(h.writes(), 0);

    h.run(&runner, &tx).await.expect("run processors");
    drop(tx);

    let mut started = 0;
    let mut finished = 0;
    while let Some(event) = rx.recv().await {
        match event {
            Event::TaskStarted { .. } => started += 1,
            Event::TaskFinished { .. } => finished += 1,
            _ => {}
        }
    }
    assert_eq!((started, finished), (2, 2));
}

#[test]
fn join_classpath_uses_the_platform_separator() {
    let entries = [PathBuf::from("/a/one.jar"), PathBuf::from("/b/two.jar")];
    let joined = join_classpath(&entries);
    let sep = if cfg!(windows) { ';' } else { ':' };
    assert_eq!(joined.matches(sep).count(), 1, "{joined}");
    assert!(joined.starts_with("/a/one.jar"), "{joined}");
    assert!(joined.ends_with("two.jar"), "{joined}");
    assert_eq!(join_classpath(&[]), "");
}

#[test]
#[cfg(unix)]
fn exit_code_reports_a_signal_death_as_minus_one() {
    use std::os::unix::process::ExitStatusExt;

    let killed = std::process::ExitStatus::from_raw(9);
    assert_eq!(killed.code(), None);
    assert_eq!(exit_code(&killed), SIGNALLED);
    // A normal exit keeps its own code. Raw status 0x100 is "exited with 1".
    assert_eq!(exit_code(&std::process::ExitStatus::from_raw(0x100)), 1);
}

/// Builds a zip holding one entry, deflated at `level`.
fn deflated_jar(level: i64) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(level))
            .last_modified_time(
                zip::DateTime::from_date_and_time(1989, 11, 26, 0, 0, 0).expect("time"),
            );
        w.start_file("assets/pack.txt", opts).expect("entry");
        w.write_all(&b"grid craft launcher forge processor payload ".repeat(400))
            .expect("write");
        w.finish().expect("finish");
    }
    buf.into_inner()
}

/// Mojang's `java-runtime-gamma` links `libzip.so` against the host `libz.so.1`. On a host
/// where that is zlib-ng, jarsplitter's deflate stream differs from the one Forge hashed into
/// `MC_EXTRA_SHA`, though every entry's name, order, CRC32 and size are identical. The output
/// check hashes the archive bytes, so it must fall back to reading the archive.
#[tokio::test]
async fn a_recompressed_output_with_identical_entries_is_accepted() {
    let forge_built = deflated_jar(6);
    let we_built = deflated_jar(9);
    assert_ne!(forge_built, we_built, "levels must produce different bytes");

    let h = Harness::new();
    let mut data = h.data.0.clone();
    data.insert("OUT_A_SHA".to_string(), sha1_hex(&forge_built));
    data.insert("OUT_B_SHA".to_string(), sha1_hex(BETA));
    let data = DataMap(data);

    let writes = BTreeMap::from([
        ("net.test.One".to_string(), vec![(h.out_a(), we_built)]),
        ("net.test.Two".to_string(), vec![(h.out_b(), BETA.to_vec())]),
    ]);
    let runner = FakeRunner::new(writes, 0);

    let result = run_processors(
        &runner,
        Path::new("/usr/bin/java"),
        &h.profile,
        &data,
        &h.root,
        &h.log_dir(),
        &null_sink(),
    )
    .await;
    assert!(
        result.is_ok(),
        "a jar whose entries match must be accepted, got {:?}",
        result.err()
    );
    assert!(h.out_a().exists(), "the jar is on disk");
}

/// A harness whose first output is a recompressed jar: right entries, wrong sha1.
fn recompressed_harness() -> (Harness, Vec<u8>) {
    let forge_built = deflated_jar(6);
    let we_built = deflated_jar(9);
    assert_ne!(forge_built, we_built, "levels must produce different bytes");
    let mut h = Harness::new();
    let mut data = h.data.0.clone();
    data.insert("OUT_A_SHA".to_string(), sha1_hex(&forge_built));
    h.data = DataMap(data);
    (h, we_built)
}

/// Writes for a run whose first processor produces `bytes` instead of `ALPHA`.
fn writes_with_a(h: &Harness, bytes: Vec<u8>) -> BTreeMap<String, Vec<(PathBuf, Vec<u8>)>> {
    BTreeMap::from([
        ("net.test.One".to_string(), vec![(h.out_a(), bytes)]),
        ("net.test.Two".to_string(), vec![(h.out_b(), BETA.to_vec())]),
    ])
}

#[tokio::test]
async fn an_accepted_output_writes_a_sidecar_that_skips_the_second_run() {
    let (h, we_built) = recompressed_harness();

    let first = FakeRunner::new(writes_with_a(&h, we_built.clone()), 0);
    h.run(&first, &null_sink()).await.expect("first run");
    assert_eq!(first.calls().len(), 2);

    let sidecar = h.root.libraries_dir().join("out/a.jar.sha1");
    let noted = std::fs::read_to_string(&sidecar).expect("sidecar");
    assert_eq!(noted.trim(), sha1_hex(&we_built));

    let second = FakeRunner::new(writes_with_a(&h, we_built), 0);
    h.run(&second, &null_sink()).await.expect("second run");
    assert!(
        second.calls().is_empty(),
        "the sidecar hash must skip both processors: {:?}",
        second.calls()
    );
}

#[tokio::test]
async fn a_damaged_archive_with_a_wrong_hash_still_fails() {
    let (h, we_built) = recompressed_harness();
    let mut damaged = we_built;
    damaged.truncate(damaged.len() - 40);

    let runner = FakeRunner::new(writes_with_a(&h, damaged), 0);
    let err = h
        .run(&runner, &null_sink())
        .await
        .expect_err("a damaged jar must fail the install");

    let text = err.to_string();
    assert!(
        matches!(err, Error::ProcessorOutputDamaged { ref path, .. } if path == &h.out_a()),
        "{err:?}"
    );
    assert!(
        text.contains("hash mismatch") && text.contains("damaged"),
        "{text}"
    );
    assert!(
        !h.root.libraries_dir().join("out/a.jar.sha1").exists(),
        "a damaged output gets no sidecar"
    );
}

#[tokio::test]
async fn a_missing_output_says_missing() {
    let h = Harness::new();
    let runner = FakeRunner::new(BTreeMap::new(), 0);

    let err = h.run(&runner, &null_sink()).await.expect_err("no output");

    assert!(err.to_string().contains("missing"), "{err}");
}
