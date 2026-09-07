//! Launcher orchestration: loader install and a dry-run launch, against mock metadata hosts.

use std::collections::BTreeMap;
use std::sync::Arc;

use gcl_core::auth::secrets::{MemoryStore, SecretStoreKind};
use gcl_core::auth::{Account, AccountKind};
use gcl_core::content::AddRequest;
use gcl_core::download::hash::sha1_hex;
use gcl_core::instances::model::{InstanceJvm, Loader};
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
        msa: common::dead_msa_endpoints(),
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
        msa: common::dead_msa_endpoints(),
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
        msa: common::dead_msa_endpoints(),
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

/// Microsoft client id the login tests configure. Any non-empty string will do: the mock
/// endpoints never check it.
const MSA_CLIENT_ID: &str = "test-client";

/// A Minecraft token expiry far enough ahead that no launch treats it as stale.
const NOT_EXPIRED: &str = "2999-01-01T00:00:00Z";

/// An expiry in the past, so a launch has to refresh before it builds a command line.
const EXPIRED: &str = "2020-01-01T00:00:00Z";

/// A launcher with Mojang and all six Microsoft hosts pointed at `uri`, a client id
/// configured, and refresh tokens kept in memory.
///
/// `client_id` of `None` leaves Microsoft login turned off, which is what a user without a
/// registered app has.
fn msa_launcher(dir: &tempfile::TempDir, uri: &str, client_id: Option<&str>) -> Launcher {
    // `GCL_MSA_CLIENT_ID` wins over the config file, and `just` loads a `.env`, so a real id
    // on this machine must not decide the test.
    // SAFETY: nextest runs every test in its own process, so nothing else reads the env.
    unsafe {
        std::env::remove_var("GCL_MSA_CLIENT_ID");
    }
    let endpoints = Endpoints {
        mojang: uri.to_string(),
        modrinth: "http://modrinth.invalid".to_string(),
        curseforge: "http://curseforge.invalid".to_string(),
        loaders: LoaderEndpoints {
            fabric: "http://fabric.invalid".to_string(),
            ..LoaderEndpoints::default()
        },
        msa: common::msa_endpoints(uri),
    };
    let (launcher, _rx) =
        Launcher::open_with_endpoints(dir.path().to_path_buf(), endpoints).expect("build launcher");
    launcher
        .update_config(|config| config.keys.msa_client_id = client_id.map(str::to_string))
        .expect("update config");
    launcher.with_secret_store(Box::new(MemoryStore::new()))
}

/// A signed-in Microsoft account as the accounts file stores one.
fn stored_msa_account(expires: &str) -> Account {
    Account {
        id: common::MSA_ACCOUNT_ID.to_string(),
        name: common::MSA_NAME.to_string(),
        kind: AccountKind::Msa,
        mc_token: Some(MC_TOKEN.to_string()),
        mc_token_expires: Some(expires.to_string()),
        xuid: Some(common::MSA_XUID.to_string()),
        refresh_store: Some(SecretStoreKind::Memory),
    }
}

/// The Minecraft token the mock services hand out, and the one a stored account carries.
const MC_TOKEN: &str = "mc-token-1";

/// An instance with a fixed java path, which keeps a launch off the Mojang runtime endpoints.
fn launchable_instance(launcher: &Launcher) -> gcl_core::instances::Instance {
    let mut instance = launcher
        .instances()
        .create("Pack", MC, Loader::None, None, &BTreeMap::new())
        .expect("create instance");
    instance.config.jvm.java_path = Some(std::path::PathBuf::from("/usr/bin/java"));
    instance.save().expect("save instance");
    instance
}

/// The value that follows `flag` in a command line.
fn arg_after(args: &[String], flag: &str) -> String {
    let at = args
        .iter()
        .position(|a| a == flag)
        .unwrap_or_else(|| panic!("a {flag} argument in {args:?}"));
    args[at + 1].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn msa_login_saves_the_account_and_its_refresh_token() {
    let server = MockServer::start().await;
    common::mount_device_code(&server).await;
    common::mount_token(&server, "refresh-1", 1).await;
    common::mount_msa_chain(&server, MC_TOKEN).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();
    let dir = tokio::task::spawn_blocking(move || {
        let launcher = msa_launcher(&dir, &uri, Some(MSA_CLIENT_ID));
        assert!(launcher.msa_available());

        let shown = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&shown);
        let account = launcher
            .msa_login(&move |code: &gcl_core::auth::msa::DeviceCode| {
                seen.lock().expect("lock").push(code.user_code.clone());
            })
            .expect("sign in");

        assert_eq!(shown.lock().expect("lock").as_slice(), ["ABCD-EFGH"]);
        assert_eq!(account.kind, AccountKind::Msa);
        assert_eq!(account.id, common::MSA_ACCOUNT_ID);
        assert_eq!(account.name, common::MSA_NAME);
        assert_eq!(account.xuid.as_deref(), Some(common::MSA_XUID));
        assert_eq!(account.mc_token.as_deref(), Some(MC_TOKEN));
        assert_eq!(
            launcher.accounts().active().expect("active"),
            Some(account.clone()),
            "the first account signed in becomes the active one"
        );
        assert_eq!(
            launcher.secrets().get(&account.id).expect("get"),
            Some("refresh-1".to_string()),
            "the refresh token went to the injected store"
        );
        dir
    })
    .await
    .expect("blocking task");
    server.verify().await;
    drop(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_launch_of_a_microsoft_account_fills_in_every_placeholder() {
    let server = MockServer::start().await;
    common::mock_vanilla_with_arguments(&server, MC, common::ACCOUNT_ARGUMENTS).await;

    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();
    tokio::task::spawn_blocking(move || {
        let launcher = msa_launcher(&dir, &uri, Some(MSA_CLIENT_ID));
        launcher
            .accounts()
            .add(stored_msa_account(NOT_EXPIRED))
            .expect("add account");
        let instance = launchable_instance(&launcher);

        // No token endpoint is mounted, so a refresh of this fresh token would fail the launch.
        let outcome = launcher
            .launch_instance(&instance.slug, None, None, true)
            .expect("dry run");
        let LaunchOutcome::DryRun(cmd) = outcome else {
            panic!("expected a dry run");
        };
        assert_eq!(arg_after(&cmd.args, "--username"), common::MSA_NAME);
        assert_eq!(arg_after(&cmd.args, "--userType"), "msa");
        assert_eq!(arg_after(&cmd.args, "--xuid"), common::MSA_XUID);
        assert_eq!(arg_after(&cmd.args, "--clientId"), MSA_CLIENT_ID);
        assert_eq!(arg_after(&cmd.args, "--accessToken"), MC_TOKEN);

        let hidden = cmd.redacted();
        assert_eq!(arg_after(&hidden.args, "--accessToken"), "<redacted>");
        assert!(
            !hidden.args.iter().any(|a| a.contains(MC_TOKEN)),
            "the printed command line carries no token: {:?}",
            hidden.args
        );
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launch_with_a_stale_token_refreshes_once_before_building_the_command() {
    let server = MockServer::start().await;
    common::mock_vanilla_with_arguments(&server, MC, common::ACCOUNT_ARGUMENTS).await;
    common::mount_token(&server, "refresh-2", 1).await;
    common::mount_msa_chain(&server, "mc-token-refreshed").await;

    let dir = tempfile::tempdir().expect("tempdir");
    let uri = server.uri();
    let dir = tokio::task::spawn_blocking(move || {
        let launcher = msa_launcher(&dir, &uri, Some(MSA_CLIENT_ID));
        launcher
            .accounts()
            .add(stored_msa_account(EXPIRED))
            .expect("add account");
        launcher
            .secrets()
            .put(common::MSA_ACCOUNT_ID, "refresh-1")
            .expect("seed the refresh token");
        let instance = launchable_instance(&launcher);

        let outcome = launcher
            .launch_instance(&instance.slug, None, None, true)
            .expect("dry run");
        let LaunchOutcome::DryRun(cmd) = outcome else {
            panic!("expected a dry run");
        };
        assert_eq!(
            arg_after(&cmd.args, "--accessToken"),
            "mc-token-refreshed",
            "the launch used the token the refresh returned"
        );
        assert_eq!(
            launcher.secrets().get(common::MSA_ACCOUNT_ID).expect("get"),
            Some("refresh-2".to_string()),
            "the rotated refresh token replaced the old one"
        );
        let saved = launcher
            .accounts()
            .find(common::MSA_ACCOUNT_ID)
            .expect("find")
            .expect("the account is still stored");
        assert_eq!(saved.mc_token.as_deref(), Some("mc-token-refreshed"));
        dir
    })
    .await
    .expect("blocking task");
    // `mount_token` expects exactly one request, so this proves one refresh, not two.
    server.verify().await;
    drop(dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn refreshing_an_offline_account_is_not_a_microsoft_account() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        // A client id is configured, so the refusal is about the account, not the config.
        let launcher = msa_launcher(&dir, "http://msa.invalid", Some("client-id"));
        launcher
            .accounts()
            .add(gcl_core::auth::offline::offline_account("Alice"))
            .expect("add account");

        let err = launcher
            .msa_refresh("Alice")
            .expect_err("an offline account has nothing to refresh");

        assert!(
            matches!(&err, gcl_core::Error::Auth(auth::Error::NotMicrosoft(name)) if name == "Alice"),
            "{err:?}"
        );
        assert_eq!(err.to_string(), "Alice is not a Microsoft account");
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_client_id_microsoft_login_is_disabled() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let launcher = msa_launcher(&dir, "http://msa.invalid", None);
        assert!(!launcher.msa_available());

        let err = launcher
            .msa_login(&|_code: &gcl_core::auth::msa::DeviceCode| {
                panic!("no code is ever asked for");
            })
            .expect_err("no client id");
        assert!(
            matches!(err, gcl_core::Error::Auth(auth::Error::Disabled)),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "microsoft login: disabled (set GCL_MSA_CLIENT_ID or keys.msa_client_id in config.toml)"
        );

        // A stale token cannot be refreshed either, so the launch says the same thing.
        launcher
            .accounts()
            .add(stored_msa_account(EXPIRED))
            .expect("add account");
        let instance = launchable_instance(&launcher);
        let err = launcher
            .launch_instance(&instance.slug, None, None, true)
            .expect_err("the token is stale and nothing can refresh it");
        assert!(
            matches!(err, gcl_core::Error::Auth(auth::Error::Disabled)),
            "{err:?}"
        );
        dir
    })
    .await
    .expect("blocking task");
}

/// A tiny shell script that stands in for a JVM: it prints one line and exits 3.
///
/// A real launch would need a real Java, which no unit test may depend on. The launch command
/// hands the script every Java argument it built; the script ignores them all.
#[cfg(unix)]
fn fake_java_script(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-java.sh");
    std::fs::write(&path, "#!/bin/sh\necho hello\nexit 3\n").expect("write fake java");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn an_async_launch_runs_the_game_and_reports_how_it_exited() {
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
        instance.config.jvm.java_path = Some(fake_java_script(dir.path()));
        instance.save().expect("save instance");

        let running = launcher
            .launch_instance_async(&instance.slug, None, Some("tester"))
            .expect("start the game");
        assert_eq!(running.slug, instance.slug);
        assert!(running.pid.is_some(), "the child was started");
        let log_path = running.log_path.clone();
        let launched_at = launcher
            .instances()
            .get(&instance.slug)
            .expect("reload")
            .config
            .last_launched;
        assert!(
            launched_at.is_some(),
            "the launch is recorded as soon as the process starts, not when it exits"
        );

        let outcome = running.wait_blocking(&launcher).expect("wait");
        let LaunchOutcome::Exited {
            code,
            log_path: reported,
            hint,
            stopped,
        } = outcome
        else {
            panic!("expected an exit, got {outcome:?}");
        };
        assert_eq!(code, 3);
        assert_eq!(reported, log_path);
        assert!(!stopped, "nothing asked this game to stop");
        assert!(hint.is_some(), "a non-zero exit carries a crash hint");

        assert!(log_path.is_file(), "{}", log_path.display());
        let written = std::fs::read_to_string(&log_path).expect("read the game log");
        assert!(written.contains("hello"), "{written}");
        assert!(
            launcher
                .instances()
                .get(&instance.slug)
                .expect("reload")
                .config
                .last_launched
                .is_some(),
            "the exit rewrote the timestamp, and did not clear it"
        );
        dir
    })
    .await
    .expect("blocking task");
}

/// A launcher over a fresh root with one vanilla instance, for the per-instance editors.
fn instance_launcher(dir: &tempfile::TempDir) -> (Launcher, String) {
    let launcher = launcher(dir, None, None);
    let slug = launcher
        .instances()
        .create("Pack", MC, Loader::None, None, &BTreeMap::new())
        .expect("create instance")
        .slug;
    (launcher, slug)
}

#[tokio::test(flavor = "multi_thread")]
async fn instance_overrides_are_set_and_unset_through_the_launcher() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let (launcher, slug) = instance_launcher(&dir);

        launcher
            .set_instance_override(&slug, "fov", "90")
            .expect("set the override");
        let saved = launcher.instances().get(&slug).expect("reload");
        assert_eq!(
            saved
                .config
                .settings_overrides
                .get("fov")
                .map(String::as_str),
            Some("90")
        );

        assert!(
            launcher
                .unset_instance_override(&slug, "fov")
                .expect("unset the override"),
            "the key was set, so unsetting it reports a change"
        );
        assert!(
            !launcher
                .unset_instance_override(&slug, "fov")
                .expect("unset again"),
            "unsetting a key that is not there reports no change"
        );
        assert!(
            launcher
                .instances()
                .get(&slug)
                .expect("reload")
                .config
                .settings_overrides
                .is_empty()
        );
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_override_with_a_bad_key_or_value_is_rejected_and_saves_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let (launcher, slug) = instance_launcher(&dir);

        let err = launcher
            .set_instance_override(&slug, "has:colon", "1")
            .expect_err("a key with a colon is not writable");
        assert!(
            matches!(
                err,
                gcl_core::Error::Settings(gcl_core::settings::Error::BadKey(_))
            ),
            "{err:?}"
        );
        let err = launcher
            .set_instance_override(&slug, "fov", "two\nlines")
            .expect_err("a value with a newline is not writable");
        assert!(
            matches!(
                err,
                gcl_core::Error::Settings(gcl_core::settings::Error::BadRawValue(_))
            ),
            "{err:?}"
        );
        assert!(
            launcher
                .instances()
                .get(&slug)
                .expect("reload")
                .config
                .settings_overrides
                .is_empty(),
            "a rejected override writes nothing"
        );
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn instance_jvm_is_saved_and_a_min_above_the_max_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let (launcher, slug) = instance_launcher(&dir);

        launcher
            .set_instance_jvm(
                &slug,
                InstanceJvm {
                    min_mib: Some(1024),
                    max_mib: Some(4096),
                    java_path: Some(std::path::PathBuf::from("/usr/bin/java")),
                    extra_args: vec!["-XX:+UseG1GC".to_string()],
                },
            )
            .expect("save the jvm settings");
        let saved = launcher.instances().get(&slug).expect("reload").config.jvm;
        assert_eq!(saved.min_mib, Some(1024));
        assert_eq!(saved.max_mib, Some(4096));
        assert_eq!(saved.extra_args, vec!["-XX:+UseG1GC".to_string()]);

        launcher
            .set_instance_jvm(
                &slug,
                InstanceJvm {
                    min_mib: Some(8192),
                    max_mib: Some(2048),
                    ..InstanceJvm::default()
                },
            )
            .expect_err("a minimum above the maximum is rejected");
        assert_eq!(
            launcher.instances().get(&slug).expect("reload").config.jvm,
            saved,
            "the rejected edit left the saved settings alone"
        );
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn instance_options_reads_the_pairs_the_game_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let (launcher, slug) = instance_launcher(&dir);
        let instance = launcher.instances().get(&slug).expect("read instance");

        assert!(
            launcher
                .instance_options(&slug)
                .expect("read options")
                .is_empty(),
            "an instance whose game never ran has no options.txt"
        );

        std::fs::write(
            instance.game_dir().join("options.txt"),
            "version:3465\nfov:0.5\nnot a pair\n",
        )
        .expect("write options.txt");
        assert_eq!(
            launcher.instance_options(&slug).expect("read options"),
            vec![
                ("version".to_string(), "3465".to_string()),
                ("fov".to_string(), "0.5".to_string()),
            ],
            "the pairs come back in file order, and a line with no colon is skipped"
        );
        dir
    })
    .await
    .expect("blocking task");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_worlds_reads_the_folders_under_saves() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::task::spawn_blocking(move || {
        let (launcher, slug) = instance_launcher(&dir);
        let saves = launcher
            .instances()
            .get(&slug)
            .expect("read instance")
            .game_dir()
            .join("saves");

        assert!(launcher.list_worlds(&slug).expect("list worlds").is_empty());

        std::fs::create_dir_all(saves.join("New World")).expect("world folder");
        std::fs::create_dir_all(saves.join("Amplified")).expect("world folder");
        std::fs::write(saves.join("stray.txt"), b"not a world").expect("stray file");
        assert_eq!(
            launcher.list_worlds(&slug).expect("list worlds"),
            vec!["Amplified".to_string(), "New World".to_string()],
            "folders only, in name order"
        );

        std::fs::remove_dir_all(&saves).expect("drop saves");
        assert!(
            launcher.list_worlds(&slug).expect("list worlds").is_empty(),
            "an instance with no saves directory has no worlds"
        );
        dir
    })
    .await
    .expect("blocking task");
}

/// A stand-in for `java`: it reports its start, then waits until a `SIGTERM` arrives.
///
/// The sleep runs in the background and `wait` is interrupted by the signal, so the trap
/// runs at once rather than after the sleep. Its output goes to `/dev/null`, so the
/// orphaned sleep does not hold the game log pipes open after the shell exits with 143,
/// the code a shell reports for "terminated by SIGTERM".
#[cfg(unix)]
const FAKE_JAVA: &str = "#!/bin/sh\n\
     trap 'kill $pid 2>/dev/null; exit 143' TERM\n\
     echo started\n\
     sleep 30 >/dev/null 2>&1 &\n\
     pid=$!\n\
     wait $pid\n";

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn stop_instance_terminates_the_game_and_clears_the_registry() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

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
        let java = dir.path().join("fake-java");
        std::fs::write(&java, FAKE_JAVA).expect("write the stand-in java");
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755))
            .expect("make it executable");
        instance.config.jvm.java_path = Some(java);
        instance.save().expect("save instance");
        let slug = instance.slug.clone();

        assert!(launcher.running_slugs().is_empty(), "nothing runs yet");
        let running = launcher
            .launch_instance_async(&slug, None, Some("tester"))
            .expect("launch");
        assert_eq!(launcher.running_slugs(), vec![slug.clone()]);
        assert!(running.pid.is_some(), "the game reports a pid");

        // The stand-in prints `started` once its trap is in place. Without this wait the
        // signal can arrive during shell startup, which kills it before the trap exists.
        let ready = Instant::now();
        while !std::fs::read_to_string(&running.log_path)
            .unwrap_or_default()
            .contains("started")
        {
            assert!(
                ready.elapsed() < Duration::from_secs(10),
                "the stand-in java never started"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        let started = Instant::now();
        launcher.stop_instance(&slug).expect("stop");
        let stopped = started.elapsed();
        assert!(
            stopped < Duration::from_secs(2),
            "SIGTERM was enough: {stopped:?}"
        );

        let outcome = running.wait_blocking(&launcher).expect("wait");
        let LaunchOutcome::Exited {
            code, stopped, hint, ..
        } = outcome
        else {
            panic!("expected an exit, got {outcome:?}");
        };
        assert_eq!(code, 143, "the stand-in traps SIGTERM and exits 143");
        assert!(stopped, "`stop_instance` asked for this exit");
        assert!(hint.is_none(), "a requested stop is not a crash");
        assert!(
            launcher.running_slugs().is_empty(),
            "the wait task cleared the registry"
        );

        let err = launcher.stop_instance(&slug).expect_err("nothing runs now");
        assert!(
            matches!(err, gcl_core::Error::Launch(gcl_core::launch::Error::NotRunning(ref s)) if *s == slug),
            "got {err:?}"
        );
        let err = launcher.stop_instance("nope").expect_err("no such instance");
        assert!(
            matches!(err, gcl_core::Error::Launch(gcl_core::launch::Error::NotRunning(_))),
            "got {err:?}"
        );
        dir
    })
    .await
    .expect("blocking task");
}
