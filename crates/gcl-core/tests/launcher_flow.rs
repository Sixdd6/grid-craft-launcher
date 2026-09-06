//! Launcher orchestration: loader install and a dry-run launch, against mock metadata hosts.

use std::collections::BTreeMap;
use std::sync::Arc;

use gcl_core::instances::model::Loader;
use gcl_core::launcher::{Endpoints, LaunchOutcome};
use gcl_core::loaders::LoaderEndpoints;
use gcl_core::{Launcher, auth};
use wiremock::MockServer;

mod common;
use common::{
    FORGE_ID, FORGE_MC, FORGE_VERSION, FakeRunner, PATCHED_PATH, UNIVERSAL_PATH, mock_forge,
    mock_vanilla, serve,
};

const MC: &str = "1.20.1";
const FABRIC: &str = "0.19.5";
const FABRIC_LOADERS: &str = include_str!("../../../tests/fixtures/fabric/loader_1.20.1.json");
const FABRIC_PROFILE: &str = include_str!("../../../tests/fixtures/fabric/profile_1.20.1.json");

/// A launcher over a fresh root, with the Mojang and Fabric hosts pointed at the mock server.
///
/// Anything left `None` keeps a host that no test reaches, so an accidental request fails
/// rather than leaving the machine.
fn launcher(dir: &tempfile::TempDir, mojang: Option<String>, fabric: Option<String>) -> Launcher {
    let endpoints = Endpoints {
        mojang: mojang.unwrap_or_else(|| "http://mojang.invalid".to_string()),
        loaders: LoaderEndpoints {
            fabric: fabric.unwrap_or_else(|| "http://fabric.invalid".to_string()),
            ..LoaderEndpoints::default()
        },
    };
    let (launcher, _rx) =
        Launcher::open_with_endpoints(dir.path().to_path_buf(), endpoints).expect("build launcher");
    launcher
}

#[tokio::test(flavor = "multi_thread")]
async fn install_loader_writes_the_recommended_version_into_the_instance() {
    let server = MockServer::start().await;
    serve(
        &server,
        &format!("/v2/versions/loader/{MC}"),
        FABRIC_LOADERS.as_bytes().to_vec(),
    )
    .await;
    serve(
        &server,
        &format!("/v2/versions/loader/{MC}/{FABRIC}/profile/json"),
        FABRIC_PROFILE.as_bytes().to_vec(),
    )
    .await;

    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();
    let id = tokio::task::spawn_blocking(move || {
        let launcher = launcher(&dir, None, Some(uri));
        let instance = launcher
            .instances()
            .create("Pack", MC, Loader::Fabric, None, &BTreeMap::new())
            .expect("create instance");
        let id = launcher.install_loader(&instance.slug).expect("install");
        let saved = launcher.instances().get(&instance.slug).expect("reload");
        assert_eq!(saved.config.loader_version.as_deref(), Some(FABRIC));
        assert!(
            launcher
                .root()
                .versions_dir()
                .join(format!("{id}.json"))
                .is_file(),
            "the loader profile is cached"
        );
        (id, dir)
    })
    .await
    .expect("blocking task");
    assert_eq!(id.0, format!("fabric-loader-{FABRIC}-{MC}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_launch_builds_a_command_for_an_offline_user() {
    let server = MockServer::start().await;
    mock_vanilla(&server, MC).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();

    tokio::task::spawn_blocking(move || {
        let launcher = launcher(&dir, Some(uri), None);
        let mut instance = launcher
            .instances()
            .create("Pack", MC, Loader::None, None, &BTreeMap::new())
            .expect("create instance");
        instance
            .config
            .settings_overrides
            .insert("renderDistance".to_string(), "12".to_string());
        // A fixed java path keeps the test off the Mojang runtime endpoints.
        instance.config.jvm.java_path = Some(std::path::PathBuf::from("/usr/bin/java"));
        instance.save().expect("save instance");

        let outcome = launcher
            .launch_instance(&instance.slug, None, Some("tester"), true)
            .expect("dry run");
        let LaunchOutcome::DryRun(cmd) = outcome else {
            panic!("expected a dry run");
        };
        let user = cmd
            .args
            .iter()
            .position(|a| a == "--username")
            .expect("a --username argument");
        assert_eq!(cmd.args[user + 1], "tester");
        assert_eq!(cmd.cwd, instance.game_dir());
        assert_eq!(cmd.program, std::path::PathBuf::from("/usr/bin/java"));

        let options =
            std::fs::read_to_string(instance.game_dir().join("options.txt")).expect("options.txt");
        assert!(options.contains("renderDistance:12"), "{options}");
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launch_without_any_account_is_no_account() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let launcher = launcher(&dir, None, None);
        let instance = launcher
            .instances()
            .create("Pack", MC, Loader::None, None, &BTreeMap::new())
            .expect("create instance");
        let err = launcher
            .launch_instance(&instance.slug, None, None, true)
            .expect_err("no account");
        assert!(
            matches!(err, gcl_core::Error::Auth(auth::Error::NoAccount)),
            "{err:?}"
        );
        dir
    })
    .await
    .expect("blocking task");
}

/// A launcher over a fresh root with Mojang and both Forge hosts pointed at `uri`, running
/// installer processors through `runner` instead of a JVM.
fn forge_launcher(dir: &tempfile::TempDir, uri: String, runner: Arc<FakeRunner>) -> Launcher {
    let endpoints = Endpoints {
        mojang: uri.clone(),
        loaders: LoaderEndpoints {
            forge_maven: uri.clone(),
            forge_meta: uri,
            ..LoaderEndpoints::default()
        },
    };
    let (launcher, _rx) =
        Launcher::open_with_endpoints(dir.path().to_path_buf(), endpoints).expect("build launcher");
    launcher.with_process_runner(runner)
}

#[tokio::test(flavor = "multi_thread")]
async fn install_instance_runs_a_forge_install_through_the_injected_process_runner() {
    let server = MockServer::start().await;
    mock_vanilla(&server, FORGE_MC).await;
    mock_forge(&server).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();
    let runner = Arc::new(FakeRunner::default());
    let calls = Arc::clone(&runner);

    let dir = tokio::task::spawn_blocking(move || {
        let launcher = forge_launcher(&dir, uri, runner);
        let mut instance = launcher
            .instances()
            .create(
                "Forge Pack",
                FORGE_MC,
                Loader::Forge,
                None,
                &BTreeMap::new(),
            )
            .expect("create instance");
        // A pinned build keeps the test off the Forge promotions endpoint, and a fixed java
        // path keeps it off the Mojang runtime endpoints. Neither java nor the processor is
        // ever started: `FakeRunner` stands in for both.
        instance.config.loader_version = Some(FORGE_VERSION.to_string());
        instance.config.jvm.java_path = Some(std::path::PathBuf::from("/nonexistent/java"));
        instance.save().expect("save instance");

        let plan = launcher
            .install_instance(&instance.slug)
            .expect("forge instance installs");

        assert_eq!(plan.resolved.id, FORGE_ID);
        let libs = launcher.root().libraries_dir();
        assert!(
            plan.classpath.contains(&libs.join(UNIVERSAL_PATH)),
            "the jar extracted from the installer's maven/ is on the classpath: {:?}",
            plan.classpath
        );
        assert_eq!(
            std::fs::read(libs.join(UNIVERSAL_PATH)).expect("universal jar"),
            common::UNIVERSAL_BYTES
        );
        assert_eq!(
            std::fs::read(libs.join(PATCHED_PATH)).expect("patched jar"),
            common::PATCHED_BYTES,
            "the fake runner wrote the processor output"
        );
        assert_eq!(
            plan.log_config,
            Some(
                launcher
                    .root()
                    .assets_dir()
                    .join("log_configs")
                    .join(common::LOG_CONFIG_ID)
            ),
            "the inherited vanilla logging block is installed"
        );
        assert_eq!(
            calls.calls.lock().expect("lock").len(),
            1,
            "one client-side processor ran"
        );
        dir
    })
    .await
    .expect("blocking task");
    server.verify().await;
    drop(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn launching_with_an_account_does_not_change_the_active_account() {
    let server = MockServer::start().await;
    mock_vanilla(&server, MC).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();

    tokio::task::spawn_blocking(move || {
        let launcher = launcher(&dir, Some(uri), None);
        let accounts = launcher.accounts();
        accounts
            .add(gcl_core::auth::offline::offline_account("active-one"))
            .expect("first account");
        let other = accounts
            .add(gcl_core::auth::offline::offline_account("other-one"))
            .expect("second account");
        let before = accounts.load().expect("load").active;
        assert_ne!(before.as_deref(), Some(other.id.as_str()));

        let mut instance = launcher
            .instances()
            .create("Pack", MC, Loader::None, None, &BTreeMap::new())
            .expect("create instance");
        instance.config.jvm.java_path = Some(std::path::PathBuf::from("/usr/bin/java"));
        instance.save().expect("save instance");

        let outcome = launcher
            .launch_instance(&instance.slug, Some("other-one"), None, true)
            .expect("dry run");
        let LaunchOutcome::DryRun(cmd) = outcome else {
            panic!("expected a dry run");
        };
        let user = cmd
            .args
            .iter()
            .position(|a| a == "--username")
            .expect("a --username argument");
        assert_eq!(cmd.args[user + 1], "other-one");

        let file = std::fs::read_to_string(launcher.root().accounts_file()).expect("accounts.json");
        let after: serde_json::Value = serde_json::from_str(&file).expect("parse");
        assert_eq!(
            after["active"].as_str(),
            before.as_deref(),
            "a --account launch leaves the active account alone"
        );
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn launching_with_an_unknown_account_is_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let launcher = launcher(&dir, None, None);
        let instance = launcher
            .instances()
            .create("Pack", MC, Loader::None, None, &BTreeMap::new())
            .expect("create instance");
        let err = launcher
            .launch_instance(&instance.slug, Some("ghost"), None, true)
            .expect_err("no such account");
        assert!(
            matches!(err, gcl_core::Error::Auth(auth::Error::NotFound(ref name)) if name == "ghost"),
            "{err:?}"
        );
        dir
    })
    .await
    .expect("blocking task");
}
