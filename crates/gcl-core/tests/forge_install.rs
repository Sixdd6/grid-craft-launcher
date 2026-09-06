//! Headless Forge install against a mock maven, with a fake processor runner.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gcl_core::download::DownloadCtx;
use gcl_core::download::hash::sha1_hex;
use gcl_core::events::null_sink;
use gcl_core::http::HttpClient;
use gcl_core::instances::model::Loader;
use gcl_core::java::{JavaInstall, JavaSource};
use gcl_core::loaders::{LoaderCtx, LoaderEndpoints, ProcessRunner};
use gcl_core::mojang::{Mojang, VersionJson};
use gcl_core::paths::Root;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod common;
use common::{jar_with_main, mock_vanilla, serve, zip_bytes};

const MC: &str = "1.20.1";
const FORGE: &str = "47.4.10";
const ID: &str = "1.20.1-forge-47.4.10";
const PROC_MAIN: &str = "com.example.Proc";
const PROC_PATH: &str = "com/example/proc/1.0/proc-1.0.jar";
const VLIB_PATH: &str = "com/example/vlib/2.0/vlib-2.0.jar";
const UNIVERSAL_PATH: &str =
    "net/minecraftforge/forge/1.20.1-47.4.10/forge-1.20.1-47.4.10-universal.jar";
const PATCHED_PATH: &str =
    "net/minecraftforge/forge/1.20.1-47.4.10/forge-1.20.1-47.4.10-client.jar";
const PATCHED_BYTES: &[u8] = b"patched client jar";
const UNIVERSAL_BYTES: &[u8] = b"universal jar";
const VLIB_BYTES: &[u8] = b"vlib jar";

/// A runner that records its calls and writes the file the processor promises.
struct FakeRunner {
    calls: Mutex<Vec<Vec<String>>>,
}

#[async_trait::async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(
        &self,
        _java: &Path,
        _classpath: &[PathBuf],
        main: &str,
        args: &[String],
        log: &Path,
    ) -> Result<i32, std::io::Error> {
        assert_eq!(main, PROC_MAIN);
        self.calls.lock().expect("lock").push(args.to_vec());
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(log, format!("ran {main}\n"))?;
        // The real processor writes the patched jar the profile names in `--output`.
        let out = args
            .iter()
            .position(|a| a == "--output")
            .and_then(|i| args.get(i + 1))
            .expect("processor was given an output");
        let out = PathBuf::from(out);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, PATCHED_BYTES)?;
        Ok(0)
    }
}

/// Builds an installer jar with one client processor, a `maven/` entry, and a `data/` file.
fn installer_jar(base: &str) -> Vec<u8> {
    let lib = |name: &str, path: &str, body: &[u8]| {
        serde_json::json!({
            "name": name,
            "downloads": { "artifact": {
                "path": path,
                "url": format!("{base}/libs/{path}"),
                "sha1": sha1_hex(body),
                "size": body.len(),
            }}
        })
    };
    // The universal jar comes out of `maven/` inside the installer, so its artifact has no URL.
    let universal = serde_json::json!({
        "name": "net.minecraftforge:forge:1.20.1-47.4.10:universal",
        "downloads": { "artifact": {
            "path": UNIVERSAL_PATH,
            "url": "",
            "sha1": sha1_hex(UNIVERSAL_BYTES),
            "size": UNIVERSAL_BYTES.len(),
        }}
    });
    let profile = serde_json::json!({
        "spec": 1,
        "profile": "forge",
        "version": ID,
        "minecraft": MC,
        "json": "/version.json",
        "data": {
            "BINPATCH": { "client": "/data/client.lzma", "server": "/data/server.lzma" },
            "PATCHED": {
                "client": "[net.minecraftforge:forge:1.20.1-47.4.10:client]",
                "server": "[net.minecraftforge:forge:1.20.1-47.4.10:server]",
            },
            "PATCHED_SHA": {
                "client": format!("'{}'", sha1_hex(PATCHED_BYTES)),
                "server": "'0000000000000000000000000000000000000000'",
            },
        },
        "processors": [
            {
                "jar": "com.example:proc:1.0",
                "classpath": [],
                "args": [
                    "--input", "{MINECRAFT_JAR}",
                    "--patch", "{BINPATCH}",
                    "--output", "{PATCHED}",
                ],
                "outputs": { "{PATCHED}": "{PATCHED_SHA}" },
            },
            {
                "sides": ["server"],
                "jar": "com.example:proc:1.0",
                "args": ["--server"],
            },
        ],
        "libraries": [lib("com.example:proc:1.0", PROC_PATH, &jar_with_main(PROC_MAIN))],
    });
    let version = serde_json::json!({
        "id": "placeholder",
        "inheritsFrom": MC,
        "type": "release",
        "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
        "libraries": [lib("com.example:vlib:2.0", VLIB_PATH, VLIB_BYTES), universal],
    });
    zip_bytes(&[
        ("install_profile.json", profile.to_string().into_bytes()),
        ("version.json", version.to_string().into_bytes()),
        ("data/client.lzma", b"binary patch".to_vec()),
        (&format!("maven/{UNIVERSAL_PATH}"), UNIVERSAL_BYTES.to_vec()),
    ])
}

/// The java the installer would run. Never executed: the fake runner stands in for it.
fn java() -> JavaInstall {
    JavaInstall {
        path: PathBuf::from("/nonexistent/java"),
        major: 17,
        version: "17.0.0".to_string(),
        vendor: "test".to_string(),
        source: JavaSource::Manual,
    }
}

#[tokio::test]
async fn forge_install_runs_processors_and_caches_the_result() {
    let server = MockServer::start().await;
    let base = server.uri();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    root.ensure_layout().expect("layout");

    mock_vanilla(&server, MC).await;
    let jar = installer_jar(&base);
    Mock::given(method("GET"))
        .and(path_matcher(format!(
            "/net/minecraftforge/forge/{MC}-{FORGE}/forge-{MC}-{FORGE}-installer.jar"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(jar))
        .expect(1)
        .mount(&server)
        .await;
    serve(
        &server,
        &format!("/libs/{PROC_PATH}"),
        jar_with_main(PROC_MAIN),
    )
    .await;
    serve(&server, &format!("/libs/{VLIB_PATH}"), VLIB_BYTES.to_vec()).await;

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
    let runner = FakeRunner {
        calls: Mutex::new(Vec::new()),
    };
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
    let runner = FakeRunner {
        calls: Mutex::new(Vec::new()),
    };
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
