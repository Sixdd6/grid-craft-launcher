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
fn debug_verify_source_is_not_implemented_yet() {
    Command::cargo_bin("gcl")
        .unwrap()
        .args(["debug", "verify-source", "modrinth"])
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
