//! Helpers shared by the integration tests: a zip builder and a mock vanilla version.
//!
//! Every test binary compiles this module on its own, so an item only one of them uses would
//! warn in the others.
#![allow(dead_code)]

use std::io::Write;

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

/// Builds an in-memory zip from `(archive name, bytes)` pairs.
pub fn zip_bytes(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(*name, opts).expect("start entry");
            zip.write_all(bytes).expect("write entry");
        }
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}

/// Builds a jar whose manifest declares `main` as its `Main-Class`.
pub fn jar_with_main(main: &str) -> Vec<u8> {
    let manifest = format!("Manifest-Version: 1.0\r\nMain-Class: {main}\r\n\r\n");
    zip_bytes(&[("META-INF/MANIFEST.MF", manifest.into_bytes())])
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
