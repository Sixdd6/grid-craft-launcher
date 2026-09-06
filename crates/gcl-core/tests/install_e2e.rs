//! End-to-end install of a synthetic vanilla version against a mock Mojang.

use std::collections::BTreeMap;
use std::io::Write;

use gcl_core::download::DownloadCtx;
use gcl_core::download::hash::sha1_hex;
use gcl_core::events::null_sink;
use gcl_core::http::HttpClient;
use gcl_core::mojang::version::{
    Artifact, AssetIndexRef, Downloads, Extract, Library, LibraryDownloads,
};
use gcl_core::mojang::{MANIFEST_PATH, Mojang, VersionJson, install_version_with};
use gcl_core::paths::Root;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A zip holding one native library and one file the extract filter must drop.
fn natives_jar() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.add_directory("META-INF/", opts).expect("dir entry");
        zip.start_file("a.so", opts).expect("start a.so");
        zip.write_all(b"\x7fELF native").expect("write a.so");
        zip.start_file("META-INF/x", opts).expect("start META-INF");
        zip.write_all(b"manifest junk").expect("write META-INF");
        zip.finish().expect("finish zip");
    }
    buf.into_inner()
}

async fn serve(server: &MockServer, at: &str, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path_matcher(at.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
}

const LIB_A_PATH: &str = "com/example/alpha/1.0/alpha-1.0.jar";
const LIB_B_PATH: &str = "com/example/beta/2.0/beta-2.0.jar";
const NATIVES_PATH: &str = "com/example/beta/2.0/beta-2.0-natives-linux.jar";

fn version_json(base: &str, natives_sha1: &str, natives_size: u64, index: &Artifact) -> String {
    let art = |path: &str, body: &[u8]| Artifact {
        path: Some(path.to_string()),
        sha1: sha1_hex(body),
        size: body.len() as u64,
        url: format!("{base}/{path}"),
    };
    let mut natives = BTreeMap::new();
    for os in ["linux", "windows", "osx"] {
        natives.insert(os.to_string(), "natives-linux".to_string());
    }
    let mut classifiers = BTreeMap::new();
    classifiers.insert(
        "natives-linux".to_string(),
        Artifact {
            path: Some(NATIVES_PATH.to_string()),
            sha1: natives_sha1.to_string(),
            size: natives_size,
            url: format!("{base}/{NATIVES_PATH}"),
        },
    );
    let v = VersionJson {
        id: "test-1.0".to_string(),
        inherits_from: None,
        main_class: Some("net.minecraft.client.main.Main".to_string()),
        arguments: None,
        minecraft_arguments: Some("--username ${auth_player_name}".to_string()),
        libraries: vec![
            Library {
                name: "com.example:alpha:1.0".to_string(),
                downloads: None,
                url: None,
                sha1: Some(sha1_hex(b"alpha jar")),
                size: Some(9),
                rules: Vec::new(),
                natives: None,
                extract: None,
            },
            Library {
                name: "com.example:beta:2.0".to_string(),
                downloads: Some(LibraryDownloads {
                    artifact: Some(art(LIB_B_PATH, b"beta jar!")),
                    classifiers: Some(classifiers),
                }),
                url: None,
                sha1: None,
                size: None,
                rules: Vec::new(),
                natives: Some(natives),
                extract: Some(Extract {
                    exclude: vec!["META-INF/".to_string()],
                }),
            },
        ],
        downloads: Some(Downloads {
            client: Artifact {
                path: None,
                sha1: sha1_hex(b"client jar"),
                size: 10,
                url: format!("{base}/client.jar"),
            },
            server: None,
            client_mappings: None,
        }),
        asset_index: Some(AssetIndexRef {
            id: "test-index".to_string(),
            sha1: index.sha1.clone(),
            size: index.size,
            total_size: Some(index.size),
            url: index.url.clone(),
        }),
        assets: Some("test-index".to_string()),
        java_version: None,
        logging: None,
        kind: Some("release".to_string()),
        release_time: None,
    };
    serde_json::to_string(&v).expect("serializes")
}

#[tokio::test]
async fn install_version_fetches_everything_and_unpacks_natives() {
    let server = MockServer::start().await;
    let base = server.uri();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    root.ensure_layout().expect("layout");

    let asset_a = b"asset one".to_vec();
    let asset_b = b"asset two".to_vec();
    let (hash_a, hash_b) = (sha1_hex(&asset_a), sha1_hex(&asset_b));
    let index_body = serde_json::json!({
        "objects": {
            "minecraft/lang/en_us.json": { "hash": hash_a, "size": asset_a.len() },
            "minecraft/sounds.json": { "hash": hash_b, "size": asset_b.len() },
        }
    })
    .to_string()
    .into_bytes();
    let index_ref = Artifact {
        path: None,
        sha1: sha1_hex(&index_body),
        size: index_body.len() as u64,
        url: format!("{base}/assets/index.json"),
    };

    let natives = natives_jar();
    let body = version_json(&base, &sha1_hex(&natives), natives.len() as u64, &index_ref);
    let manifest = serde_json::json!({
        "latest": { "release": "test-1.0", "snapshot": "test-1.0" },
        "versions": [{
            "id": "test-1.0",
            "type": "release",
            "url": format!("{base}/version.json"),
            "time": "2026-01-01T00:00:00+00:00",
            "releaseTime": "2026-01-01T00:00:00+00:00",
            "sha1": sha1_hex(body.as_bytes()),
            "complianceLevel": 1,
        }]
    })
    .to_string();

    serve(&server, MANIFEST_PATH, manifest.into_bytes()).await;
    serve(&server, "/version.json", body.into_bytes()).await;
    serve(&server, "/client.jar", b"client jar".to_vec()).await;
    serve(&server, &format!("/{LIB_A_PATH}"), b"alpha jar".to_vec()).await;
    serve(&server, &format!("/{LIB_B_PATH}"), b"beta jar!".to_vec()).await;
    serve(&server, &format!("/{NATIVES_PATH}"), natives).await;
    serve(&server, "/assets/index.json", index_body).await;
    serve(&server, &format!("/{}/{hash_a}", &hash_a[..2]), asset_a).await;
    serve(&server, &format!("/{}/{hash_b}", &hash_b[..2]), asset_b).await;

    let http = HttpClient::new().expect("client builds");
    let mojang = Mojang::with_base_url(http.clone(), root.clone(), base.clone());
    let sink = null_sink();
    let cancel = CancellationToken::new();
    let dl = DownloadCtx {
        http: &http,
        root: &root,
        sink: &sink,
        cancel: &cancel,
        parallel: 4,
    };

    let plan = install_version_with(&mojang, &dl, "test-1.0", Some(&base), &base)
        .await
        .expect("installs");

    assert!(plan.client_jar.is_file(), "{}", plan.client_jar.display());
    assert_eq!(plan.classpath.len(), 2);
    for jar in &plan.classpath {
        assert!(jar.is_file(), "{}", jar.display());
    }
    assert_eq!(plan.natives.len(), 1);
    assert!(plan.natives[0].0.is_file());
    assert!(
        root.assets_dir().join("indexes/test-index.json").is_file(),
        "asset index cached"
    );
    for hash in [&hash_a, &hash_b] {
        let object = root
            .assets_dir()
            .join("objects")
            .join(&hash[..2])
            .join(hash);
        assert!(object.is_file(), "{}", object.display());
    }
    assert_eq!(
        std::fs::read(plan.natives_dir.join("a.so")).expect("a.so extracted"),
        b"\x7fELF native"
    );
    assert!(
        !plan.natives_dir.join("META-INF").exists(),
        "excluded prefix stays out"
    );
    assert_eq!(plan.java_major, 8);
    assert_eq!(plan.asset_index_id, "test-index");
}
