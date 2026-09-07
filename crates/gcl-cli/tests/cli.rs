use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MANIFEST: &str = include_str!("../../../tests/fixtures/mojang/version_manifest_v2.json");
const MANIFEST_PATH: &str = "/mc/game/version_manifest_v2.json";

/// A `gcl` command whose app root is the given directory.
fn gcl(root: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gcl").expect("gcl binary builds");
    cmd.env("GCL_ROOT", root);
    cmd
}

/// A mock piston-meta serving the manifest fixture, with every version URL pointed at it.
async fn mock_mojang() -> MockServer {
    let server = MockServer::start().await;
    let body = MANIFEST.replace("https://piston-meta.mojang.com", &server.uri());
    Mock::given(method("GET"))
        .and(path_matcher(MANIFEST_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    server
}

#[test]
fn version_flag_prints_version() {
    Command::cargo_bin("gcl")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn debug_verify_source_rejects_a_source_it_cannot_check() {
    Command::cargo_bin("gcl")
        .unwrap()
        .args(["debug", "verify-source", "nowhere"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("not implemented"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_list_json_prints_the_manifest_entries() {
    let server = mock_mojang().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    let out = tokio::task::spawn_blocking(move || {
        gcl(&root)
            .env("GCL_MOJANG_BASE_URL", uri)
            .args(["--json", "version", "list"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    })
    .await
    .expect("command runs");

    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    let entries = parsed.as_array().expect("an array");
    assert!(
        entries.iter().any(|e| e["id"] == "1.20.1"),
        "1.20.1 missing from {parsed}"
    );
    assert!(
        !entries.iter().any(|e| e["id"] == "26.3-pre-2"),
        "snapshots listed without --snapshots: {parsed}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_list_snapshots_includes_the_latest_snapshot() {
    let server = mock_mojang().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    tokio::task::spawn_blocking(move || {
        gcl(&root)
            .env("GCL_MOJANG_BASE_URL", uri)
            .args(["version", "list", "--snapshots"])
            .assert()
            .success()
            .stdout(predicates::str::contains("26.3-pre-2"));
    })
    .await
    .expect("command runs");
}

#[test]
fn instance_create_then_list_shows_the_slug() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["instance", "create", "Test", "--minecraft", "1.20.1"])
        .assert()
        .success();

    let out = gcl(dir.path())
        .args(["--json", "instance", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    let entries = parsed.as_array().expect("an array");
    assert_eq!(entries.len(), 1, "{parsed}");
    assert_eq!(entries[0]["slug"], "test");
    assert_eq!(entries[0]["name"], "Test");
    assert_eq!(entries[0]["minecraft"], "1.20.1");
    assert_eq!(entries[0]["loader"], "none");
}

#[test]
fn instance_delete_needs_yes_and_then_empties_the_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["instance", "create", "Test", "--minecraft", "1.20.1"])
        .assert()
        .success();

    gcl(dir.path())
        .args(["instance", "delete", "test"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("--yes"));

    gcl(dir.path())
        .args(["instance", "delete", "test", "--yes"])
        .assert()
        .success();

    let out = gcl(dir.path())
        .args(["--json", "instance", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    assert_eq!(parsed.as_array().expect("an array").len(), 0);
}

#[test]
fn instance_rename_changes_the_display_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["instance", "create", "Test", "--minecraft", "1.20.1"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["instance", "rename", "test", "Renamed"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["instance", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Renamed"));
}

#[test]
fn config_set_jvm_is_written_and_shown() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["config", "set-jvm", "--min", "1024", "--max", "4096"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicates::str::contains("max_mib = 4096"));
}

#[test]
fn config_show_redacts_keys() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("config.toml"),
        "[keys]\ncurseforge_api_key = \"super-secret\"\n",
    )
    .expect("write config");
    gcl(dir.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(
            predicates::str::contains("<set>").and(predicates::str::contains("super-secret").not()),
        );
}

#[test]
fn config_set_root_is_saved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["config", "set-root", &target.path().display().to_string()])
        .assert()
        .success();
    let text = std::fs::read_to_string(dir.path().join("config.toml")).expect("config written");
    assert!(
        text.contains(&target.path().display().to_string()),
        "{text}"
    );
}

#[test]
fn java_list_exits_zero() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path()).args(["java", "list"]).assert().success();
}

#[test]
fn java_list_json_is_an_array() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = gcl(dir.path())
        .args(["--json", "java", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    assert!(parsed.is_array(), "{parsed}");
}

#[test]
fn unknown_instance_reports_an_error_and_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["instance", "rename", "nope", "New"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("error:"));
}

mod common;

const FABRIC_LOADERS: &str = include_str!("../../../tests/fixtures/fabric/loader_1.20.1.json");
const MC: &str = "1.20.1";

/// A mock Fabric meta serving the loader list fixture for 1.20.1.
async fn mock_fabric() -> MockServer {
    let server = MockServer::start().await;
    common::serve(
        &server,
        &format!("/v2/versions/loader/{MC}"),
        FABRIC_LOADERS.as_bytes().to_vec(),
    )
    .await;
    server
}

/// Writes a `config.toml` that pins the java binary, so no test reaches the runtime endpoints.
fn pin_java(root: &Path) {
    std::fs::write(
        root.join("config.toml"),
        "[jvm]\njava_path = \"/usr/bin/java\"\n",
    )
    .expect("write config");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loader_list_json_prints_every_build() {
    let server = mock_fabric().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    let out = tokio::task::spawn_blocking(move || {
        gcl(&root)
            .env("GCL_FABRIC_BASE_URL", uri)
            .args(["--json", "loader", "list", MC, "--loader", "fabric"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    })
    .await
    .expect("command runs");

    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    let entries = parsed.as_array().expect("an array");
    assert_eq!(entries.len(), 3, "{parsed}");
    assert_eq!(entries[0]["version"], "0.19.5");
    assert_eq!(entries[0]["recommended"], true);
}

#[test]
fn account_add_offline_becomes_the_active_account() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["account", "add-offline", "alice"])
        .assert()
        .success()
        .stdout(predicates::str::contains("alice"));

    let out = gcl(dir.path())
        .args(["--json", "account", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    let entries = parsed.as_array().expect("an array");
    assert_eq!(entries.len(), 1, "{parsed}");
    assert_eq!(entries[0]["name"], "alice");
    assert_eq!(entries[0]["kind"], "offline");
    assert_eq!(entries[0]["active"], true);
}

#[test]
fn account_remove_takes_a_name_and_empties_the_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["account", "add-offline", "alice"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["account", "remove", "alice"])
        .assert()
        .success();
    let out = gcl(dir.path())
        .args(["--json", "account", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    assert_eq!(parsed.as_array().expect("an array").len(), 0);
}

#[test]
fn settings_set_is_shown_as_an_override_over_the_preseeded_options() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["settings", "defaults", "set", "renderDistance", "8"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["instance", "create", "Demo", "--minecraft", MC])
        .assert()
        .success();
    gcl(dir.path())
        .args(["settings", "set", "demo", "renderDistance", "16"])
        .assert()
        .success();

    let out = gcl(dir.path())
        .args(["--json", "settings", "show", "demo"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    assert_eq!(parsed["overrides"]["renderDistance"], "16", "{parsed}");
    assert_eq!(parsed["options"]["renderDistance"], "8", "{parsed}");

    gcl(dir.path())
        .args(["settings", "unset", "demo", "renderDistance"])
        .assert()
        .success();
    let out = gcl(dir.path())
        .args(["--json", "settings", "show", "demo"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    assert!(parsed["overrides"].as_object().expect("a map").is_empty());
}

#[test]
fn settings_set_rejects_a_value_outside_the_catalog_range() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["instance", "create", "Demo", "--minecraft", MC])
        .assert()
        .success();

    gcl(dir.path())
        .args(["settings", "set", "demo", "renderDistance", "999"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("must be between 2 and 32"));

    gcl(dir.path())
        .args(["settings", "defaults", "set", "renderDistance", "999"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("must be between 2 and 32"));

    // A valid value still goes through, and the rejected one was never saved.
    gcl(dir.path())
        .args(["settings", "set", "demo", "renderDistance", "16"])
        .assert()
        .success();
    let out = gcl(dir.path())
        .args(["--json", "settings", "show", "demo"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    assert_eq!(parsed["overrides"]["renderDistance"], "16", "{parsed}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_dry_run_names_the_offline_user() {
    let server = MockServer::start().await;
    common::mock_vanilla(&server, MC).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    let out = tokio::task::spawn_blocking(move || {
        pin_java(&root);
        gcl(&root)
            .env("GCL_MOJANG_BASE_URL", &uri)
            .args(["instance", "create", "Demo", "--minecraft", MC])
            .assert()
            .success();
        gcl(&root)
            .env("GCL_MOJANG_BASE_URL", &uri)
            .args(["launch", "demo", "--offline-user", "bob", "--dry-run"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    })
    .await
    .expect("command runs");

    let text = String::from_utf8(out).expect("utf-8 stdout");
    assert!(text.starts_with("program: "), "{text}");
    assert!(text.contains("\nargs:\n"), "{text}");
    let args: Vec<&str> = text.lines().map(str::trim).collect();
    let user = args
        .iter()
        .position(|a| *a == "--username")
        .expect("a --username argument");
    assert_eq!(args[user + 1], "bob", "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_dry_run_json_is_the_launch_command() {
    let server = MockServer::start().await;
    common::mock_vanilla(&server, MC).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    let out = tokio::task::spawn_blocking(move || {
        pin_java(&root);
        gcl(&root)
            .env("GCL_MOJANG_BASE_URL", &uri)
            .args(["instance", "create", "Demo", "--minecraft", MC])
            .assert()
            .success();
        gcl(&root)
            .env("GCL_MOJANG_BASE_URL", &uri)
            .args([
                "--json",
                "launch",
                "demo",
                "--offline-user",
                "bob",
                "--dry-run",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    })
    .await
    .expect("command runs");

    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    assert_eq!(parsed["program"], "/usr/bin/java", "{parsed}");
    let args: Vec<String> = parsed["args"]
        .as_array()
        .expect("an args array")
        .iter()
        .map(|a| a.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(args.iter().any(|a| a == "bob"), "{parsed}");
}

#[test]
fn launch_of_an_unknown_instance_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["launch", "nope", "--offline-user", "bob", "--dry-run"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("error:"));
}

#[test]
fn debug_verify_source_skips_curseforge_without_a_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .env_remove("CURSEFORGE_API_KEY")
        .args(["debug", "verify-source", "curseforge"])
        .assert()
        .success()
        .stdout(predicates::str::contains("SKIP curseforge"));
}

#[test]
fn settings_set_rejects_a_key_options_txt_cannot_hold() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["instance", "create", "Demo", "--minecraft", MC])
        .assert()
        .success();
    let toml_path = dir
        .path()
        .join("instances")
        .join("demo")
        .join("instance.toml");
    let before = std::fs::read_to_string(&toml_path).expect("instance.toml");

    for key in ["a:b", "", " a", "a\nb"] {
        gcl(dir.path())
            .args(["settings", "set", "demo", key, "x"])
            .assert()
            .code(1)
            .stderr(predicates::str::contains("error:"));
    }
    gcl(dir.path())
        .args(["settings", "defaults", "set", "a:b", "x"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("error:"));

    let after = std::fs::read_to_string(&toml_path).expect("instance.toml");
    assert_eq!(before, after, "instance.toml was rewritten");
}

#[test]
fn account_remove_of_the_active_account_says_none_is_active() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["account", "add-offline", "alice"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["account", "remove", "alice"])
        .assert()
        .success()
        .stdout(predicates::str::contains("no active account now"));
}
