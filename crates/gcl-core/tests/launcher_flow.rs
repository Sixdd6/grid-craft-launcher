//! Launcher orchestration: loader install and a dry-run launch, against mock metadata hosts.

use std::collections::BTreeMap;

use gcl_core::instances::model::Loader;
use gcl_core::launcher::LaunchOutcome;
use gcl_core::loaders::LoaderEndpoints;
use gcl_core::{Launcher, auth};
use wiremock::MockServer;

mod common;
use common::{mock_vanilla, serve};

const MC: &str = "1.20.1";
const FABRIC: &str = "0.19.5";
const FABRIC_LOADERS: &str = include_str!("../../../tests/fixtures/fabric/loader_1.20.1.json");
const FABRIC_PROFILE: &str = include_str!("../../../tests/fixtures/fabric/profile_1.20.1.json");

/// A launcher over a fresh root, with Fabric and Mojang pointed at `endpoints`.
fn launcher(dir: &tempfile::TempDir, mojang: Option<String>, fabric: Option<String>) -> Launcher {
    let endpoints = LoaderEndpoints {
        fabric: fabric.unwrap_or_else(|| LoaderEndpoints::default().fabric),
        ..LoaderEndpoints::default()
    };
    let (launcher, _rx) =
        Launcher::open_with_endpoints(dir.path().to_path_buf(), mojang, endpoints)
            .expect("build launcher");
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
