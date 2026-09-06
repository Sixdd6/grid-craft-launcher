//! CLI tests for `gcl content` and `gcl modpack`, answered by a mock Modrinth.

use std::path::Path;

use assert_cmd::Command;
use wiremock::MockServer;

mod common;

/// A `gcl` command whose app root is `root` and whose Modrinth base URL is `uri`.
///
/// `CURSEFORGE_API_KEY` is removed, so a key on the developer's machine cannot decide what
/// a test sees.
fn gcl(root: &Path, uri: &str) -> Command {
    let mut cmd = Command::cargo_bin("gcl").expect("gcl binary builds");
    cmd.env("GCL_ROOT", root);
    cmd.env("GCL_MODRINTH_BASE_URL", uri);
    cmd.env_remove("CURSEFORGE_API_KEY");
    cmd
}

/// Creates a Fabric 1.20.1 instance, which needs no network.
fn create_instance(root: &Path, uri: &str, name: &str) {
    gcl(root, uri)
        .args([
            "instance",
            "create",
            name,
            "--minecraft",
            "1.20.1",
            "--loader",
            "fabric",
        ])
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_search_json_lists_a_hit() {
    let server = MockServer::start().await;
    common::mock_modrinth(&server).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    let out = tokio::task::spawn_blocking(move || {
        gcl(&root, &uri)
            .args(["--json", "content", "search", "sodium"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    })
    .await
    .expect("command runs");

    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
    let hits = parsed["hits"].as_array().expect("a hits array");
    assert!(
        hits.iter().any(|h| h["slug"] == "sodium"),
        "no sodium hit in {parsed}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_add_then_list_shows_the_mod_and_disable_flips_it() {
    let server = MockServer::start().await;
    common::mock_modrinth(&server).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    tokio::task::spawn_blocking(move || {
        create_instance(&root, &uri, "demo");
        gcl(&root, &uri)
            .args([
                "content",
                "add",
                "demo",
                "--source",
                "modrinth",
                "--project",
                "sodium",
            ])
            .assert()
            .success();

        let out = gcl(&root, &uri)
            .args(["--json", "content", "list", "demo"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
        let entries = parsed.as_array().expect("an array");
        assert_eq!(entries.len(), 1, "{parsed}");
        assert_eq!(entries[0]["project_id"], common::SODIUM_ID);
        assert_eq!(entries[0]["file_name"], "sodium.jar");
        assert_eq!(entries[0]["enabled"], true, "{parsed}");

        // Nothing needed a hand download, so the pending list is empty.
        let out = gcl(&root, &uri)
            .args(["--json", "content", "pending", "demo"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
        assert_eq!(parsed, serde_json::json!([]), "{parsed}");

        gcl(&root, &uri)
            .args(["content", "disable", "demo", common::SODIUM_ID])
            .assert()
            .success();
        let out = gcl(&root, &uri)
            .args(["--json", "content", "list", "demo"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is json");
        assert_eq!(parsed[0]["enabled"], false, "{parsed}");
    })
    .await
    .expect("command runs");
}

#[test]
fn content_add_from_curseforge_without_a_key_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    create_instance(dir.path(), "http://modrinth.invalid", "demo");
    gcl(dir.path(), "http://modrinth.invalid")
        .args([
            "content",
            "add",
            "demo",
            "--source",
            "curseforge",
            "--project",
            "jei",
        ])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("disabled"));
}

#[test]
fn content_search_of_an_unknown_source_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path(), "http://modrinth.invalid")
        .args(["content", "search", "sodium", "--source", "nowhere"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("unknown source"));
}

#[test]
fn content_import_file_without_a_pending_download_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    create_instance(dir.path(), "http://modrinth.invalid", "demo");
    let jar = dir.path().join("hand.jar");
    std::fs::write(&jar, b"hand downloaded").expect("write jar");
    gcl(dir.path(), "http://modrinth.invalid")
        .args(["content", "import-file", "demo"])
        .arg(&jar)
        .args(["--kind", "mod"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("no pending download matches"));
}

#[test]
fn modpack_install_file_rejects_an_archive_that_is_not_a_pack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = dir.path().join("not-a-pack.zip");
    std::fs::write(&zip, b"not a zip at all").expect("write zip");
    gcl(dir.path(), "http://modrinth.invalid")
        .args(["modpack", "install-file"])
        .arg(&zip)
        .assert()
        .code(1);
}
