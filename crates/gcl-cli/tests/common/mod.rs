//! Mock hosts shared by the CLI integration tests.
//!
//! Every test binary compiles this module on its own, so an item only one of them uses would
//! warn in the others.
#![allow(dead_code)]

use gcl_core::download::hash::sha1_hex;
use gcl_core::mojang::MANIFEST_PATH;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Serves `body` at `at` for every GET.
pub async fn serve(server: &MockServer, at: &str, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path_matcher(at.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
}

/// Client jar bytes the mock vanilla version serves.
pub const CLIENT_JAR: &[u8] = b"vanilla client jar";

/// Serves the manifest, version JSON, client jar, and an empty asset index for one version id.
///
/// The version has no libraries and no assets, so installing it fetches only the client jar.
pub async fn mock_vanilla(server: &MockServer, id: &str) {
    let base = server.uri();
    let index_body = serde_json::json!({ "objects": {} })
        .to_string()
        .into_bytes();
    let version = serde_json::json!({
        "id": id,
        "type": "release",
        "mainClass": "net.minecraft.client.main.Main",
        "minecraftArguments": "--username ${auth_player_name}",
        "libraries": [],
        "downloads": {
            "client": {
                "sha1": sha1_hex(CLIENT_JAR),
                "size": CLIENT_JAR.len(),
                "url": format!("{base}/vanilla/client.jar"),
            }
        },
        "assetIndex": {
            "id": "empty",
            "sha1": sha1_hex(&index_body),
            "size": index_body.len(),
            "url": format!("{base}/vanilla/index.json"),
        },
        "assets": "empty",
    })
    .to_string();
    let manifest = serde_json::json!({
        "latest": { "release": id, "snapshot": id },
        "versions": [{
            "id": id,
            "type": "release",
            "url": format!("{base}/vanilla/version.json"),
            "time": "2026-01-01T00:00:00+00:00",
            "releaseTime": "2026-01-01T00:00:00+00:00",
            "sha1": sha1_hex(version.as_bytes()),
            "complianceLevel": 1,
        }]
    })
    .to_string();

    serve(server, MANIFEST_PATH, manifest.into_bytes()).await;
    serve(server, "/vanilla/version.json", version.into_bytes()).await;
    serve(server, "/vanilla/client.jar", CLIENT_JAR.to_vec()).await;
    serve(server, "/vanilla/index.json", index_body).await;
}

/// Modrinth search fixture, answered at `/search`.
pub const MODRINTH_SEARCH: &str =
    include_str!("../../../../tests/fixtures/modrinth/search_sodium.json");

/// Modrinth project fixture for Sodium, answered at `/project/sodium`.
pub const MODRINTH_PROJECT: &str =
    include_str!("../../../../tests/fixtures/modrinth/project_sodium.json");

/// Modrinth versions fixture for Sodium on 1.20.1 Fabric.
pub const MODRINTH_VERSIONS: &str =
    include_str!("../../../../tests/fixtures/modrinth/versions_sodium_1.20.1_fabric.json");

/// Project id the Sodium fixtures carry.
pub const SODIUM_ID: &str = "AANobbMI";

/// Bytes the mock Modrinth serves as the Sodium jar.
pub const SODIUM_JAR: &[u8] = b"synthetic sodium jar";

/// Serves search, the Sodium project, one version rewritten to this server, and the jar.
///
/// The fixture's file points at Modrinth's CDN and hashes the real jar, so the URL and the
/// sha1 are both rewritten here: the launcher verifies every byte it downloads.
pub async fn mock_modrinth(server: &MockServer) {
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

    serve(server, "/search", MODRINTH_SEARCH.as_bytes().to_vec()).await;
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
