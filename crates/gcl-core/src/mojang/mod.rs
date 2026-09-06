//! Mojang piston-meta client: version manifest, version JSON, rules, and inheritance.
//!
//! The client takes a base URL so tests can point it at a mock server. Both the manifest
//! and every version JSON are cached under `cache/versions/`.

pub mod args;
pub mod manifest;
pub mod rules;
pub mod version;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::download::hash::sha1_hex;
use crate::http::HttpClient;
use crate::paths::Root;

pub use args::{ArgContext, default_legacy_jvm_args, expand_arguments, expand_legacy};
pub use manifest::{Latest, ManifestEntry, VersionManifest, VersionType};
pub use rules::{Action, OsRule, Rule, RuleContext, rules_allow};
pub use version::{
    ArgValue, Argument, Arguments, Artifact, AssetIndexRef, Downloads, Extract, JavaVersion,
    Library, LibraryDownloads, MavenCoord, VersionJson,
};

/// Production base URL for Mojang metadata.
pub const PISTON_META: &str = "https://piston-meta.mojang.com";

/// Path of the v2 version manifest below the base URL.
pub const MANIFEST_PATH: &str = "/mc/game/version_manifest_v2.json";

/// Errors from fetching, caching, or parsing Mojang metadata.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A request to piston-meta failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// Reading or writing a cache file failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being read or written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A cached or fetched document was not the JSON we expect.
    #[error("{path}: invalid JSON: {source}")]
    Json {
        /// The file or URL the JSON came from.
        path: String,
        /// The underlying parse error.
        source: serde_json::Error,
    },
    /// The manifest has no entry for the requested version id.
    #[error("unknown version: {0}")]
    UnknownVersion(String),
    /// A library `name` is not a maven coordinate.
    #[error("bad maven coordinate: {0}")]
    BadMavenCoord(String),
    /// An `inheritsFrom` chain points back at a version it already visited.
    #[error("inheritance loop at version {0}")]
    InheritanceLoop(String),
    /// A resolved version is missing a field the launcher needs.
    #[error("version {version} is missing {field}")]
    MissingField {
        /// The version id.
        version: String,
        /// The missing field's name.
        field: &'static str,
    },
    /// A downloaded version JSON did not match the sha1 the manifest published.
    #[error("version {id}: sha1 mismatch, expected {expected}, got {actual}")]
    Sha1Mismatch {
        /// The version id.
        id: String,
        /// The sha1 from the manifest.
        expected: String,
        /// The sha1 of the bytes received.
        actual: String,
    },
}

/// Client for Mojang version metadata, backed by an on-disk cache.
#[derive(Debug, Clone)]
pub struct Mojang {
    http: HttpClient,
    base_url: String,
    root: Root,
}

impl Mojang {
    /// Builds a client against the production piston-meta base URL.
    pub fn new(http: HttpClient, root: Root) -> Self {
        Self::with_base_url(http, root, PISTON_META.to_string())
    }

    /// Builds a client against a given base URL. Tests pass a mock server URI.
    pub fn with_base_url(http: HttpClient, root: Root, base: String) -> Self {
        Mojang {
            http,
            base_url: base.trim_end_matches('/').to_string(),
            root,
        }
    }

    /// Path of the cached manifest JSON.
    fn manifest_file(&self) -> PathBuf {
        self.root.versions_dir().join("manifest.json")
    }

    /// Path of the cached manifest ETag.
    fn etag_file(&self) -> PathBuf {
        self.root.versions_dir().join("manifest.etag")
    }

    /// Path of a cached version JSON.
    fn version_file(&self, id: &str) -> PathBuf {
        self.root.versions_dir().join(format!("{id}.json"))
    }

    /// Fetches the version manifest, using the cached copy on a 304 or a network failure.
    #[tracing::instrument(skip(self))]
    pub async fn manifest(&self) -> Result<VersionManifest, Error> {
        let url = format!("{}{}", self.base_url, MANIFEST_PATH);
        let etag = read_optional(&self.etag_file())?;
        match self
            .http
            .get_text_if_changed(&url, etag.as_deref().map(str::trim))
            .await
        {
            Ok(Some((body, new_etag))) => {
                let parsed = parse_json::<VersionManifest>(&url, &body)?;
                write_atomic(&self.manifest_file(), body.as_bytes())?;
                match new_etag {
                    Some(tag) => write_atomic(&self.etag_file(), tag.as_bytes())?,
                    None => remove_if_present(&self.etag_file())?,
                }
                Ok(parsed)
            }
            Ok(None) => match read_optional(&self.manifest_file())? {
                Some(body) => parse_json(&self.manifest_file().display().to_string(), &body),
                None => {
                    tracing::warn!("server sent 304 but the manifest cache is gone; refetching");
                    let (body, _) = self
                        .http
                        .get_text_if_changed(&url, None)
                        .await?
                        .ok_or_else(|| Error::UnknownVersion("manifest".to_string()))?;
                    let parsed = parse_json::<VersionManifest>(&url, &body)?;
                    write_atomic(&self.manifest_file(), body.as_bytes())?;
                    Ok(parsed)
                }
            },
            Err(err) => match read_optional(&self.manifest_file())? {
                Some(body) => {
                    tracing::warn!(%err, "manifest fetch failed; using the cached copy");
                    parse_json(&self.manifest_file().display().to_string(), &body)
                }
                None => Err(Error::Http(err)),
            },
        }
    }

    /// Returns a version JSON, from the cache when present and from the network otherwise.
    #[tracing::instrument(skip(self))]
    pub async fn version(&self, id: &str) -> Result<VersionJson, Error> {
        if let Some(cached) = self.load_cached_version(id)? {
            return Ok(cached);
        }
        let manifest = self.manifest().await?;
        let entry = manifest
            .entry(id)
            .ok_or_else(|| Error::UnknownVersion(id.to_string()))?;
        let body = self.http.get_bytes(&entry.url).await?;
        let actual = sha1_hex(&body);
        if actual != entry.sha1 {
            return Err(Error::Sha1Mismatch {
                id: id.to_string(),
                expected: entry.sha1.clone(),
                actual,
            });
        }
        let parsed = parse_json::<VersionJson>(&entry.url, &String::from_utf8_lossy(&body))?;
        write_atomic(&self.version_file(id), &body)?;
        Ok(parsed)
    }

    /// Reads a version JSON from the cache. Loader profiles are written there by the installers.
    pub fn load_cached_version(&self, id: &str) -> Result<Option<VersionJson>, Error> {
        let path = self.version_file(id);
        match read_optional(&path)? {
            Some(body) => Ok(Some(parse_json(&path.display().to_string(), &body)?)),
            None => Ok(None),
        }
    }

    /// Follows `inheritsFrom` through the cache and merges the chain into one version.
    pub fn resolve(&self, v: VersionJson) -> Result<VersionJson, Error> {
        let mut seen = HashSet::from([v.id.clone()]);
        let mut ancestors: Vec<VersionJson> = Vec::new();
        let mut next = v.inherits_from.clone();
        while let Some(parent_id) = next {
            if !seen.insert(parent_id.clone()) {
                return Err(Error::InheritanceLoop(parent_id));
            }
            let parent = self
                .load_cached_version(&parent_id)?
                .ok_or_else(|| Error::UnknownVersion(parent_id))?;
            next = parent.inherits_from.clone();
            ancestors.push(parent);
        }
        // Fold the oldest ancestor forward, then the version we started from.
        let mut acc: Option<VersionJson> = None;
        for ancestor in ancestors.into_iter().rev() {
            acc = Some(match acc {
                None => ancestor,
                Some(parent) => merge(parent, ancestor),
            });
        }
        Ok(match acc {
            None => v,
            Some(parent) => merge(parent, v),
        })
    }
}

/// Merges a child profile over its parent version. See the `mojang-meta` skill for the rules.
pub fn merge(parent: VersionJson, child: VersionJson) -> VersionJson {
    let keep_both = {
        let id = child.id.to_lowercase();
        id.contains("forge")
    };
    VersionJson {
        id: child.id,
        inherits_from: None,
        main_class: child.main_class.or(parent.main_class),
        arguments: merge_arguments(parent.arguments, child.arguments),
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        libraries: merge_libraries(parent.libraries, child.libraries, keep_both),
        downloads: child.downloads.or(parent.downloads),
        asset_index: child.asset_index.or(parent.asset_index),
        assets: child.assets.or(parent.assets),
        java_version: child.java_version.or(parent.java_version),
        logging: child.logging.or(parent.logging),
        kind: child.kind.or(parent.kind),
        release_time: child.release_time.or(parent.release_time),
    }
}

/// Appends the child's argument lists to the parent's, keeping order.
fn merge_arguments(parent: Option<Arguments>, child: Option<Arguments>) -> Option<Arguments> {
    match (parent, child) {
        (None, child) => child,
        (parent, None) => parent,
        (Some(mut parent), Some(child)) => {
            parent.game.extend(child.game);
            parent.jvm.extend(child.jvm);
            Some(parent)
        }
    }
}

/// Merges libraries by `group:artifact`. Forge profiles keep both versions on the classpath.
fn merge_libraries(parent: Vec<Library>, child: Vec<Library>, keep_both: bool) -> Vec<Library> {
    let mut out = parent;
    for lib in child {
        let key = MavenCoord::parse(&lib.name).ok().map(|c| c.key());
        let existing = match (keep_both, &key) {
            (false, Some(key)) => out.iter().position(|l| {
                MavenCoord::parse(&l.name).ok().map(|c| c.key()).as_ref() == Some(key)
            }),
            _ => None,
        };
        match existing {
            Some(i) => out[i] = lib,
            None => out.push(lib),
        }
    }
    out
}

/// Reads a file, returning `None` when it does not exist.
fn read_optional(path: &Path) -> Result<Option<String>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Deletes a file, ignoring the case where it is already gone.
fn remove_if_present(path: &Path) -> Result<(), Error> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Writes bytes through a temporary file, so a crash never leaves a half-written cache entry.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Error::Io { path, source }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(io(&tmp))?;
    std::fs::rename(&tmp, path).map_err(io(path))
}

/// Parses JSON, naming the file or URL it came from in the error.
fn parse_json<T: serde::de::DeserializeOwned>(path: &str, body: &str) -> Result<T, Error> {
    serde_json::from_str(body).map_err(|source| Error::Json {
        path: path.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const MANIFEST: &str =
        include_str!("../../../../tests/fixtures/mojang/version_manifest_v2.json");
    const V1_20_1: &str = include_str!("../../../../tests/fixtures/mojang/1.20.1.json");

    /// The manifest fixture with URLs pointed at the mock server and sha1s made true.
    fn rewritten_manifest(base: &str) -> String {
        let mut m: VersionManifest = serde_json::from_str(MANIFEST).expect("fixture parses");
        for entry in &mut m.versions {
            entry.url = entry.url.replace(PISTON_META, base);
            if entry.id == "1.20.1" {
                entry.sha1 = sha1_hex(V1_20_1.as_bytes());
            }
        }
        serde_json::to_string(&m).expect("serializes")
    }

    fn client() -> HttpClient {
        HttpClient::new()
            .expect("client builds")
            .with_backoff(vec![std::time::Duration::ZERO])
    }

    #[tokio::test]
    async fn manifest_caches_the_body_and_the_etag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(MANIFEST_PATH))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"v1\"")
                    .set_body_string(MANIFEST),
            )
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let mojang = Mojang::with_base_url(client(), root.clone(), server.uri());

        let m = mojang.manifest().await.expect("first fetch");
        assert_eq!(m.latest.release, "26.2");
        assert!(root.versions_dir().join("manifest.json").is_file());
        assert_eq!(
            std::fs::read_to_string(root.versions_dir().join("manifest.etag")).expect("etag file"),
            "\"v1\""
        );
        drop(server);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(MANIFEST_PATH))
            .and(header("if-none-match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;
        let mojang = Mojang::with_base_url(client(), root, server.uri());
        let again = mojang
            .manifest()
            .await
            .expect("304 falls back to the cache");
        assert_eq!(again, m);
    }

    #[tokio::test]
    async fn manifest_falls_back_to_the_cache_when_the_network_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        write_atomic(
            &root.versions_dir().join("manifest.json"),
            MANIFEST.as_bytes(),
        )
        .expect("seed cache");

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(MANIFEST_PATH))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let mojang = Mojang::with_base_url(client(), root, server.uri());
        let m = mojang.manifest().await.expect("cache serves the failure");
        assert_eq!(m.latest.release, "26.2");
    }

    #[tokio::test]
    async fn manifest_reports_the_error_with_no_cache() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(MANIFEST_PATH))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let mojang = Mojang::with_base_url(client(), Root::from_path(dir.path()), server.uri());
        assert!(matches!(mojang.manifest().await, Err(Error::Http(_))));
    }

    #[tokio::test]
    async fn version_fetches_once_then_serves_from_the_cache() {
        let server = MockServer::start().await;
        let body = rewritten_manifest(&server.uri());
        Mock::given(method("GET"))
            .and(path(MANIFEST_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/v1/packages/19f5ae58f9c31bd3b0923cb822e99e3162bd62ab/1.20.1.json",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(V1_20_1))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let mojang = Mojang::with_base_url(client(), root.clone(), server.uri());

        let v = mojang.version("1.20.1").await.expect("first fetch");
        assert_eq!(
            v.main_class.as_deref(),
            Some("net.minecraft.client.main.Main")
        );
        assert!(root.versions_dir().join("1.20.1.json").is_file());

        drop(server);
        let offline =
            Mojang::with_base_url(client(), root, "http://127.0.0.1:1/unused".to_string());
        let again = offline
            .version("1.20.1")
            .await
            .expect("cache hit, no network");
        assert_eq!(again, v);
    }

    #[tokio::test]
    async fn version_rejects_a_sha1_mismatch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(MANIFEST_PATH))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(MANIFEST.replace(PISTON_META, &server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/v1/packages/19f5ae58f9c31bd3b0923cb822e99e3162bd62ab/1.20.1.json",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(V1_20_1))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("tempdir");
        let mojang = Mojang::with_base_url(client(), Root::from_path(dir.path()), server.uri());
        assert!(matches!(
            mojang.version("1.20.1").await,
            Err(Error::Sha1Mismatch { .. })
        ));
    }

    #[tokio::test]
    async fn version_reports_an_id_the_manifest_does_not_have() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(MANIFEST_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_string(MANIFEST))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().expect("tempdir");
        let mojang = Mojang::with_base_url(client(), Root::from_path(dir.path()), server.uri());
        assert!(matches!(
            mojang.version("9.9.9").await,
            Err(Error::UnknownVersion(_))
        ));
    }

    #[test]
    fn load_cached_version_is_none_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mojang = Mojang::new(client(), Root::from_path(dir.path()));
        assert!(
            mojang
                .load_cached_version("1.20.1")
                .expect("no error")
                .is_none()
        );
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    const V1_20_1: &str = include_str!("../../../../tests/fixtures/mojang/1.20.1.json");

    fn parent() -> VersionJson {
        serde_json::from_str(V1_20_1).expect("fixture parses")
    }

    /// A loader-style child: a new main class and one library the parent also has.
    fn child(id: &str) -> VersionJson {
        VersionJson {
            id: id.to_string(),
            inherits_from: Some("1.20.1".to_string()),
            main_class: Some("net.fabricmc.loader.impl.launch.knot.KnotClient".to_string()),
            arguments: Some(Arguments {
                game: vec![Argument::Plain("--fabric".to_string())],
                jvm: Vec::new(),
            }),
            minecraft_arguments: None,
            libraries: vec![Library {
                name: "com.google.code.gson:gson:2.11".to_string(),
                downloads: None,
                url: Some("https://maven.fabricmc.net/".to_string()),
                sha1: None,
                size: None,
                rules: Vec::new(),
                natives: None,
                extract: None,
            }],
            downloads: None,
            asset_index: None,
            assets: None,
            java_version: None,
            logging: None,
            kind: None,
            release_time: None,
        }
    }

    fn versions_of(v: &VersionJson, key: &str) -> Vec<String> {
        v.libraries
            .iter()
            .filter_map(|l| MavenCoord::parse(&l.name).ok())
            .filter(|c| c.key() == key)
            .map(|c| c.version)
            .collect()
    }

    #[test]
    fn child_library_replaces_the_parent_version() {
        let p = parent();
        let game_args = p
            .arguments
            .as_ref()
            .map(|a| a.game.len())
            .expect("parent has arguments");
        let merged = merge(p, child("fabric-loader-0.15.11-1.20.1"));
        assert_eq!(versions_of(&merged, "com.google.code.gson:gson"), ["2.11"]);
        assert_eq!(
            merged.main_class.as_deref(),
            Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
        );
        assert_eq!(merged.id, "fabric-loader-0.15.11-1.20.1");
        assert_eq!(
            merged.arguments.expect("arguments").game.len(),
            game_args + 1
        );
    }

    #[test]
    fn child_inherits_parent_downloads_and_assets() {
        let p = parent();
        let assets = p.assets.clone();
        let merged = merge(p, child("fabric-loader-0.15.11-1.20.1"));
        assert_eq!(merged.assets, assets);
        assert!(merged.downloads.is_some());
        assert!(merged.asset_index.is_some());
        assert_eq!(merged.java_version.expect("inherited").major_version, 17);
    }

    #[test]
    fn a_forge_child_keeps_both_library_versions() {
        let merged = merge(parent(), child("1.20.1-forge-47.2.0"));
        assert_eq!(
            versions_of(&merged, "com.google.code.gson:gson"),
            ["2.10", "2.11"]
        );
    }

    #[test]
    fn legacy_arguments_take_the_child_when_it_has_them() {
        let mut c = child("legacy-child");
        c.minecraft_arguments = Some("--child".to_string());
        let mut p = parent();
        p.minecraft_arguments = Some("--parent".to_string());
        assert_eq!(merge(p, c).minecraft_arguments.as_deref(), Some("--child"));

        let mut p = parent();
        p.minecraft_arguments = Some("--parent".to_string());
        assert_eq!(
            merge(p, child("legacy-child"))
                .minecraft_arguments
                .as_deref(),
            Some("--parent")
        );
    }

    fn mojang_with(dir: &Path) -> Mojang {
        Mojang::new(
            HttpClient::new().expect("client builds"),
            Root::from_path(dir),
        )
    }

    #[test]
    fn resolve_merges_a_cached_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mojang = mojang_with(dir.path());
        write_atomic(&mojang.version_file("1.20.1"), V1_20_1.as_bytes()).expect("seed parent");
        let resolved = mojang
            .resolve(child("fabric-loader-0.15.11-1.20.1"))
            .expect("resolves");
        assert_eq!(resolved.id, "fabric-loader-0.15.11-1.20.1");
        assert_eq!(
            versions_of(&resolved, "com.google.code.gson:gson"),
            ["2.11"]
        );
        assert!(resolved.inherits_from.is_none());
    }

    #[test]
    fn resolve_returns_a_version_with_no_parent_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let v = parent();
        let resolved = mojang_with(dir.path())
            .resolve(v.clone())
            .expect("resolves");
        assert_eq!(resolved, v);
    }

    #[test]
    fn resolve_reports_a_self_referencing_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mojang = mojang_with(dir.path());
        let mut v = parent();
        v.inherits_from = Some(v.id.clone());
        let body = serde_json::to_string(&v).expect("serializes");
        write_atomic(&mojang.version_file(&v.id), body.as_bytes()).expect("seed");
        assert!(matches!(mojang.resolve(v), Err(Error::InheritanceLoop(_))));
    }

    #[test]
    fn resolve_reports_a_parent_that_is_not_cached() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            mojang_with(dir.path()).resolve(child("fabric-loader-0.15.11-1.20.1")),
            Err(Error::UnknownVersion(_))
        ));
    }
}
