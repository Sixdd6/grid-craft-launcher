//! Launcher orchestration: loader install and a dry-run launch, against mock metadata hosts.

use std::collections::BTreeMap;
use std::sync::Arc;

use gcl_core::content::AddRequest;
use gcl_core::download::hash::sha1_hex;
use gcl_core::instances::model::Loader;
use gcl_core::launcher::{Endpoints, LaunchOutcome};
use gcl_core::loaders::LoaderEndpoints;
use gcl_core::sources::SourceId;
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
        ..Endpoints::default()
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
        ..Endpoints::default()
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

/// Modrinth fixtures the content tests answer from.
const MODRINTH_PROJECT: &str = include_str!("../../../tests/fixtures/modrinth/project_sodium.json");
const MODRINTH_VERSIONS: &str =
    include_str!("../../../tests/fixtures/modrinth/versions_sodium_1.20.1_fabric.json");

/// Project id the Sodium fixture carries.
const SODIUM_ID: &str = "AANobbMI";

/// Bytes the mock Modrinth serves as the mod jar.
const SODIUM_JAR: &[u8] = b"synthetic sodium jar";

/// A launcher over a fresh root with Modrinth pointed at `uri` and every other host dead.
fn modrinth_launcher(dir: &tempfile::TempDir, uri: String) -> Launcher {
    let endpoints = Endpoints {
        mojang: "http://mojang.invalid".to_string(),
        modrinth: uri,
        curseforge: "http://curseforge.invalid".to_string(),
        loaders: LoaderEndpoints {
            fabric: "http://fabric.invalid".to_string(),
            ..LoaderEndpoints::default()
        },
    };
    let (launcher, _rx) =
        Launcher::open_with_endpoints(dir.path().to_path_buf(), endpoints).expect("build launcher");
    launcher
}

/// Serves the Sodium project, one version rewritten to this server, and the jar itself.
///
/// The fixture's file points at Modrinth's CDN and hashes the real jar, so both the URL and
/// the sha1 are rewritten here: the launcher verifies what it downloads.
async fn mock_modrinth(server: &MockServer) {
    let base = server.uri();
    let versions: Vec<serde_json::Value> =
        serde_json::from_str(MODRINTH_VERSIONS).expect("versions fixture");
    let mut version = versions.into_iter().next().expect("one version");
    version["dependencies"] = serde_json::json!([]);
    version["files"] = serde_json::json!([{
        "url": format!("{base}/files/sodium.jar"),
        "filename": "sodium.jar",
        "size": SODIUM_JAR.len(),
        "primary": true,
        "hashes": { "sha1": sha1_hex(SODIUM_JAR) },
    }]);

    serve(
        server,
        "/project/sodium",
        MODRINTH_PROJECT.as_bytes().to_vec(),
    )
    .await;
    serve(
        server,
        &format!("/project/{SODIUM_ID}/version"),
        serde_json::json!([version]).to_string().into_bytes(),
    )
    .await;
    serve(server, "/files/sodium.jar", SODIUM_JAR.to_vec()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn add_content_installs_a_modrinth_mod_and_the_list_reflects_it() {
    let server = MockServer::start().await;
    mock_modrinth(&server).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();

    tokio::task::spawn_blocking(move || {
        let launcher = modrinth_launcher(&dir, uri);
        let instance = launcher
            .instances()
            .create("Pack", MC, Loader::Fabric, None, &BTreeMap::new())
            .expect("create instance");

        let outcome = launcher
            .add_content(
                &instance.slug,
                AddRequest {
                    source: SourceId::Modrinth,
                    project: "sodium".to_string(),
                    version: None,
                    kind: None,
                    world: None,
                },
            )
            .expect("add content");
        assert_eq!(outcome.installed.len(), 1, "{outcome:?}");
        assert!(outcome.manual.is_empty());
        assert_eq!(outcome.installed[0].project_id, SODIUM_ID);

        let jar = instance.game_dir().join("mods").join("sodium.jar");
        assert_eq!(std::fs::read(&jar).expect("placed jar"), SODIUM_JAR);

        let listed = launcher.list_content(&instance.slug).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].file_name, "sodium.jar");

        let disabled = launcher
            .set_content_enabled(&instance.slug, SODIUM_ID, false)
            .expect("disable");
        assert!(disabled.ends_with("sodium.jar.disabled"), "{disabled:?}");
        assert!(!jar.exists());

        launcher
            .remove_content(&instance.slug, SODIUM_ID)
            .expect("remove");
        assert!(
            launcher
                .list_content(&instance.slug)
                .expect("list")
                .is_empty(),
            "the entry is gone from instance.toml"
        );
        assert!(!disabled.exists(), "the file is gone from disk");
        assert!(
            launcher
                .pending_manual(&instance.slug)
                .expect("pending")
                .is_empty(),
            "nothing needed a hand download"
        );
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn sources_hold_curseforge_only_when_a_key_is_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let with_key = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        // The key is read from the environment first, and `just` loads a `.env`, so a real
        // key on this machine must not decide the test.
        // SAFETY: nextest runs every test in its own process, so nothing else reads the env.
        unsafe {
            std::env::remove_var("CURSEFORGE_API_KEY");
        }
        let launcher = modrinth_launcher(&dir, "http://modrinth.invalid".to_string());
        let ids: Vec<SourceId> = launcher.sources().iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec![SourceId::Modrinth]);
        assert!(launcher.source(SourceId::CurseForge).is_err());

        // The key is read once, when the launcher opens, so it goes into `config.toml` first.
        let config = gcl_core::config::Config {
            keys: gcl_core::config::Keys {
                curseforge_api_key: Some("test-key".to_string()),
                msa_client_id: None,
            },
            ..gcl_core::config::Config::default()
        };
        config
            .save(&with_key.path().join("config.toml"))
            .expect("save config");
        let keyed = modrinth_launcher(&with_key, "http://modrinth.invalid".to_string());
        let ids: Vec<SourceId> = keyed.sources().iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec![SourceId::Modrinth, SourceId::CurseForge]);
        assert!(keyed.source(SourceId::CurseForge).is_ok());
        (dir, with_key)
    })
    .await
    .expect("blocking task");
}
