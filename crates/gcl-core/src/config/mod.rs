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

/// API keys and client ids. Environment variables win over these when reading via
/// [`Config::curseforge_api_key`] and [`Config::msa_client_id`].
#[derive(Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Keys {
    /// CurseForge API key, if configured.
    pub curseforge_api_key: Option<String>,
    /// Microsoft Entra (MSA) client id used for Microsoft account login.
    pub msa_client_id: Option<String>,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn state(v: &Option<String>) -> &'static str {
            if v.is_some() { "<set>" } else { "<unset>" }
        }
        f.debug_struct("Keys")
            .field("curseforge_api_key", &state(&self.curseforge_api_key))
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
        toml::from_str(&contents).map_err(|source| Error::Parse {
            path: path.to_path_buf(),
            source,
        })
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

    /// The CurseForge API key: `CURSEFORGE_API_KEY` env var first, then the config file.
    ///
    /// An empty or blank value in either place counts as unset: `CURSEFORGE_API_KEY=`
    /// in a shell or a `.env` must turn the source off, not send a blank key.
    pub fn curseforge_api_key(&self) -> Option<String> {
        non_blank(std::env::var("CURSEFORGE_API_KEY").ok())
            .or_else(|| non_blank(self.keys.curseforge_api_key.clone()))
    }

    /// The Microsoft account client id: `GCL_MSA_CLIENT_ID` env var first, then the config
    /// file. An empty or blank value in either place counts as unset.
    pub fn msa_client_id(&self) -> Option<String> {
        non_blank(std::env::var("GCL_MSA_CLIENT_ID").ok())
            .or_else(|| non_blank(self.keys.msa_client_id.clone()))
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
        let keys = Keys {
            curseforge_api_key: Some("abc".to_string()),
            msa_client_id: None,
        };
        let debug = format!("{keys:?}");
        assert!(!debug.contains("abc"));
        assert!(debug.contains("<set>"));
        assert!(debug.contains("<unset>"));
    }

    #[test]
    fn curseforge_api_key_prefers_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut config = Config::default();
        config.keys.curseforge_api_key = Some("from-file".to_string());
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
    fn falls_back_to_file_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: guarded by ENV_LOCK; ensures a clean slate for this process.
        unsafe {
            std::env::remove_var("CURSEFORGE_API_KEY");
        }
        let mut config = Config::default();
        config.keys.curseforge_api_key = Some("from-file".to_string());
        assert_eq!(config.curseforge_api_key(), Some("from-file".to_string()));
    }

    #[test]
    fn a_blank_key_counts_as_unset_in_the_env_and_in_the_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut config = Config::default();
        config.keys.curseforge_api_key = Some("   ".to_string());
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
        assert_eq!(curseforge, None, "a blank env var must not mask the file");
        assert_eq!(msa, None);

        // A blank file value alone is unset too.
        assert_eq!(config.curseforge_api_key(), None);
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
