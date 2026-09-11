//! Launcher config file: `config.toml` in the app root.
//!
//! See the `instance-model` skill for the schema. Secrets can also come from environment
//! variables, which win over the file so CI and packaging can avoid writing them to disk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::instances::model::GcPreset;

/// Errors loading or saving the config file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O operation on the config file failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The config file's contents could not be parsed as TOML.
    #[error("could not parse config at {path}: {source}")]
    Parse {
        /// The path being parsed.
        path: PathBuf,
        /// The underlying parse error.
        source: toml::de::Error,
    },
    /// The config could not be serialized to TOML.
    #[error(transparent)]
    Serialize(#[from] toml::ser::Error),
}

/// Default JVM memory bounds and executable override, used unless an instance overrides them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct JvmDefaults {
    /// Minimum heap size, in MiB.
    pub min_mib: u32,
    /// Maximum heap size, in MiB.
    pub max_mib: u32,
    /// Path to a specific `java` executable, if not the one on `PATH`.
    pub java_path: Option<PathBuf>,
    /// Garbage collector preset seeded into a new instance.
    #[serde(default)]
    pub gc: GcPreset,
}

impl Default for JvmDefaults {
    fn default() -> Self {
        JvmDefaults {
            min_mib: 1024,
            max_mib: 4096,
            java_path: None,
            gc: GcPreset::default(),
        }
    }
}

/// Client ids. The `GCL_MSA_CLIENT_ID` environment variable wins over this when reading via
/// [`Config::msa_client_id`]. The CurseForge key is not here: see
/// [`Config::curseforge_api_key`].
#[derive(Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Keys {
    /// Microsoft Entra (MSA) client id used for Microsoft account login.
    pub msa_client_id: Option<String>,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn state(v: &Option<String>) -> &'static str {
            if v.is_some() { "<set>" } else { "<unset>" }
        }
        f.debug_struct("Keys")
            .field("msa_client_id", &state(&self.msa_client_id))
            .finish()
    }
}

/// Launcher-wide configuration, loaded from and saved to `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Overrides the app root that would otherwise be resolved from the platform data
    /// directory.
    pub root: Option<PathBuf>,
    /// Maximum number of concurrent downloads.
    pub parallel_downloads: usize,
    /// Default JVM memory bounds and executable, used unless an instance overrides them.
    pub jvm: JvmDefaults,
    /// API keys and client ids.
    pub keys: Keys,
    /// `options.txt` keys preseeded into every new instance.
    pub game_defaults: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            root: None,
            parallel_downloads: 8,
            jvm: JvmDefaults::default(),
            keys: Keys::default(),
            game_defaults: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Loads config from `path`. A missing file yields [`Config::default`].
    pub fn load(path: &Path) -> Result<Config, Error> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Config::default());
            }
            Err(source) => {
                return Err(Error::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let config: Config = toml::from_str(&contents).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        warn_on_stale_curseforge_key(path, &contents);
        Ok(config)
    }

    /// Saves config to `path`, creating parent directories as needed.
    ///
    /// The write goes through a temp file and a rename, so a reader never sees a half-written
    /// `config.toml`.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let contents = toml::to_string_pretty(self)?;
        crate::paths::write_atomic(path, contents.as_bytes()).map_err(|err| match err {
            crate::paths::Error::Io { path, source } => Error::Io { path, source },
            other => Error::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other(other.to_string()),
            },
        })
    }

    /// The CurseForge API key: the `CURSEFORGE_API_KEY` environment variable first, then the
    /// key compiled into this build from `GCL_CURSEFORGE_API_KEY`.
    ///
    /// The environment variable is the development path, for the api-verifier agent and for
    /// recording fixtures. A release build carries the key, so a user never enters one. The
    /// config file is not read: a key there is ignored, and [`Config::load`] warns about it.
    /// An empty or blank value in either place counts as unset: `CURSEFORGE_API_KEY=` in a
    /// shell or a `.env` must turn the source off, not send a blank key.
    pub fn curseforge_api_key(&self) -> Option<String> {
        non_blank(std::env::var("CURSEFORGE_API_KEY").ok())
            .or_else(|| non_blank(option_env!("GCL_CURSEFORGE_API_KEY").map(str::to_string)))
    }

    /// The Microsoft account client id: `GCL_MSA_CLIENT_ID` env var first, then the config
    /// file. An empty or blank value in either place counts as unset.
    pub fn msa_client_id(&self) -> Option<String> {
        non_blank(std::env::var("GCL_MSA_CLIENT_ID").ok())
            .or_else(|| non_blank(self.keys.msa_client_id.clone()))
    }
}

/// Warns once when a config file still carries the removed `keys.curseforge_api_key`.
///
/// The value is never read and never logged. `Keys` no longer has the field, so serde drops
/// it silently; this reads the raw TOML to say so out loud.
fn warn_on_stale_curseforge_key(path: &Path, contents: &str) {
    let Ok(raw) = contents.parse::<toml::Value>() else {
        return;
    };
    let present = raw
        .get("keys")
        .and_then(|keys| keys.get("curseforge_api_key"))
        .is_some();
    if present {
        tracing::warn!(
            path = %path.display(),
            "config.toml: keys.curseforge_api_key is no longer read; CurseForge access ships with the build"
        );
    }
}

/// Drops a value that is empty or only whitespace.
fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_round_trips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config::default();
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn save_leaves_no_temp_file_beside_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        Config::default().save(&path).unwrap();
        let second = Config {
            parallel_downloads: 3,
            ..Config::default()
        };
        second.save(&path).unwrap();

        let leftovers: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert_eq!(Config::load(&path).unwrap().parallel_downloads, 3);
    }

    #[test]
    fn missing_file_loads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded, Config::default());
    }

    #[test]
    fn partial_file_fills_in_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[jvm]\nmax_mib = 8192\n").unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.jvm.max_mib, 8192);
        assert_eq!(loaded.jvm.min_mib, 1024);
        assert_eq!(loaded.parallel_downloads, 8);
    }

    #[test]
    fn game_defaults_survive_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();
        config
            .game_defaults
            .insert("renderDistance".to_string(), "12".to_string());
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(
            loaded.game_defaults.get("renderDistance"),
            Some(&"12".to_string())
        );
    }

    #[test]
    fn keys_debug_redacts_secrets() {
        let set = Keys {
            msa_client_id: Some("abc".to_string()),
        };
        let debug = format!("{set:?}");
        assert!(!debug.contains("abc"));
        assert!(debug.contains("<set>"));
        assert!(format!("{:?}", Keys::default()).contains("<unset>"));
    }

    #[test]
    fn curseforge_api_key_comes_from_the_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config = Config::default();
        // SAFETY: guarded by ENV_LOCK, restored before returning.
        unsafe {
            std::env::set_var("CURSEFORGE_API_KEY", "from-env");
        }
        let result = config.curseforge_api_key();
        unsafe {
            std::env::remove_var("CURSEFORGE_API_KEY");
        }
        assert_eq!(result, Some("from-env".to_string()));
    }

    #[test]
    fn msa_client_id_prefers_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut config = Config::default();
        config.keys.msa_client_id = Some("from-file".to_string());
        // SAFETY: guarded by ENV_LOCK, restored before returning.
        unsafe {
            std::env::set_var("GCL_MSA_CLIENT_ID", "from-env");
        }
        let result = config.msa_client_id();
        unsafe {
            std::env::remove_var("GCL_MSA_CLIENT_ID");
        }
        assert_eq!(result, Some("from-env".to_string()));
    }

    #[test]
    fn a_stale_key_in_the_file_still_loads_and_is_not_read() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: guarded by ENV_LOCK; ensures a clean slate for this process.
        unsafe {
            std::env::remove_var("CURSEFORGE_API_KEY");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "parallel_downloads = 5\n[keys]\ncurseforge_api_key = \"from-file\"\n",
        )
        .unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.parallel_downloads, 5, "the rest of the file loaded");
        // The build may carry a key on a developer machine; then the answer is that key,
        // never the file's.
        assert_ne!(loaded.curseforge_api_key(), Some("from-file".to_string()));
    }

    #[test]
    fn a_blank_key_counts_as_unset_in_the_env_and_in_the_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut config = Config::default();
        config.keys.msa_client_id = Some(String::new());

        // SAFETY: guarded by ENV_LOCK; both variables are restored before the asserts.
        unsafe {
            std::env::set_var("CURSEFORGE_API_KEY", "");
            std::env::set_var("GCL_MSA_CLIENT_ID", "  \t ");
        }
        let curseforge = config.curseforge_api_key();
        let msa = config.msa_client_id();
        unsafe {
            std::env::remove_var("CURSEFORGE_API_KEY");
            std::env::remove_var("GCL_MSA_CLIENT_ID");
        }
        assert_eq!(
            curseforge,
            non_blank(option_env!("GCL_CURSEFORGE_API_KEY").map(str::to_string)),
            "a blank env var must not count as a key"
        );
        assert_eq!(msa, None);

        // A blank file value alone is unset too.
        assert_eq!(config.msa_client_id(), None);
    }

    #[test]
    fn jvm_gc_defaults_to_launcher_default_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[jvm]\nmax_mib = 8192\n").unwrap();
        assert_eq!(Config::load(&path).unwrap().jvm.gc, GcPreset::Default);

        let mut config = Config::default();
        config.jvm.gc = GcPreset::G1;
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().jvm.gc, GcPreset::G1);
    }
}
