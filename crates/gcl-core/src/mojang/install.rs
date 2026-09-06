//! Plans and performs a vanilla version install: client jar, libraries, natives, assets.

use std::path::{Path, PathBuf};

use super::assets::{AssetIndex, RESOURCES_BASE, asset_specs};
use super::rules::{RuleContext, rules_allow};
use super::{Error, Mojang, VersionJson, write_atomic};
use crate::download::hash::sha1_hex;
use crate::download::{DownloadCtx, DownloadSpec, download_all};
use crate::mojang::version::{Artifact, Library, MavenCoord};

/// Default maven repository for libraries without a `downloads` block.
pub const LIBRARIES_BASE: &str = "https://libraries.minecraft.net/";

/// Everything an install needs, and everything a launch needs afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    /// Path of the client jar in the cache.
    pub client_jar: PathBuf,
    /// Library jars, in version-JSON order, that belong on the classpath.
    pub classpath: Vec<PathBuf>,
    /// Old-style natives jars with the archive path prefixes to skip when extracting.
    pub natives: Vec<(PathBuf, Vec<String>)>,
    /// Every file the install must fetch before the game can start.
    pub specs: Vec<DownloadSpec>,
    /// Asset index id, which is also the index's cache file name.
    pub asset_index_id: String,
    /// Major Java version this version asks for.
    pub java_major: u32,
    /// Directory old-style natives are extracted into.
    pub natives_dir: PathBuf,
}

/// Builds the download plan for one resolved version. Does no I/O.
pub fn plan_install(
    v: &VersionJson,
    root: &crate::paths::Root,
    rules: &RuleContext,
    libraries_base_override: Option<&str>,
) -> Result<InstallPlan, Error> {
    let client = v
        .downloads
        .as_ref()
        .map(|d| &d.client)
        .ok_or_else(|| Error::MissingField {
            version: v.id.clone(),
            field: "downloads.client",
        })?;
    let client_jar = root
        .versions_dir()
        .join(&v.id)
        .join(format!("{}.jar", v.id));
    let mut specs = vec![DownloadSpec {
        url: client.url.clone(),
        sha1: Some(client.sha1.clone()),
        size: Some(client.size),
        dest: client_jar.clone(),
        label: format!("client {}", v.id),
    }];

    let mut classpath = Vec::new();
    let mut natives = Vec::new();
    for lib in &v.libraries {
        if !rules_allow(&lib.rules, rules) {
            continue;
        }
        let (spec, dest) = library_spec(lib, root, libraries_base_override)?;
        classpath.push(dest);
        specs.push(spec);
        if let Some((spec, dest)) = natives_spec(lib, root, rules, libraries_base_override)? {
            let exclude = lib
                .extract
                .as_ref()
                .map(|e| e.exclude.clone())
                .unwrap_or_default();
            specs.push(spec);
            natives.push((dest, exclude));
        }
    }

    let asset_index_id = v
        .asset_index
        .as_ref()
        .map(|a| a.id.clone())
        .or_else(|| v.assets.clone())
        .ok_or_else(|| Error::MissingField {
            version: v.id.clone(),
            field: "assetIndex",
        })?;

    Ok(InstallPlan {
        client_jar,
        classpath,
        natives,
        specs,
        asset_index_id,
        java_major: v.java_version.as_ref().map_or(8, |j| j.major_version),
        natives_dir: root.natives_dir(&v.id),
    })
}

/// Builds the spec and cache path for a library's main jar.
fn library_spec(
    lib: &Library,
    root: &crate::paths::Root,
    base_override: Option<&str>,
) -> Result<(DownloadSpec, PathBuf), Error> {
    let coord = MavenCoord::parse(&lib.name)?;
    let artifact = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref());
    let path = artifact
        .and_then(|a| a.path.clone())
        .unwrap_or_else(|| coord.path());
    let dest = root.libraries_dir().join(&path);
    let url = match (artifact, base_override) {
        (Some(a), None) => a.url.clone(),
        (_, base) => join_url(base.or(lib.url.as_deref()).unwrap_or(LIBRARIES_BASE), &path),
    };
    Ok((
        DownloadSpec {
            url,
            sha1: artifact
                .map(|a| a.sha1.clone())
                .or_else(|| lib.sha1.clone()),
            size: artifact.map(|a| a.size).or(lib.size),
            dest: dest.clone(),
            label: format!("library {}", lib.name),
        },
        dest,
    ))
}

/// Builds the spec and cache path for an old-style natives classifier, when there is one.
fn natives_spec(
    lib: &Library,
    root: &crate::paths::Root,
    rules: &RuleContext,
    base_override: Option<&str>,
) -> Result<Option<(DownloadSpec, PathBuf)>, Error> {
    let Some(classifier) = lib
        .natives
        .as_ref()
        .and_then(|m| m.get(rules.os_name))
        .map(|c| expand_arch(c, rules.arch))
    else {
        return Ok(None);
    };
    let artifact: Option<&Artifact> = lib
        .downloads
        .as_ref()
        .and_then(|d| d.classifiers.as_ref())
        .and_then(|c| c.get(&classifier));
    let Some(artifact) = artifact else {
        return Ok(None);
    };
    let mut coord = MavenCoord::parse(&lib.name)?;
    coord.classifier = Some(classifier.clone());
    let path = artifact.path.clone().unwrap_or_else(|| coord.path());
    let dest = root.libraries_dir().join(&path);
    let url = match base_override {
        None => artifact.url.clone(),
        Some(base) => join_url(base, &path),
    };
    Ok(Some((
        DownloadSpec {
            url,
            sha1: Some(artifact.sha1.clone()),
            size: Some(artifact.size),
            dest: dest.clone(),
            label: format!("natives {} {classifier}", lib.name),
        },
        dest,
    )))
}

/// Substitutes `${arch}` in an old natives classifier with 32 or 64.
fn expand_arch(classifier: &str, arch: &str) -> String {
    let bits = if arch == "x86" { "32" } else { "64" };
    classifier.replace("${arch}", bits)
}

/// Joins a repository base URL with a repository-relative path.
fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Installs a vanilla version from the production Mojang endpoints.
pub async fn install_version(
    m: &Mojang,
    dl: &DownloadCtx<'_>,
    id: &str,
) -> Result<InstallPlan, Error> {
    install_version_with(m, dl, id, None, RESOURCES_BASE).await
}

/// Installs a version, letting callers and tests redirect the library and asset bases.
#[tracing::instrument(skip(m, dl))]
pub async fn install_version_with(
    m: &Mojang,
    dl: &DownloadCtx<'_>,
    id: &str,
    libraries_base_override: Option<&str>,
    resources_base: &str,
) -> Result<InstallPlan, Error> {
    let version = m.resolve(m.version(id).await?)?;
    let rules = RuleContext::current();
    let plan = plan_install(&version, dl.root, &rules, libraries_base_override)?;
    download_all(dl, plan.specs.clone()).await?;

    let index = fetch_asset_index(dl, &version, &plan.asset_index_id).await?;
    download_all(dl, asset_specs(&index, dl.root, resources_base)).await?;

    extract_natives(plan.natives.clone(), plan.natives_dir.clone()).await?;
    Ok(plan)
}

/// Fetches and caches the asset index, verifying the sha1 the version JSON publishes.
async fn fetch_asset_index(
    dl: &DownloadCtx<'_>,
    version: &VersionJson,
    index_id: &str,
) -> Result<AssetIndex, Error> {
    let reference = version
        .asset_index
        .as_ref()
        .ok_or_else(|| Error::MissingField {
            version: version.id.clone(),
            field: "assetIndex",
        })?;
    let path = dl
        .root
        .assets_dir()
        .join("indexes")
        .join(format!("{index_id}.json"));
    if let Ok(bytes) = std::fs::read(&path)
        && sha1_hex(&bytes) == reference.sha1
    {
        return super::parse_json(
            &path.display().to_string(),
            &String::from_utf8_lossy(&bytes),
        );
    }

    let bytes = dl.http.get_bytes(&reference.url).await?;
    let actual = sha1_hex(&bytes);
    if actual != reference.sha1 {
        return Err(Error::Sha1Mismatch {
            id: index_id.to_string(),
            expected: reference.sha1.clone(),
            actual,
        });
    }
    let index = super::parse_json(&reference.url, &String::from_utf8_lossy(&bytes))?;
    write_atomic(&path, &bytes)?;
    Ok(index)
}

/// Unpacks every natives jar into `dir`, skipping directories and excluded prefixes.
async fn extract_natives(natives: Vec<(PathBuf, Vec<String>)>, dir: PathBuf) -> Result<(), Error> {
    if natives.is_empty() {
        return Ok(());
    }
    tokio::task::spawn_blocking(move || {
        for (jar, exclude) in &natives {
            extract_one(jar, exclude, &dir)?;
        }
        Ok(())
    })
    .await
    .map_err(|source| Error::Io {
        path: PathBuf::from("natives"),
        source: std::io::Error::other(source),
    })?
}

/// Unpacks one natives jar. Blocking; call it inside `spawn_blocking`.
fn extract_one(jar: &Path, exclude: &[String], dir: &Path) -> Result<(), Error> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source: std::io::Error| Error::Io { path, source }
    };
    let file = std::fs::File::open(jar).map_err(io(jar))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|source| Error::Zip {
        path: jar.to_path_buf(),
        source,
    })?;
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|source| Error::Zip {
            path: jar.to_path_buf(),
            source,
        })?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name_str = name.to_string_lossy().replace('\\', "/");
        if exclude.iter().any(|p| name_str.starts_with(p.as_str())) {
            continue;
        }
        let dest = dir.join(&name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let mut out = std::fs::File::create(&dest).map_err(io(&dest))?;
        std::io::copy(&mut entry, &mut out).map_err(io(&dest))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Root;

    const V1_20_1: &str = include_str!("../../../../tests/fixtures/mojang/1.20.1.json");
    const V1_8_9: &str = include_str!("../../../../tests/fixtures/mojang/1.8.9.json");

    fn linux_ctx() -> RuleContext {
        RuleContext {
            os_name: "linux",
            os_version: "6.0.0".to_string(),
            arch: "x86_64",
            features: Default::default(),
        }
    }

    fn parse(text: &str) -> VersionJson {
        serde_json::from_str(text).expect("fixture parses")
    }

    #[test]
    fn a_modern_version_plans_a_classpath_and_no_natives() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = parse(V1_20_1);
        let plan = plan_install(&v, &root, &linux_ctx(), None).expect("plans");

        assert_eq!(plan.java_major, 17);
        assert_eq!(
            plan.client_jar,
            root.versions_dir().join("1.20.1").join("1.20.1.jar")
        );
        assert!(plan.natives.is_empty(), "{:?}", plan.natives);
        assert!(!plan.classpath.is_empty());
        for jar in &plan.classpath {
            assert!(jar.starts_with(root.libraries_dir()), "{}", jar.display());
        }
        assert!(
            plan.classpath
                .iter()
                .any(|p| p.to_string_lossy().contains("lwjgl")
                    && p.to_string_lossy().contains("natives-linux")),
            "modern natives ride the classpath"
        );
        assert_eq!(plan.asset_index_id, "5");
        assert_eq!(plan.natives_dir, root.natives_dir("1.20.1"));
        assert_eq!(plan.specs.len(), plan.classpath.len() + 1);
    }

    #[test]
    fn a_legacy_version_plans_natives_with_excludes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = parse(V1_8_9);
        let plan = plan_install(&v, &root, &linux_ctx(), None).expect("plans");

        assert_eq!(plan.java_major, 8, "1.8.9 publishes no javaVersion");
        let (jar, exclude) = plan
            .natives
            .iter()
            .find(|(jar, _)| jar.to_string_lossy().contains("natives-linux"))
            .expect("an lwjgl natives-linux entry");
        assert!(jar.starts_with(root.libraries_dir()), "{}", jar.display());
        assert!(exclude.contains(&"META-INF/".to_string()), "{exclude:?}");
        assert!(
            plan.specs
                .iter()
                .any(|s| s.dest == *jar && s.sha1.is_some()),
            "the natives jar is downloaded"
        );
    }

    #[test]
    fn a_library_without_downloads_uses_the_base_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = VersionJson {
            libraries: vec![Library {
                name: "com.example:thing:1.0".to_string(),
                downloads: None,
                url: None,
                sha1: Some("a".repeat(40)),
                size: Some(7),
                rules: Vec::new(),
                natives: None,
                extract: None,
            }],
            ..synthetic("x")
        };
        let plan = plan_install(&v, &root, &linux_ctx(), None).expect("plans");
        let spec = plan
            .specs
            .iter()
            .find(|s| s.label.contains("com.example"))
            .expect("library spec");
        assert_eq!(
            spec.url,
            "https://libraries.minecraft.net/com/example/thing/1.0/thing-1.0.jar"
        );
        assert_eq!(spec.size, Some(7));

        let plan = plan_install(&v, &root, &linux_ctx(), Some("http://mock/m2")).expect("plans");
        let spec = plan
            .specs
            .iter()
            .find(|s| s.label.contains("com.example"))
            .expect("library spec");
        assert_eq!(
            spec.url,
            "http://mock/m2/com/example/thing/1.0/thing-1.0.jar"
        );
    }

    #[test]
    fn a_version_without_downloads_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = VersionJson {
            downloads: None,
            ..synthetic("x")
        };
        assert!(matches!(
            plan_install(&v, &root, &linux_ctx(), None),
            Err(Error::MissingField {
                field: "downloads.client",
                ..
            })
        ));
    }

    /// A minimal version JSON used as a base for the cases above.
    fn synthetic(id: &str) -> VersionJson {
        use crate::mojang::version::{AssetIndexRef, Downloads};
        VersionJson {
            id: id.to_string(),
            inherits_from: None,
            main_class: Some("net.minecraft.client.main.Main".to_string()),
            arguments: None,
            minecraft_arguments: None,
            libraries: Vec::new(),
            downloads: Some(Downloads {
                client: Artifact {
                    path: None,
                    sha1: "b".repeat(40),
                    size: 3,
                    url: "http://mock/client.jar".to_string(),
                },
                server: None,
                client_mappings: None,
            }),
            asset_index: Some(AssetIndexRef {
                id: "test".to_string(),
                sha1: "c".repeat(40),
                size: 2,
                total_size: None,
                url: "http://mock/index.json".to_string(),
            }),
            assets: Some("test".to_string()),
            java_version: None,
            logging: None,
            kind: None,
            release_time: None,
        }
    }
}

#[cfg(test)]
mod e2e_tests {
    use super::*;
    use crate::events::null_sink;
    use crate::http::HttpClient;
    use crate::mojang::MANIFEST_PATH;
    use crate::mojang::version::{
        Artifact, AssetIndexRef, Downloads, Extract, Library, LibraryDownloads,
    };
    use crate::paths::Root;
    use std::collections::BTreeMap;
    use std::io::Write;
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
}
