//! App root layout: where config, instances, and caches live on disk.
//!
//! See `ARCHITECTURE.md` "App root layout" and the `instance-model` skill for the schema.

use std::path::{Path, PathBuf};

/// Errors resolving or creating the app root layout.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No platform data directory is available to fall back to.
    #[error("no data directory available on this platform")]
    NoDataDir,
    /// An I/O operation on a path in the layout failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// The launcher's app root: base directory for config, instances, cache, and logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root(PathBuf);

impl Root {
    /// Resolves the app root: `GCL_ROOT` env var, then `config_override`, then the
    /// platform data directory.
    pub fn resolve(config_override: Option<&Path>) -> Result<Root, Error> {
        let env_root = std::env::var_os("GCL_ROOT").map(PathBuf::from);
        Self::resolve_with(env_root, config_override)
    }

    fn resolve_with(
        env_root: Option<PathBuf>,
        config_override: Option<&Path>,
    ) -> Result<Root, Error> {
        if let Some(root) = env_root {
            return Ok(Root(root));
        }
        if let Some(root) = config_override {
            return Ok(Root(root.to_path_buf()));
        }
        let dirs = directories::ProjectDirs::from("", "", "grid-craft-launcher")
            .ok_or(Error::NoDataDir)?;
        Ok(Root(dirs.data_dir().to_path_buf()))
    }

    /// Builds a `Root` directly from a path, bypassing resolution. For tests and CLI overrides.
    pub fn from_path(p: impl Into<PathBuf>) -> Root {
        Root(p.into())
    }

    /// The root directory itself.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Path to `config.toml`.
    pub fn config_file(&self) -> PathBuf {
        self.0.join("config.toml")
    }

    /// Path to `accounts.json`.
    pub fn accounts_file(&self) -> PathBuf {
        self.0.join("accounts.json")
    }

    /// Directory holding every instance.
    pub fn instances_dir(&self) -> PathBuf {
        self.0.join("instances")
    }

    /// Directory for a single instance, named by its slug.
    pub fn instance_dir(&self, slug: &str) -> PathBuf {
        self.instances_dir().join(slug)
    }

    /// The cache directory root.
    pub fn cache_dir(&self) -> PathBuf {
        self.0.join("cache")
    }

    /// Directory holding cached version manifests.
    pub fn versions_dir(&self) -> PathBuf {
        self.cache_dir().join("versions")
    }

    /// Directory holding cached library jars.
    pub fn libraries_dir(&self) -> PathBuf {
        self.cache_dir().join("libraries")
    }

    /// Directory holding cached asset indexes and objects.
    pub fn assets_dir(&self) -> PathBuf {
        self.cache_dir().join("assets")
    }

    /// Directory holding extracted natives for a given version id.
    pub fn natives_dir(&self, version_id: &str) -> PathBuf {
        self.cache_dir().join("natives").join(version_id)
    }

    /// Directory holding installed JVM runtimes.
    pub fn runtimes_dir(&self) -> PathBuf {
        self.cache_dir().join("runtimes")
    }

    /// Directory holding downloaded mod loader installers.
    pub fn installers_dir(&self) -> PathBuf {
        self.cache_dir().join("installers")
    }

    /// Directory holding content-addressed objects, keyed by sha1.
    pub fn objects_dir(&self) -> PathBuf {
        self.cache_dir().join("objects")
    }

    /// Path to a content-addressed object, sharded by the first two hex chars of its sha1.
    pub fn object_path(&self, sha1: &str) -> PathBuf {
        let shard = if sha1.len() >= 2 { &sha1[..2] } else { sha1 };
        self.objects_dir().join(shard).join(sha1)
    }

    /// Directory holding launcher and game log files.
    pub fn logs_dir(&self) -> PathBuf {
        self.0.join("logs")
    }

    /// Creates every directory in the layout (except per-instance directories, which are
    /// created on instance creation).
    pub fn ensure_layout(&self) -> Result<(), Error> {
        for dir in [
            self.0.clone(),
            self.instances_dir(),
            self.cache_dir(),
            self.versions_dir(),
            self.libraries_dir(),
            self.assets_dir(),
            self.runtimes_dir(),
            self.installers_dir(),
            self.objects_dir(),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(&dir).map_err(|source| Error::Io {
                path: dir.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

/// Turns an arbitrary name into a filesystem- and URL-safe slug.
///
/// Lowercases, keeps `[a-z0-9-]`, collapses runs of `-`, trims leading/trailing `-`, and
/// falls back to `"instance"` if the result would be empty.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_dash = false;
    for ch in name.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "instance".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Returns `base` if it doesn't collide, else `base-2`, `base-3`, ... until `exists` is false.
pub fn unique_slug(base: &str, exists: impl Fn(&str) -> bool) -> String {
    if !exists(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_replaces_punctuation() {
        assert_eq!(slugify("My Fabric 1.20.1!"), "my-fabric-1-20-1");
    }

    #[test]
    fn slugify_empty_falls_back_to_instance() {
        assert_eq!(slugify(""), "instance");
        assert_eq!(slugify("!!!"), "instance");
    }

    #[test]
    fn unique_slug_appends_next_free_suffix() {
        let taken = |s: &str| s == "x" || s == "x-2";
        assert_eq!(unique_slug("x", taken), "x-3");
    }

    #[test]
    fn unique_slug_returns_base_when_free() {
        assert_eq!(unique_slug("free", |_| false), "free");
    }

    #[test]
    fn object_path_shards_by_first_two_chars() {
        let root = Root::from_path("/root");
        let p = root.object_path("abcdef0123");
        assert!(p.ends_with("objects/ab/abcdef0123"));
    }

    #[test]
    fn resolve_honors_gcl_root_env_over_override() {
        let root = Root::resolve_with(
            Some(PathBuf::from("/from-env")),
            Some(Path::new("/from-override")),
        )
        .unwrap();
        assert_eq!(root.path(), Path::new("/from-env"));
    }

    #[test]
    fn resolve_falls_back_to_override_when_no_env() {
        let root = Root::resolve_with(None, Some(Path::new("/from-override"))).unwrap();
        assert_eq!(root.path(), Path::new("/from-override"));
    }

    #[test]
    fn resolve_falls_back_to_platform_data_dir() {
        let root = Root::resolve_with(None, None).unwrap();
        assert!(!root.path().as_os_str().is_empty());
    }

    #[test]
    fn ensure_layout_creates_every_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = Root::from_path(dir.path());
        root.ensure_layout().unwrap();
        for p in [
            root.path().to_path_buf(),
            root.instances_dir(),
            root.cache_dir(),
            root.versions_dir(),
            root.libraries_dir(),
            root.assets_dir(),
            root.runtimes_dir(),
            root.installers_dir(),
            root.objects_dir(),
            root.logs_dir(),
        ] {
            assert!(p.is_dir(), "{p:?} was not created");
        }
    }
}
