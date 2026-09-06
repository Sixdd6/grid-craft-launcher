//! Importing a synthetic `.mrpack` end to end: loader install, file downloads,
//! overrides, and the cleanup that runs when a file cannot be fetched.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gcl_core::content::ContentCtx;
use gcl_core::download::DownloadCtx;
use gcl_core::download::hash::sha1_hex;
use gcl_core::events::{Event, EventSink};
use gcl_core::http::HttpClient;
use gcl_core::instances::Instances;
use gcl_core::instances::model::{ContentKind, Loader, PackSource};
use gcl_core::loaders::{LoaderCtx, LoaderEndpoints};
use gcl_core::modpacks::{self, ImportRequest};
use gcl_core::paths::Root;
use tokio_util::sync::CancellationToken;
use wiremock::MockServer;

mod common;
use common::{mock_vanilla, serve, zip_bytes};

/// Minecraft version the Fabric fixtures answer for.
const MC: &str = "1.20.1";

/// Fabric loader version the fixture marks as the one to install.
const FABRIC: &str = "0.19.5";

const FABRIC_LOADERS: &str = include_str!("../../../tests/fixtures/fabric/loader_1.20.1.json");
const FABRIC_PROFILE: &str = include_str!("../../../tests/fixtures/fabric/profile_1.20.1.json");

/// Bytes of the mod the pack installs into `mods/`.
const MOD_JAR: &[u8] = b"synthetic mod jar";

/// Bytes of the pack file that lands outside a folder the content list manages.
const CONFIG_TXT: &[u8] = b"key = value";

/// Body of the one override file, which must end up in the game directory.
const OVERRIDE_TOML: &str = "greeting = \"hello\"\n";

/// Everything the contexts borrow, kept alive for the length of a test.
struct Harness {
    http: HttpClient,
    root: Root,
    sink: EventSink,
    rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    cancel: CancellationToken,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        let (sink, rx) = tokio::sync::mpsc::unbounded_channel();
        Harness {
            http: HttpClient::new().expect("http"),
            root,
            sink,
            rx,
            cancel: CancellationToken::new(),
            _dir: dir,
        }
    }

    fn dl(&self) -> DownloadCtx<'_> {
        DownloadCtx {
            http: &self.http,
            root: &self.root,
            sink: &self.sink,
            cancel: &self.cancel,
            parallel: 4,
        }
    }

    fn logs(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            if let Event::Log { message, .. } = event {
                out.push(message);
            }
        }
        out
    }
}

/// Serves the Fabric metadata, the synthetic vanilla version, and the pack's two files.
///
/// `mod_status` is the status code the `mods/` file answers with, so one test can make
/// it 404 and watch the import roll back.
async fn mock_pack_hosts(server: &MockServer, mod_status: u16) {
    mock_vanilla(server, MC).await;
    serve(
        server,
        &format!("/v2/versions/loader/{MC}"),
        FABRIC_LOADERS.as_bytes().to_vec(),
    )
    .await;
    serve(
        server,
        &format!("/v2/versions/loader/{MC}/{FABRIC}/profile/json"),
        FABRIC_PROFILE.as_bytes().to_vec(),
    )
    .await;
    if mod_status == 200 {
        serve(server, "/files/alpha.jar", MOD_JAR.to_vec()).await;
    } else {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/files/alpha.jar"))
            .respond_with(wiremock::ResponseTemplate::new(mod_status))
            .mount(server)
            .await;
    }
    serve(server, "/files/notes.txt", CONFIG_TXT.to_vec()).await;
}

/// Builds a `.mrpack` on disk: two downloaded files and one override.
fn write_mrpack(dir: &std::path::Path, base: &str) -> PathBuf {
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "name": "Test Pack",
        "versionId": "1.2.3",
        "dependencies": { "minecraft": MC, "fabric-loader": FABRIC },
        "files": [
            {
                "path": "mods/alpha.jar",
                "hashes": { "sha1": sha1_hex(MOD_JAR) },
                "env": { "client": "required", "server": "required" },
                "downloads": [format!("{base}/files/alpha.jar")],
                "fileSize": MOD_JAR.len(),
            },
            {
                "path": "config/notes.txt",
                "hashes": { "sha1": sha1_hex(CONFIG_TXT) },
                "env": { "client": "required", "server": "required" },
                "downloads": [format!("{base}/files/notes.txt")],
                "fileSize": CONFIG_TXT.len(),
            },
            {
                "path": "mods/server-only.jar",
                "hashes": { "sha1": sha1_hex(b"never fetched") },
                "env": { "client": "unsupported", "server": "required" },
                "downloads": [format!("{base}/files/missing.jar")],
                "fileSize": 13,
            }
        ]
    })
    .to_string();

    let bytes = zip_bytes(&[
        ("modrinth.index.json", index.into_bytes()),
        ("overrides/config/x.toml", OVERRIDE_TOML.as_bytes().to_vec()),
        ("overrides/", Vec::new()),
    ]);
    let path = dir.join("test.mrpack");
    std::fs::write(&path, bytes).expect("write mrpack");
    path
}

/// Lets a pack download from the mock server, which is not on the mrpack allowlist.
///
/// Every test sets the same value, so nextest's process-per-test isolation and a
/// same-process run both see a stable one.
fn allow_local_hosts() {
    // SAFETY: tests run one per process under nextest, and every caller sets the same
    // value, so no other thread can observe a half-written environment.
    unsafe { std::env::set_var(modpacks::EXTRA_HOSTS_ENV, "127.0.0.1,localhost") };
}

/// Runs `import` over the harness with the pack's own source recorded.
async fn import(
    harness: &Harness,
    fabric: &str,
    zip: PathBuf,
    keep_partial: bool,
) -> Result<modpacks::ImportOutcome, modpacks::Error> {
    let dl = harness.dl();
    let ctx = ContentCtx {
        sources: &[],
        dl: &dl,
        root: &harness.root,
        sink: &harness.sink,
    };
    let loader_ctx = LoaderCtx {
        http: &harness.http,
        root: &harness.root,
        dl: &dl,
        java: None,
        runner: None,
        mojang: None,
    };
    let ep = LoaderEndpoints {
        fabric: fabric.to_string(),
        ..LoaderEndpoints::default()
    };
    let instances = Instances::new(harness.root.clone());
    modpacks::import(
        &ctx,
        &instances,
        &loader_ctx,
        &ep,
        &BTreeMap::new(),
        ImportRequest {
            zip,
            name: None,
            keep_partial,
            pack_source: Some(PackSource {
                source: "modrinth".to_string(),
                project_id: "abc123".to_string(),
                version_id: "ver456".to_string(),
            }),
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn import_installs_the_loader_the_files_and_the_overrides() {
    allow_local_hosts();
    let server = MockServer::start().await;
    mock_pack_hosts(&server, 200).await;

    let mut harness = Harness::new();
    let zip = write_mrpack(harness.root.path(), &server.uri());

    let outcome = import(&harness, &server.uri(), zip, false)
        .await
        .expect("import");
    let instance = outcome.instance;

    assert_eq!(instance.slug, "test-pack");
    assert_eq!(instance.config.name, "Test Pack");
    assert_eq!(instance.config.minecraft, MC);
    assert_eq!(instance.config.loader, Loader::Fabric);
    assert_eq!(instance.config.loader_version.as_deref(), Some(FABRIC));
    assert_eq!(outcome.installed, 2, "both client files were placed");
    assert!(outcome.manual.is_empty());

    // The loader profile is in the shared version cache.
    let version_id = gcl_core::loaders::version_id(Loader::Fabric, MC, FABRIC);
    assert!(
        harness
            .root
            .versions_dir()
            .join(format!("{version_id}.json"))
            .is_file(),
        "the Fabric profile was cached"
    );

    // Both pack files landed at the path the index gave them.
    let game_dir = instance.game_dir();
    assert_eq!(
        std::fs::read(game_dir.join("mods/alpha.jar")).expect("mod jar"),
        MOD_JAR
    );
    assert_eq!(
        std::fs::read(game_dir.join("config/notes.txt")).expect("config file"),
        CONFIG_TXT
    );
    assert!(
        !game_dir.join("mods/server-only.jar").exists(),
        "the client-unsupported file was skipped"
    );

    // The override landed too.
    assert_eq!(
        std::fs::read_to_string(game_dir.join("config/x.toml")).expect("override"),
        OVERRIDE_TOML
    );

    // Only the file under `mods/` is recorded in the content list.
    let saved = Instances::new(harness.root.clone())
        .get(&instance.slug)
        .expect("reload");
    assert_eq!(saved.config.content.len(), 1);
    let entry = &saved.config.content[0];
    assert_eq!(entry.file_name, "alpha.jar");
    assert_eq!(entry.kind, ContentKind::Mod);
    assert_eq!(entry.source, "modrinth");
    assert_eq!(entry.sha1.as_deref(), Some(sha1_hex(MOD_JAR).as_str()));

    // The pack the instance came from is recorded.
    let pack = saved.config.pack.expect("pack source");
    assert_eq!(pack.source, "modrinth");
    assert_eq!(pack.project_id, "abc123");
    assert_eq!(pack.version_id, "ver456");

    let logs = harness.logs();
    assert!(
        logs.iter().any(|l| l.contains("files 1/2")),
        "a per-file log line: {logs:?}"
    );
    assert!(
        logs.iter().any(|l| l.contains("override")),
        "an overrides log line: {logs:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_download_removes_the_partial_instance() {
    allow_local_hosts();
    let server = MockServer::start().await;
    mock_pack_hosts(&server, 404).await;

    let harness = Harness::new();
    let zip = write_mrpack(harness.root.path(), &server.uri());

    let err = import(&harness, &server.uri(), zip, false)
        .await
        .expect_err("404");
    assert!(
        matches!(err, modpacks::Error::Content(_)),
        "the download failure surfaces: {err:?}"
    );
    assert!(
        !harness.root.instance_dir("test-pack").exists(),
        "the partial instance was deleted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn keep_partial_leaves_the_instance_behind() {
    allow_local_hosts();
    let server = MockServer::start().await;
    mock_pack_hosts(&server, 404).await;

    let harness = Harness::new();
    let zip = write_mrpack(harness.root.path(), &server.uri());

    import(&harness, &server.uri(), zip, true)
        .await
        .expect_err("404");
    let kept = harness.root.instance_dir("test-pack");
    assert!(kept.is_dir(), "the partial instance was kept");
    assert!(
        kept.join("instance.toml").is_file(),
        "and it still has its config"
    );
}
