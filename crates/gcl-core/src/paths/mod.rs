//! App root layout: where config, instances, and caches live on disk.
//!
//! See `ARCHITECTURE.md` "App root layout" and the `instance-model` skill for the schema.

use std::path::{Component, Path, PathBuf};

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
    /// A remote source named a relative path that would escape its base directory.
    #[error("unsafe path: {0}")]
    UnsafePath(String),
    /// A string used as a content hash is not 40 lowercase hex characters.
    #[error("not a sha1 hash: {0}")]
    BadHash(String),
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
    ///
    /// Rejects anything that is not 40 lowercase hex characters, so a hash taken from remote
    /// JSON can never steer the write out of `cache/objects/`.
    pub fn object_path(&self, sha1: &str) -> Result<PathBuf, Error> {
        if !is_sha1_hex(sha1) {
            return Err(Error::BadHash(sha1.to_string()));
        }
        Ok(self.objects_dir().join(&sha1[..2]).join(sha1))
    }

    /// Directory holding launcher and game log files.
    pub fn logs_dir(&self) -> PathBuf {
        self.0.join("logs")
    }

    /// Creates every directory in the layout.
    ///
    /// It excludes the per-instance directories under `instances/`, created by
    /// `instances::create`, and the per-version natives directories under `cache/natives/`,
    /// created when an install extracts natives.
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

/// Sequence number keeping two temp files in the same directory apart.
static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Writes `bytes` to `path` through a temp file in the same directory, then renames.
///
/// A reader never sees a half-written file. Parent directories are created as needed.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    write_atomic_inner(path, bytes, None)
}

/// Writes `bytes` to `path` atomically, with the temp file created at `mode` on unix.
///
/// Use this for a file that must never be readable by anyone else, such as the refresh-token
/// store: [`write_atomic`] leaves its temp file at the process umask, so a secret written
/// through it is world-readable until the caller chmods the renamed file, and forever if the
/// process dies in between. `mode` is ignored off unix, where the user's profile directory
/// carries the restriction instead.
pub fn write_atomic_with_mode(path: &Path, bytes: &[u8], mode: u32) -> Result<(), Error> {
    write_atomic_inner(path, bytes, Some(mode))
}

/// Shared body: write a temp sibling, flush it to disk, then rename it over `path`.
fn write_atomic_inner(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), Error> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Error::Io { path, source }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_file_name(format!("{name}.{}.{seq}.tmp", std::process::id()));
    match write_tmp(&tmp, bytes, mode) {
        Ok(()) => std::fs::rename(&tmp, path).map_err(io(path)),
        Err(source) => {
            let _ = std::fs::remove_file(&tmp);
            Err(Error::Io { path: tmp, source })
        }
    }
}

/// Creates the temp file, at `mode` when one is asked for, writes it, and syncs it.
///
/// `create_new` means the file is never opened over an existing one, so the mode is the mode
/// the bytes are written under: there is no window at a wider mode.
fn write_tmp(tmp: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), std::io::Error> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// True when `s` is exactly 40 lowercase hex characters, the shape of a sha1 we store by.
pub fn is_sha1_hex(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Joins a source-supplied relative path onto `base`, refusing anything that escapes it.
///
/// Rejects absolute paths, roots and prefixes, `..`, and an empty result. `.` components are
/// dropped. Every path a remote JSON file names goes through here.
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, Error> {
    let escape = || Error::UnsafePath(rel.to_string());
    let mut out = base.to_path_buf();
    let mut pushed = 0usize;
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => {
                out.push(part);
                pushed += 1;
            }
            Component::CurDir => {}
            _ => return Err(escape()),
        }
    }
    if pushed == 0 || !out.starts_with(base) {
        return Err(escape());
    }
    Ok(out)
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
        let sha1 = "abcdef0123456789abcdef0123456789abcdef01";
        let p = root.object_path(sha1).expect("valid sha1");
        assert!(p.ends_with(format!("objects/ab/{sha1}")));
    }

    #[test]
    fn object_path_rejects_anything_that_is_not_a_sha1() {
        let root = Root::from_path("/root");
        for bad in [
            "../x",
            "",
            "ab12",
            "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
            "../../etc/passwd/aaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(
                matches!(root.object_path(bad), Err(Error::BadHash(_))),
                "{bad} was accepted"
            );
        }
    }

    #[test]
    fn safe_join_keeps_relative_paths_inside_the_base() {
        let base = Path::new("/base");
        assert_eq!(
            safe_join(base, "com/example/a.jar").expect("plain path"),
            Path::new("/base/com/example/a.jar")
        );
        assert_eq!(
            safe_join(base, "./a.jar").expect("cur dir"),
            Path::new("/base/a.jar")
        );
    }

    #[test]
    fn safe_join_refuses_paths_that_escape() {
        let base = Path::new("/base");
        for bad in [
            "../escape.jar",
            "a/../../escape.jar",
            "/etc/passwd",
            "",
            ".",
        ] {
            assert!(
                matches!(safe_join(base, bad), Err(Error::UnsafePath(_))),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn is_sha1_hex_wants_forty_lowercase_hex_chars() {
        assert!(is_sha1_hex(&"a0f".repeat(14)[..40]));
        assert!(!is_sha1_hex(&"A".repeat(40)));
        assert!(!is_sha1_hex(&"a".repeat(39)));
        assert!(!is_sha1_hex(&"g".repeat(40)));
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
    fn write_atomic_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("file.toml");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_with_mode_never_widens_the_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.json");
        write_atomic_with_mode(&path, b"one", 0o600).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode was {mode:o}");

        // A second write renames over the first file and keeps the mode.
        write_atomic_with_mode(&path, b"two", 0o600).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode was {mode:o}");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "secret.json")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
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
