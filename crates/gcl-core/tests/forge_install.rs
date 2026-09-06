//! Headless Forge install against a mock maven, with a fake processor runner.

use gcl_core::download::DownloadCtx;
use gcl_core::events::null_sink;
use gcl_core::http::HttpClient;
use gcl_core::instances::model::Loader;
use gcl_core::loaders::{LoaderCtx, LoaderEndpoints};
use gcl_core::mojang::{Mojang, VersionJson};
use gcl_core::paths::Root;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{
    FORGE_ID as ID, FORGE_MC as MC, FORGE_VERSION as FORGE, FakeRunner, PATCHED_BYTES,
    PATCHED_PATH, PROC_PATH, UNIVERSAL_BYTES, UNIVERSAL_PATH, VLIB_PATH, fake_java as java,
    mock_forge, mock_vanilla,
};

#[tokio::test]
async fn forge_install_runs_processors_and_caches_the_result() {
    let server = MockServer::start().await;
    let base = server.uri();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    root.ensure_layout().expect("layout");

    mock_vanilla(&server, MC).await;
    mock_forge(&server).await;

    let http = HttpClient::new().expect("client");
    let sink = null_sink();
    let cancel = CancellationToken::new();
    let dl = DownloadCtx {
        http: &http,
        root: &root,
        sink: &sink,
        cancel: &cancel,
        parallel: 2,
    };
    let mojang = Mojang::with_base_url(http.clone(), root.clone(), base.clone());
    let java = java();
    let runner = FakeRunner::default();
    let ctx = LoaderCtx {
        http: &http,
        root: &root,
        dl: &dl,
        java: Some(&java),
        runner: Some(&runner),
        mojang: Some(&mojang),
    };
    let ep = LoaderEndpoints {
        forge_maven: base.clone(),
        forge_meta: base.clone(),
        ..LoaderEndpoints::default()
    };

    let id = gcl_core::loaders::install(&ctx, &ep, Loader::Forge, MC, FORGE)
        .await
        .expect("forge installs");
    assert_eq!(id, ID);

    // The version JSON lands in the cache with the launcher's id and vanilla as its parent.
    let file = root.versions_dir().join(format!("{ID}.json"));
    let written: VersionJson =
        serde_json::from_str(&std::fs::read_to_string(&file).expect("read")).expect("parse");
    assert_eq!(written.id, ID);
    assert_eq!(written.inherits_from.as_deref(), Some(MC));
    assert_eq!(
        written.main_class.as_deref(),
        Some("cpw.mods.bootstraplauncher.BootstrapLauncher")
    );

    // `maven/` is extracted, urlless libraries are skipped, and the rest are downloaded.
    let libs = root.libraries_dir();
    assert_eq!(
        std::fs::read(libs.join(UNIVERSAL_PATH)).expect("universal extracted"),
        UNIVERSAL_BYTES
    );
    assert!(libs.join(PROC_PATH).is_file(), "processor jar downloaded");
    assert!(libs.join(VLIB_PATH).is_file(), "version library downloaded");

    // One client-side processor ran, with the data map substituted into its arguments.
    let calls = runner.calls.lock().expect("lock").clone();
    assert_eq!(calls.len(), 1, "the server-side processor must be skipped");
    let args = &calls[0];
    let client_jar = root.versions_dir().join(MC).join(format!("{MC}.jar"));
    assert_eq!(args[1], client_jar.display().to_string());
    assert!(client_jar.is_file(), "vanilla client jar was installed");
    assert!(args[3].ends_with("client.lzma"), "{args:?}");
    assert_eq!(args[5], libs.join(PATCHED_PATH).display().to_string());
    assert_eq!(
        std::fs::read(libs.join(PATCHED_PATH)).expect("patched jar"),
        PATCHED_BYTES
    );
    assert!(
        root.logs_dir()
            .join("installers")
            .join(ID)
            .join("0-proc.log")
            .is_file(),
        "processor log written under logs/installers/<id>"
    );

    // A second install is served from the cache: no installer request, no processor run.
    let again = gcl_core::loaders::install(&ctx, &ep, Loader::Forge, MC, FORGE)
        .await
        .expect("second install");
    assert_eq!(again, ID);
    assert_eq!(runner.calls.lock().expect("lock").len(), 1);
    server.verify().await;
}

#[tokio::test]
async fn forge_install_of_a_build_the_maven_does_not_have_is_no_such_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    root.ensure_layout().expect("layout");

    let http = HttpClient::new()
        .expect("client")
        .with_backoff(vec![std::time::Duration::ZERO]);
    let sink = null_sink();
    let cancel = CancellationToken::new();
    let dl = DownloadCtx {
        http: &http,
        root: &root,
        sink: &sink,
        cancel: &cancel,
        parallel: 2,
    };
    let java = java();
    let runner = FakeRunner::default();
    let ctx = LoaderCtx {
        http: &http,
        root: &root,
        dl: &dl,
        java: Some(&java),
        runner: Some(&runner),
        mojang: None,
    };
    let ep = LoaderEndpoints {
        neoforge: server.uri(),
        ..LoaderEndpoints::default()
    };

    let err = gcl_core::loaders::install(&ctx, &ep, Loader::NeoForge, "1.21.1", "9.9.9")
        .await
        .expect_err("no such build");
    match err {
        gcl_core::loaders::Error::NoSuchVersion {
            loader,
            mc,
            version,
        } => {
            assert_eq!(loader, Loader::NeoForge);
            assert_eq!(mc, "1.21.1");
            assert_eq!(version, "9.9.9");
        }
        other => panic!("expected NoSuchVersion, got {other:?}"),
    }
    assert!(runner.calls.lock().expect("lock").is_empty());
}
