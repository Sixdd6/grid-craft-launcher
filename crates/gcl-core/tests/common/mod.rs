//! Helpers shared by the integration tests: a zip builder, a mock vanilla version, a
//! synthetic Forge installer, and the fake processor runner that stands in for a JVM.
//!
//! Every test binary compiles this module on its own, so an item only one of them uses would
//! warn in the others.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gcl_core::download::hash::sha1_hex;
use gcl_core::java::{JavaInstall, JavaSource};
use gcl_core::loaders::ProcessRunner;
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

/// log4j2 configuration bytes the mock vanilla version serves.
pub const LOG_CONFIG: &[u8] = b"<Configuration></Configuration>";

/// File name the served log4j2 configuration is cached under.
pub const LOG_CONFIG_ID: &str = "client-1.12.xml";

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
        "logging": { "client": {
            "argument": "-Dlog4j.configurationFile=${path}",
            "type": "log4j2-xml",
            "file": {
                "id": LOG_CONFIG_ID,
                "sha1": sha1_hex(LOG_CONFIG),
                "size": LOG_CONFIG.len(),
                "url": format!("{base}/vanilla/log4j2.xml"),
            },
        }},
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
    serve(server, "/vanilla/log4j2.xml", LOG_CONFIG.to_vec()).await;
}

/// Minecraft version the synthetic Forge installer targets.
pub const FORGE_MC: &str = "1.20.1";

/// Forge build the synthetic installer claims to be.
pub const FORGE_VERSION: &str = "47.4.10";

/// Version id a Forge install of [`FORGE_VERSION`] resolves to.
pub const FORGE_ID: &str = "1.20.1-forge-47.4.10";

/// Main class of the one processor the synthetic installer runs.
pub const PROC_MAIN: &str = "com.example.Proc";

/// Library path of the processor jar.
pub const PROC_PATH: &str = "com/example/proc/1.0/proc-1.0.jar";

/// Library path of a plain version library the installer's `version.json` names.
pub const VLIB_PATH: &str = "com/example/vlib/2.0/vlib-2.0.jar";

/// Library path of the universal jar, which is extracted from the installer's `maven/`.
pub const UNIVERSAL_PATH: &str =
    "net/minecraftforge/forge/1.20.1-47.4.10/forge-1.20.1-47.4.10-universal.jar";

/// Library path of the patched client jar the processor writes.
pub const PATCHED_PATH: &str =
    "net/minecraftforge/forge/1.20.1-47.4.10/forge-1.20.1-47.4.10-client.jar";

/// Bytes the fake processor writes as the patched client jar.
pub const PATCHED_BYTES: &[u8] = b"patched client jar";

/// Bytes of the universal jar inside the installer's `maven/`.
pub const UNIVERSAL_BYTES: &[u8] = b"universal jar";

/// Bytes of the plain version library.
pub const VLIB_BYTES: &[u8] = b"vlib jar";

/// Builds a Forge installer jar with one client processor, a `maven/` entry, and a `data/` file.
///
/// `base` is the mock server serving the libraries it names.
pub fn forge_installer_jar(base: &str) -> Vec<u8> {
    let lib = |name: &str, path: &str, body: &[u8]| {
        serde_json::json!({
            "name": name,
            "downloads": { "artifact": {
                "path": path,
                "url": format!("{base}/libs/{path}"),
                "sha1": sha1_hex(body),
                "size": body.len(),
            }}
        })
    };
    // The universal jar comes out of `maven/` inside the installer, so its artifact has no URL.
    let universal = serde_json::json!({
        "name": "net.minecraftforge:forge:1.20.1-47.4.10:universal",
        "downloads": { "artifact": {
            "path": UNIVERSAL_PATH,
            "url": "",
            "sha1": sha1_hex(UNIVERSAL_BYTES),
            "size": UNIVERSAL_BYTES.len(),
        }}
    });
    let profile = serde_json::json!({
        "spec": 1,
        "profile": "forge",
        "version": FORGE_ID,
        "minecraft": FORGE_MC,
        "json": "/version.json",
        "data": {
            "BINPATCH": { "client": "/data/client.lzma", "server": "/data/server.lzma" },
            "PATCHED": {
                "client": "[net.minecraftforge:forge:1.20.1-47.4.10:client]",
                "server": "[net.minecraftforge:forge:1.20.1-47.4.10:server]",
            },
            "PATCHED_SHA": {
                "client": format!("'{}'", sha1_hex(PATCHED_BYTES)),
                "server": "'0000000000000000000000000000000000000000'",
            },
        },
        "processors": [
            {
                "jar": "com.example:proc:1.0",
                "classpath": [],
                "args": [
                    "--input", "{MINECRAFT_JAR}",
                    "--patch", "{BINPATCH}",
                    "--output", "{PATCHED}",
                ],
                "outputs": { "{PATCHED}": "{PATCHED_SHA}" },
            },
            {
                "sides": ["server"],
                "jar": "com.example:proc:1.0",
                "args": ["--server"],
            },
        ],
        "libraries": [lib("com.example:proc:1.0", PROC_PATH, &jar_with_main(PROC_MAIN))],
    });
    let version = serde_json::json!({
        "id": "placeholder",
        "inheritsFrom": FORGE_MC,
        "type": "release",
        "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
        "libraries": [lib("com.example:vlib:2.0", VLIB_PATH, VLIB_BYTES), universal],
    });
    zip_bytes(&[
        ("install_profile.json", profile.to_string().into_bytes()),
        ("version.json", version.to_string().into_bytes()),
        ("data/client.lzma", b"binary patch".to_vec()),
        (&format!("maven/{UNIVERSAL_PATH}"), UNIVERSAL_BYTES.to_vec()),
    ])
}

/// Mounts the installer jar and every library the synthetic Forge installer names.
///
/// The installer is served once only, so a second install proves it came from the cache.
pub async fn mock_forge(server: &MockServer) {
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path_matcher(format!(
            "/net/minecraftforge/forge/{FORGE_MC}-{FORGE_VERSION}/forge-{FORGE_MC}-{FORGE_VERSION}-installer.jar"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(forge_installer_jar(&base)))
        .expect(1)
        .mount(server)
        .await;
    serve(
        server,
        &format!("/libs/{PROC_PATH}"),
        jar_with_main(PROC_MAIN),
    )
    .await;
    serve(server, &format!("/libs/{VLIB_PATH}"), VLIB_BYTES.to_vec()).await;
}

/// A [`ProcessRunner`] that records its calls and writes the file the processor promises.
///
/// It never starts a JVM, so a test can install Forge without a java runtime on the machine.
#[derive(Default)]
pub struct FakeRunner {
    /// Argument list of every call, in order.
    pub calls: Mutex<Vec<Vec<String>>>,
}

#[async_trait::async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(
        &self,
        _java: &Path,
        _classpath: &[PathBuf],
        main: &str,
        args: &[String],
        log: &Path,
    ) -> Result<i32, std::io::Error> {
        assert_eq!(main, PROC_MAIN);
        self.calls.lock().expect("lock").push(args.to_vec());
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(log, format!("ran {main}\n"))?;
        // The real processor writes the patched jar the profile names in `--output`.
        let out = args
            .iter()
            .position(|a| a == "--output")
            .and_then(|i| args.get(i + 1))
            .expect("processor was given an output");
        let out = PathBuf::from(out);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, PATCHED_BYTES)?;
        Ok(0)
    }
}

/// The java the installer would run. Never executed: [`FakeRunner`] stands in for it.
pub fn fake_java() -> JavaInstall {
    JavaInstall {
        path: PathBuf::from("/nonexistent/java"),
        major: 17,
        version: "17.0.0".to_string(),
        vendor: "test".to_string(),
        source: JavaSource::Manual,
    }
}
