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
#[derive(Debug, Clone, PartialEq)]
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
    /// The merged version JSON the plan was built from. A launch reads arguments and
    /// logging config straight off it.
    pub resolved: VersionJson,
    /// JVM entry point from the version JSON, when it names one.
    pub main_class: Option<String>,
    /// Mojang runtime component the version JSON asks for, when it names one.
    pub java_component: Option<String>,
    /// Asset directory name the game is launched with: `assets`, falling back to the asset
    /// index id.
    pub assets_id: Option<String>,
    /// Cached log4j configuration file, set by [`install_resolved`] once it is downloaded.
    /// [`plan_install`] leaves it `None` because it does no I/O.
    pub log_config: Option<PathBuf>,
}

/// Builds the download plan for one resolved version. Does no I/O.
///
/// `override_all_library_urls` replaces the base of *every* library and natives URL, including
/// the ones a `downloads` block publishes. It exists so a test can serve a whole version from
/// one mock server; production callers pass `None`.
pub fn plan_install(
    v: &VersionJson,
    root: &crate::paths::Root,
    rules: &RuleContext,
    override_all_library_urls: Option<&str>,
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
        let native = natives_spec(lib, root, rules, override_all_library_urls)?;
        // A pre-1.13 `*-platform` library ships only natives: it has a `natives` map and no
        // artifact, so there is no base jar to fetch or to put on the classpath.
        let natives_only = native.is_some()
            && lib
                .downloads
                .as_ref()
                .and_then(|d| d.artifact.as_ref())
                .is_none();
        if !natives_only {
            let (spec, dest) = library_spec(lib, root, override_all_library_urls)?;
            classpath.push(dest);
            specs.push(spec);
        }
        if let Some((spec, dest)) = native {
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
        asset_index_id: asset_index_id.clone(),
        java_major: v.java_version.as_ref().map_or(8, |j| j.major_version),
        natives_dir: root.natives_dir(&v.id),
        main_class: v.main_class.clone(),
        java_component: v.java_version.as_ref().map(|j| j.component.clone()),
        assets_id: v.assets.clone().or(Some(asset_index_id)),
        log_config: None,
        resolved: v.clone(),
    })
}

/// Builds the spec and cache path for a library's main jar.
pub(crate) fn library_spec(
    lib: &Library,
    root: &crate::paths::Root,
    base_override: Option<&str>,
) -> Result<(DownloadSpec, PathBuf), Error> {
    let coord = MavenCoord::parse(&lib.name)?;
    let artifact = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref());
    let path = artifact
        .and_then(|a| a.path.clone())
        .unwrap_or_else(|| coord.path());
    let dest = crate::paths::safe_join(&root.libraries_dir(), &path)?;
    // A loader profile can publish an artifact with an empty URL; fall back to the repository.
    let url = match (artifact.filter(|a| !a.url.trim().is_empty()), base_override) {
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
    let dest = crate::paths::safe_join(&root.libraries_dir(), &path)?;
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
///
/// `override_all_library_urls` replaces the base of every library URL, as in [`plan_install`],
/// and is meant for tests. `resources_base` replaces the asset object base.
#[tracing::instrument(skip(m, dl))]
pub async fn install_version_with(
    m: &Mojang,
    dl: &DownloadCtx<'_>,
    id: &str,
    override_all_library_urls: Option<&str>,
    resources_base: &str,
) -> Result<InstallPlan, Error> {
    let resolved = m.resolve_auto(m.version(id).await?)?;
    install_resolved(m, dl, resolved, override_all_library_urls, resources_base).await
}

/// Installs an already-resolved version: libraries, log config, assets, and natives.
///
/// Callers that resolved the version themselves — a loader install knows whether to keep both
/// copies of a library — use this instead of [`install_version_with`], which resolves first.
/// `_m` is taken for symmetry with [`install_version_with`] and for the metadata fetches this
/// step will grow; nothing here needs it yet.
#[tracing::instrument(skip(_m, dl, resolved), fields(id = %resolved.id))]
pub async fn install_resolved(
    _m: &Mojang,
    dl: &DownloadCtx<'_>,
    resolved: VersionJson,
    override_all_library_urls: Option<&str>,
    resources_base: &str,
) -> Result<InstallPlan, Error> {
    let rules = RuleContext::current();
    let mut plan = plan_install(&resolved, dl.root, &rules, override_all_library_urls)?;
    download_all(dl, plan.specs.clone()).await?;

    if let Some((spec, dest)) = log_config_spec(&resolved, dl.root) {
        download_all(dl, vec![spec]).await?;
        plan.log_config = Some(dest);
    }

    let index = fetch_asset_index(dl, &resolved, &plan.asset_index_id).await?;
    download_all(dl, asset_specs(&index, dl.root, resources_base)).await?;

    extract_natives(plan.natives.clone(), plan.natives_dir.clone()).await?;
    Ok(plan)
}

/// Builds the download spec and cache path for a version's log4j configuration file.
///
/// Returns `None` when the version publishes no client logging block, or when its file id is
/// not a plain file name, which would otherwise steer the write out of the cache.
pub(crate) fn log_config_spec(
    v: &VersionJson,
    root: &crate::paths::Root,
) -> Option<(DownloadSpec, PathBuf)> {
    let client = v.logging.as_ref()?.client.as_ref()?;
    let dest = crate::paths::safe_join(&root.assets_dir().join("log_configs"), &client.file.id)
        .ok()
        .filter(|p| p.parent() == Some(root.assets_dir().join("log_configs").as_path()))?;
    Some((
        DownloadSpec {
            url: client.file.url.clone(),
            sha1: Some(client.file.sha1.clone()),
            size: Some(client.file.size),
            dest: dest.clone(),
            label: format!("log config {}", client.file.id),
        },
        dest,
    ))
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
    fn a_version_with_logging_gets_a_log_config_spec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = parse(V1_20_1);
        let (spec, dest) = log_config_spec(&v, &root).expect("1.20.1 publishes a logging block");
        assert_eq!(
            dest,
            root.assets_dir()
                .join("log_configs")
                .join("client-1.12.xml")
        );
        assert_eq!(spec.dest, dest);
        assert_eq!(
            spec.sha1.as_deref(),
            Some("bd65e7d2e3c237be76cfbef4c2405033d7f91521")
        );
        assert_eq!(spec.size, Some(888));
        assert!(spec.url.ends_with("client-1.12.xml"));
    }

    #[test]
    fn a_version_without_logging_gets_no_log_config_spec() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let mut v = parse(V1_20_1);
        v.logging = None;
        assert!(log_config_spec(&v, &root).is_none());
    }

    #[test]
    fn a_log_config_id_that_escapes_the_cache_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let mut v = parse(V1_20_1);
        if let Some(client) = v.logging.as_mut().and_then(|l| l.client.as_mut()) {
            client.file.id = "../../escaped.xml".to_string();
        }
        assert!(log_config_spec(&v, &root).is_none());
    }

    #[test]
    fn a_plan_starts_with_no_log_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = parse(V1_20_1);
        let plan = plan_install(&v, &root, &linux_ctx(), None).expect("plans");
        assert_eq!(plan.log_config, None);
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

        // The `*-platform` libraries ship only natives: no base jar anywhere in the plan.
        // `lwjgl-platform` 2.9.4 does publish an artifact in this fixture, so it keeps its
        // base jar; `jinput-platform` publishes classifiers only.
        let name = "jinput-platform-2.0.5.jar";
        assert!(
            !plan
                .classpath
                .iter()
                .any(|p| p.to_string_lossy().ends_with(name)),
            "{name} is on the classpath"
        );
        assert!(
            !plan
                .specs
                .iter()
                .any(|s| s.dest.to_string_lossy().ends_with(name)),
            "{name} is downloaded"
        );
        assert!(
            plan.natives
                .iter()
                .any(|(jar, _)| jar.to_string_lossy().contains("lwjgl-platform")),
            "the lwjgl natives entry survives"
        );
    }

    #[test]
    fn a_windows_context_resolves_the_arch_classifier() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let ctx = RuleContext {
            os_name: "windows",
            os_version: "10.0".to_string(),
            arch: "x86_64",
            features: Default::default(),
        };
        let plan = plan_install(&parse(V1_8_9), &root, &ctx, None).expect("plans");
        assert!(
            plan.natives
                .iter()
                .any(|(jar, _)| jar.to_string_lossy().contains("natives-windows-64")),
            "${{arch}} expands to 64: {:?}",
            plan.natives
        );
        assert!(
            !plan
                .natives
                .iter()
                .any(|(jar, _)| jar.to_string_lossy().contains("${arch}")),
            "no unexpanded placeholder"
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
    fn a_plan_carries_what_a_launch_needs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = parse(V1_20_1);
        let plan = plan_install(&v, &root, &linux_ctx(), None).expect("plans");
        assert_eq!(
            plan.main_class.as_deref(),
            Some("net.minecraft.client.main.Main")
        );
        assert_eq!(plan.java_component.as_deref(), Some("java-runtime-gamma"));
        assert_eq!(plan.assets_id.as_deref(), Some("5"));
        assert_eq!(plan.resolved, v);
    }

    #[test]
    fn assets_id_falls_back_to_the_asset_index_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v = VersionJson {
            assets: None,
            ..synthetic("x")
        };
        let plan = plan_install(&v, &root, &linux_ctx(), None).expect("plans");
        assert_eq!(plan.assets_id.as_deref(), Some("test"));
        assert!(
            plan.java_component.is_none(),
            "synthetic has no javaVersion"
        );
    }

    #[test]
    fn a_library_path_that_escapes_the_cache_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v: VersionJson = serde_json::from_str(
            r#"{
                "id": "escape",
                "downloads": {
                    "client": {
                        "sha1": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "size": 3,
                        "url": "http://mock/client.jar"
                    }
                },
                "assets": "test",
                "libraries": [{
                    "name": "com.example:thing:1.0",
                    "downloads": {
                        "artifact": {
                            "path": "../escape.jar",
                            "sha1": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            "size": 1,
                            "url": "http://mock/escape.jar"
                        }
                    }
                }]
            }"#,
        )
        .expect("parses");
        assert!(matches!(
            plan_install(&v, &root, &linux_ctx(), None),
            Err(Error::Paths(crate::paths::Error::UnsafePath(_)))
        ));
    }

    #[test]
    fn a_natives_classifier_path_that_escapes_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let v: VersionJson = serde_json::from_str(
            r#"{
                "id": "escape-natives",
                "downloads": {
                    "client": {
                        "sha1": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "size": 3,
                        "url": "http://mock/client.jar"
                    }
                },
                "assets": "test",
                "libraries": [{
                    "name": "com.example:thing:1.0",
                    "natives": { "linux": "natives-linux" },
                    "downloads": {
                        "classifiers": {
                            "natives-linux": {
                                "path": "../../escape-natives.jar",
                                "sha1": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                "size": 1,
                                "url": "http://mock/n.jar"
                            }
                        }
                    }
                }]
            }"#,
        )
        .expect("parses");
        assert!(matches!(
            plan_install(&v, &root, &linux_ctx(), None),
            Err(Error::Paths(crate::paths::Error::UnsafePath(_)))
        ));
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
