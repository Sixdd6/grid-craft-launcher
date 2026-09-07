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

/// A CurseForge mock whose only file has a null `downloadUrl`.
///
/// The class list, the slug lookup, and the files list are the three calls
/// `content add --source curseforge` makes for one project.
async fn mock_curseforge_without_a_download_url(server: &MockServer) {
    const CLASSES: &str =
        include_str!("../../../tests/fixtures/curseforge/categories_classes.json");
    const SEARCH: &str = include_str!("../../../tests/fixtures/curseforge/search_mods.json");
    const FILES: &str = include_str!("../../../tests/fixtures/curseforge/get_mod_files.json");

    let mut files: serde_json::Value = serde_json::from_str(FILES).expect("files fixture is json");
    // The author opted out of third-party distribution, so the API sends no URL.
    for file in files["data"].as_array_mut().expect("a data array") {
        file["downloadUrl"] = serde_json::Value::Null;
    }

    common::serve(server, "/v1/categories", CLASSES.as_bytes().to_vec()).await;
    common::serve(server, "/v1/mods/search", SEARCH.as_bytes().to_vec()).await;
    common::serve(
        server,
        "/v1/mods/394468/files",
        files.to_string().into_bytes(),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_add_of_a_file_without_a_url_exits_three_and_lists_it_as_pending() {
    let server = MockServer::start().await;
    mock_curseforge_without_a_download_url(&server).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    let pending = tokio::task::spawn_blocking(move || {
        create_instance(&root, "http://modrinth.invalid", "demo");
        let mut add = gcl(&root, "http://modrinth.invalid");
        add.env("CURSEFORGE_API_KEY", "test");
        add.env("GCL_CURSEFORGE_BASE_URL", &uri);
        add.args([
            "content",
            "add",
            "demo",
            "--source",
            "curseforge",
            "--project",
            "sodium",
        ])
        .assert()
        .code(3)
        .stdout(predicates::str::contains(
            "https://www.curseforge.com/minecraft/mc-mods/sodium/files/5230381",
        ));

        let mut list = gcl(&root, "http://modrinth.invalid");
        list.env("CURSEFORGE_API_KEY", "test");
        list.env("GCL_CURSEFORGE_BASE_URL", &uri);
        list.args(["--json", "content", "pending", "demo"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    })
    .await
    .expect("commands run");

    let parsed: serde_json::Value = serde_json::from_slice(&pending).expect("stdout is json");
    let rows = parsed.as_array().expect("an array of pending downloads");
    assert_eq!(rows.len(), 1, "{parsed}");
    assert_eq!(rows[0]["file_name"], "sodium-fabric-0.5.13+mc1.20.1.jar");
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
