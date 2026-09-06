//! Installs a Mojang Java runtime component into the launcher cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{Error, JAVA_BIN, JavaInstall, JavaSource, detect::parse_java_version};
use crate::download::{DownloadCtx, DownloadSpec, download_all};
use crate::http::HttpClient;

/// The runtime index Mojang's launcher uses.
pub const RUNTIME_MANIFEST: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// The whole runtime index: platform key, then component name, then builds.
type AllRuntimes = BTreeMap<String, BTreeMap<String, Vec<RuntimeBuild>>>;

/// One published build of a runtime component.
#[derive(Debug, Clone, Deserialize)]
struct RuntimeBuild {
    manifest: ManifestRef,
    #[serde(default)]
    version: Option<BuildVersion>,
}

/// Pointer to a build's file manifest.
#[derive(Debug, Clone, Deserialize)]
struct ManifestRef {
    url: String,
}

/// The build's own version, used for the returned [`JavaInstall`].
#[derive(Debug, Clone, Deserialize)]
struct BuildVersion {
    name: String,
}

/// A build's file manifest: every path the runtime needs.
#[derive(Debug, Clone, Deserialize)]
struct FileManifest {
    #[serde(default)]
    files: BTreeMap<String, FileEntry>,
}

/// One entry of a file manifest: a file, a directory, or a symlink.
#[derive(Debug, Clone, Deserialize)]
struct FileEntry {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    downloads: Option<FileDownloads>,
    #[serde(default)]
    executable: bool,
    #[serde(default)]
    target: Option<String>,
}

/// The download variants of a file entry. Only `raw` is used.
#[derive(Debug, Clone, Deserialize)]
struct FileDownloads {
    raw: RawDownload,
}

/// The uncompressed download of a runtime file.
#[derive(Debug, Clone, Deserialize)]
struct RawDownload {
    sha1: String,
    size: u64,
    url: String,
}

/// The Mojang platform key for the target this binary was built for.
pub fn platform_key() -> Option<&'static str> {
    let arch = std::env::consts::ARCH;
    if cfg!(target_os = "linux") {
        match arch {
            "x86_64" => Some("linux"),
            "x86" => Some("linux-i386"),
            _ => None,
        }
    } else if cfg!(target_os = "windows") {
        match arch {
            "x86_64" => Some("windows-x64"),
            "x86" => Some("windows-x86"),
            "aarch64" => Some("windows-arm64"),
            _ => None,
        }
    } else if cfg!(target_os = "macos") {
        match arch {
            "x86_64" => Some("mac-os"),
            "aarch64" => Some("mac-os-arm64"),
            _ => None,
        }
    } else {
        None
    }
}

/// The Mojang runtime component that serves a major Java version.
pub fn component_for_major(major: u32) -> &'static str {
    match major {
        8 => "jre-legacy",
        16 => "java-runtime-alpha",
        17 => "java-runtime-gamma",
        _ => "java-runtime-delta",
    }
}

/// Downloads a runtime component for this platform and returns its `java` binary.
#[tracing::instrument(skip(http, dl))]
pub async fn install_runtime(
    http: &HttpClient,
    dl: &DownloadCtx<'_>,
    manifest_url: &str,
    component: &str,
) -> Result<JavaInstall, Error> {
    let platform = platform_key().ok_or(Error::UnsupportedPlatform)?;
    let all: AllRuntimes = http.get_json(manifest_url).await?;
    let build = all
        .get(platform)
        .and_then(|components| components.get(component))
        .and_then(|builds| builds.first())
        .ok_or_else(|| Error::NoRuntimeForPlatform {
            platform: platform.to_string(),
            component: component.to_string(),
        })?;

    let files: FileManifest = http.get_json(&build.manifest.url).await?;
    let base = dl.root.runtimes_dir().join(component).join(platform);
    let (specs, executables) = lay_out(&files, &base)?;
    download_all(dl, specs).await?;
    for path in &executables {
        make_executable(path)?;
    }

    let java = java_path(&base, platform);
    if !java.is_file() {
        return Err(Error::Probe {
            path: java,
            reason: "the runtime manifest published no java binary".to_string(),
        });
    }
    make_executable(&java)?;
    let version = build
        .version
        .as_ref()
        .map(|v| v.name.clone())
        .unwrap_or_default();
    let major = parse_java_version(&version)
        .map(|(major, _)| major)
        .unwrap_or_default();
    Ok(JavaInstall {
        path: java,
        major,
        version,
        vendor: "Mojang".to_string(),
        source: JavaSource::Mojang,
    })
}

/// Creates the directories and links a manifest names, and returns the files to fetch.
fn lay_out(files: &FileManifest, base: &Path) -> Result<(Vec<DownloadSpec>, Vec<PathBuf>), Error> {
    let mut specs = Vec::new();
    let mut executables = Vec::new();
    for (path, entry) in &files.files {
        let dest = base.join(path);
        match entry.kind.as_str() {
            "directory" => create_dir(&dest)?,
            "file" => {
                let Some(raw) = entry.downloads.as_ref().map(|d| &d.raw) else {
                    continue;
                };
                if let Some(parent) = dest.parent() {
                    create_dir(parent)?;
                }
                specs.push(DownloadSpec {
                    url: raw.url.clone(),
                    sha1: Some(raw.sha1.clone()),
                    size: Some(raw.size),
                    dest: dest.clone(),
                    label: format!("runtime {path}"),
                });
                if entry.executable {
                    executables.push(dest);
                }
            }
            "link" => link(entry.target.as_deref(), &dest)?,
            other => tracing::warn!(kind = other, path, "unknown runtime manifest entry"),
        }
    }
    Ok((specs, executables))
}

/// The `java` binary inside an installed runtime, allowing for the macOS bundle layout.
fn java_path(base: &Path, platform: &str) -> PathBuf {
    if platform.starts_with("mac-os") {
        base.join("jre.bundle/Contents/Home/bin").join(JAVA_BIN)
    } else {
        base.join("bin").join(JAVA_BIN)
    }
    .components()
    .collect()
}

/// Creates a directory and every parent, reporting the path on failure.
fn create_dir(path: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Creates a symlink on unix. Windows runtimes have no link entries, so this is a no-op there.
#[cfg(unix)]
fn link(target: Option<&str>, dest: &Path) -> Result<(), Error> {
    let Some(target) = target else {
        return Ok(());
    };
    if let Some(parent) = dest.parent() {
        create_dir(parent)?;
    }
    if dest.symlink_metadata().is_ok() {
        std::fs::remove_file(dest).map_err(|source| Error::Io {
            path: dest.to_path_buf(),
            source,
        })?;
    }
    std::os::unix::fs::symlink(target, dest).map_err(|source| Error::Io {
        path: dest.to_path_buf(),
        source,
    })
}

/// Windows has no symlink entries in Mojang's runtime manifests; skip them.
#[cfg(not(unix))]
fn link(_target: Option<&str>, _dest: &Path) -> Result<(), Error> {
    Ok(())
}

/// Sets the owner, group, and other execute bits on unix. A no-op elsewhere.
#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    let io = |source: std::io::Error| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut perms = std::fs::metadata(path).map_err(io)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    std::fs::set_permissions(path, perms).map_err(io)
}

/// Windows decides executability by extension, so there is nothing to set.
#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::null_sink;
    use crate::paths::Root;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ALL_JSON: &str = include_str!("../../../../tests/fixtures/mojang/java_runtime_all.json");

    #[test]
    fn components_map_to_major_versions() {
        assert_eq!(component_for_major(8), "jre-legacy");
        assert_eq!(component_for_major(16), "java-runtime-alpha");
        assert_eq!(component_for_major(17), "java-runtime-gamma");
        assert_eq!(component_for_major(21), "java-runtime-delta");
        assert_eq!(component_for_major(25), "java-runtime-delta");
    }

    #[test]
    fn the_platform_key_is_one_mojang_publishes() {
        if let Some(key) = platform_key() {
            assert!(
                [
                    "linux",
                    "linux-i386",
                    "windows-x64",
                    "windows-x86",
                    "windows-arm64",
                    "mac-os",
                    "mac-os-arm64"
                ]
                .contains(&key),
                "{key}"
            );
        }
    }

    #[test]
    fn the_mac_java_path_uses_the_bundle_layout() {
        let base = Path::new("/cache/runtimes/java-runtime-gamma/mac-os");
        assert!(
            java_path(base, "mac-os-arm64")
                .to_string_lossy()
                .ends_with("Contents/Home/bin/java")
        );
        assert!(
            java_path(base, "linux")
                .to_string_lossy()
                .ends_with(&format!("bin/{JAVA_BIN}"))
        );
    }

    async fn serve(server: &MockServer, at: &str, body: String) {
        Mock::given(method("GET"))
            .and(path_matcher(at.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn install_runtime_lays_out_a_component() {
        let Some(platform) = platform_key() else {
            return;
        };
        let server = MockServer::start().await;
        let base = server.uri();
        let all = ALL_JSON.replace("https://piston-meta.mojang.com", &base);
        let parsed: serde_json::Value = serde_json::from_str(&all).expect("fixture parses");
        let manifest_url = parsed[platform]["java-runtime-gamma"][0]["manifest"]["url"]
            .as_str()
            .expect("a gamma build for this platform")
            .to_string();
        let manifest_path = manifest_url
            .strip_prefix(&base)
            .expect("rewritten to the mock")
            .to_string();

        let java_body = b"#!/bin/sh\necho java\n".to_vec();
        let lib_body = b"native library bytes".to_vec();
        let sha1 = crate::download::hash::sha1_hex;
        let prefix = if platform.starts_with("mac-os") {
            "jre.bundle/Contents/Home/"
        } else {
            ""
        };
        let files = serde_json::json!({
            "files": {
                format!("{prefix}bin"): { "type": "directory" },
                format!("{prefix}bin/{JAVA_BIN}"): {
                    "type": "file",
                    "executable": true,
                    "downloads": { "raw": {
                        "sha1": sha1(&java_body), "size": java_body.len(),
                        "url": format!("{base}/files/java"),
                    }},
                },
                format!("{prefix}lib/libjvm.so"): {
                    "type": "file",
                    "executable": false,
                    "downloads": { "raw": {
                        "sha1": sha1(&lib_body), "size": lib_body.len(),
                        "url": format!("{base}/files/libjvm"),
                    }},
                },
                format!("{prefix}lib/alias.so"): { "type": "link", "target": "libjvm.so" },
            }
        })
        .to_string();

        serve(&server, "/all.json", all).await;
        serve(&server, &manifest_path, files).await;
        serve(
            &server,
            "/files/java",
            String::from_utf8_lossy(&java_body).into(),
        )
        .await;
        serve(
            &server,
            "/files/libjvm",
            String::from_utf8_lossy(&lib_body).into(),
        )
        .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        let http = HttpClient::new().expect("client builds");
        let sink = null_sink();
        let cancel = CancellationToken::new();
        let dl = DownloadCtx {
            http: &http,
            root: &root,
            sink: &sink,
            cancel: &cancel,
            parallel: 4,
        };

        let install = install_runtime(
            &http,
            &dl,
            &format!("{base}/all.json"),
            "java-runtime-gamma",
        )
        .await
        .expect("installs");

        assert!(
            install
                .path
                .to_string_lossy()
                .ends_with(&format!("bin/{JAVA_BIN}")),
            "{}",
            install.path.display()
        );
        assert!(install.path.is_file());
        assert_eq!(install.major, 17);
        assert_eq!(install.source, JavaSource::Mojang);
        assert_eq!(install.vendor, "Mojang");
        let home = root
            .runtimes_dir()
            .join("java-runtime-gamma")
            .join(platform);
        assert!(home.join(format!("{prefix}lib/libjvm.so")).is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&install.path)
                .expect("stat java")
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "executable bit set");
            assert!(
                home.join(format!("{prefix}lib/alias.so"))
                    .symlink_metadata()
                    .expect("link entry created")
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[tokio::test]
    async fn a_missing_component_is_reported() {
        let Some(_) = platform_key() else { return };
        let server = MockServer::start().await;
        let base = server.uri();
        serve(&server, "/all.json", ALL_JSON.to_string()).await;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let http = HttpClient::new().expect("client builds");
        let sink = null_sink();
        let cancel = CancellationToken::new();
        let dl = DownloadCtx {
            http: &http,
            root: &root,
            sink: &sink,
            cancel: &cancel,
            parallel: 2,
        };
        let err = install_runtime(
            &http,
            &dl,
            &format!("{base}/all.json"),
            "java-runtime-omega",
        )
        .await
        .expect_err("no such component");
        assert!(matches!(err, Error::NoRuntimeForPlatform { .. }), "{err:?}");
    }
}
