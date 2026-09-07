//! CLI tests for `gcl account add-msa`, `gcl account refresh`, and `verify-source msa`.
//!
//! Every request goes to one mock server: the six `GCL_MSA_*_URL` overrides point the login
//! chain at it, so no test talks to Microsoft. `GCL_NO_KEYRING=1` keeps the refresh token in
//! the temporary app root instead of the developer's OS keyring.

use std::path::Path;

use assert_cmd::Command;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Profile id the mock Minecraft services answers with, as 32 undashed hex digits.
const PROFILE_ID: &str = "b50ad385829d3141a2167e7d7539ba7f";

/// The name the mock profile carries.
const PROFILE_NAME: &str = "Notch";

/// A `gcl` command whose app root is `root` and whose Microsoft login is turned off.
fn gcl(root: &Path) -> Command {
    let mut cmd = Command::cargo_bin("gcl").expect("gcl binary builds");
    cmd.env("GCL_ROOT", root);
    cmd.env("GCL_NO_KEYRING", "1");
    // A client id in the developer's environment must not decide what a test sees.
    cmd.env_remove("GCL_MSA_CLIENT_ID");
    cmd
}

/// A `gcl` command with a client id and every login endpoint pointed at `uri`.
fn gcl_msa(root: &Path, uri: &str) -> Command {
    let mut cmd = gcl(root);
    cmd.env("GCL_MSA_CLIENT_ID", "test-client");
    cmd.env("GCL_MSA_DEVICE_URL", format!("{uri}/devicecode"));
    cmd.env("GCL_MSA_TOKEN_URL", format!("{uri}/token"));
    cmd.env("GCL_MSA_XBL_URL", format!("{uri}/xbl"));
    cmd.env("GCL_MSA_XSTS_URL", format!("{uri}/xsts"));
    cmd.env("GCL_MSA_MC_URL", format!("{uri}/mclogin"));
    cmd.env("GCL_MSA_PROFILE_URL", format!("{uri}/profile"));
    cmd
}

/// Answers `body` with HTTP 200 for every POST at `at`.
async fn post_json(server: &MockServer, at: &str, body: String) {
    Mock::given(method("POST"))
        .and(path_matcher(at.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

/// Mounts the whole login chain, approving the device code on the first poll.
///
/// The device code asks for a zero second interval, so the poll loop does not wait.
async fn mock_msa(server: &MockServer) {
    post_json(
        server,
        "/devicecode",
        r#"{"user_code":"ABCD-EFGH","device_code":"dev-secret",
            "verification_uri":"https://microsoft.com/link",
            "expires_in":900,"interval":0,"message":"Sign in at the link."}"#
            .to_string(),
    )
    .await;
    post_json(
        server,
        "/token",
        r#"{"access_token":"msa-access","refresh_token":"msa-refresh","expires_in":3600}"#
            .to_string(),
    )
    .await;
    post_json(
        server,
        "/xbl",
        r#"{"Token":"xbl-token","DisplayClaims":{"xui":[{"uhs":"user-hash"}]}}"#.to_string(),
    )
    .await;
    post_json(
        server,
        "/xsts",
        r#"{"Token":"xsts-token","DisplayClaims":{"xui":[{"uhs":"user-hash","xid":"2535"}]}}"#
            .to_string(),
    )
    .await;
    post_json(
        server,
        "/mclogin",
        r#"{"access_token":"mc-token","expires_in":86400}"#.to_string(),
    )
    .await;
    Mock::given(method("GET"))
        .and(path_matcher("/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"id":"{PROFILE_ID}","name":"{PROFILE_NAME}"}}"#
        )))
        .mount(server)
        .await;
}

/// Parses stdout as one JSON document per line.
fn json_lines(out: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(out)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("every stdout line is json"))
        .collect()
}

/// How many requests the mock server saw at one path.
async fn hits(server: &MockServer, at: &str) -> usize {
    server
        .received_requests()
        .await
        .expect("the mock server records requests")
        .iter()
        .filter(|r| r.url.path() == at)
        .count()
}

#[test]
fn account_add_msa_without_a_client_id_is_disabled() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["account", "add-msa"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("disabled"));
}

#[test]
fn debug_verify_source_msa_skips_without_a_client_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["debug", "verify-source", "msa"])
        .assert()
        .success()
        .stdout(predicates::str::contains("SKIP msa"));
}

#[test]
fn debug_verify_source_msa_passes_with_a_client_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .env("GCL_MSA_CLIENT_ID", "test-client")
        .args(["debug", "verify-source", "msa"])
        .assert()
        .success()
        .stdout(predicates::str::contains("PASS msa"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_msa_signs_in_and_list_and_refresh_report_the_account() {
    let server = MockServer::start().await;
    mock_msa(&server).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    let login = {
        let (root, uri) = (root.clone(), uri.clone());
        tokio::task::spawn_blocking(move || {
            gcl_msa(&root, &uri)
                .args(["--json", "account", "add-msa"])
                .assert()
                .success()
                .get_output()
                .stdout
                .clone()
        })
        .await
        .expect("add-msa runs")
    };
    let lines = json_lines(&login);
    assert_eq!(lines.len(), 2, "expected two json lines, got {lines:?}");
    assert_eq!(lines[0]["user_code"], "ABCD-EFGH");
    assert_eq!(lines[0]["verification_uri"], "https://microsoft.com/link");
    assert_eq!(lines[1]["name"], PROFILE_NAME);
    assert_eq!(lines[1]["kind"], "msa");

    let listed = {
        let (root, uri) = (root.clone(), uri.clone());
        tokio::task::spawn_blocking(move || {
            gcl_msa(&root, &uri)
                .args(["--json", "account", "list"])
                .assert()
                .success()
                .get_output()
                .stdout
                .clone()
        })
        .await
        .expect("account list runs")
    };
    let rows: serde_json::Value = serde_json::from_slice(&listed).expect("stdout is json");
    let rows = rows.as_array().expect("an array of rows");
    assert_eq!(rows.len(), 1, "expected one account, got {rows:?}");
    assert_eq!(rows[0]["kind"], "msa");
    assert_eq!(rows[0]["active"], true);
    let expires = rows[0]["expires"].as_str().unwrap_or_default();
    assert!(!expires.is_empty(), "no expiry in {:?}", rows[0]);

    assert_eq!(hits(&server, "/token").await, 1, "one poll signs in");

    let refreshed = {
        let (root, uri) = (root.clone(), uri.clone());
        tokio::task::spawn_blocking(move || {
            gcl_msa(&root, &uri)
                .args(["--json", "account", "refresh", PROFILE_NAME])
                .assert()
                .success()
                .get_output()
                .stdout
                .clone()
        })
        .await
        .expect("account refresh runs")
    };
    let row: serde_json::Value = serde_json::from_slice(&refreshed).expect("stdout is json");
    assert_eq!(row["kind"], "msa");
    assert_eq!(row["name"], PROFILE_NAME);
    assert!(
        !row["expires"].as_str().unwrap_or_default().is_empty(),
        "no expiry in {row:?}"
    );
    assert_eq!(hits(&server, "/token").await, 2, "refresh redeems once");
}

#[test]
fn account_refresh_of_an_offline_account_fails_without_a_client_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["account", "add-offline", "alice"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["account", "refresh", "alice"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("disabled"));
}

#[test]
fn account_list_text_has_an_expires_column() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["account", "add-offline", "alice"])
        .assert()
        .success();
    gcl(dir.path())
        .args(["account", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("EXPIRES"));
}

#[test]
fn launch_without_an_account_prints_the_offline_hint() {
    let dir = tempfile::tempdir().expect("tempdir");
    gcl(dir.path())
        .args(["launch", "demo"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("no account selected"))
        .stderr(predicates::str::contains(
            "hint: use --offline-user <name> to play offline",
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn add_msa_text_prints_the_link_and_the_code() {
    let server = MockServer::start().await;
    mock_msa(&server).await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let uri = server.uri();

    tokio::task::spawn_blocking(move || {
        gcl_msa(&root, &uri)
            .args(["account", "add-msa"])
            .assert()
            .success()
            .stdout(predicates::str::contains(
                "Open https://microsoft.com/link and enter code ABCD-EFGH",
            ))
            .stdout(predicates::str::contains("Sign in at the link."));
    })
    .await
    .expect("add-msa runs");
}
