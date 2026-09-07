//! Where the Microsoft refresh token is kept: the OS keyring, or a restricted file.
//!
//! [`open_default`] picks the store. The OS keyring is preferred. When there is none —
//! a Linux box with no Secret Service, for instance — the launcher falls back to
//! `<root>/secrets.json` and emits one [`Event::Warning`] saying so.
//!
//! Only the refresh token lives here. The Minecraft token is cached in `accounts.json`,
//! so a launch never touches the keyring.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::events::{Event, EventSink};
use crate::paths::{Root, write_atomic};

use super::Error;

/// Keyring service name every account entry is filed under.
pub const SERVICE: &str = "grid-craft-launcher";

/// Keyring user [`KeyringStore::probe`] writes and deletes to test the store.
const PROBE_USER: &str = "__probe__";

/// Debug-only override that forces the file store. Tests set it so they never write to
/// the developer's real keyring.
pub const NO_KEYRING_ENV: &str = "GCL_NO_KEYRING";

/// Name of the fallback file inside the app root.
const FILE_NAME: &str = "secrets.json";

/// Which backing store a [`SecretStore`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretStoreKind {
    /// The operating system's credential store.
    Keyring,
    /// A JSON file in the app root, readable by the user only.
    File,
    /// In-process only, for tests.
    Memory,
}

/// Stores one secret per account id.
pub trait SecretStore: Send + Sync {
    /// Saves `refresh_token` for `account_id`, replacing any earlier one.
    fn put(&self, account_id: &str, refresh_token: &str) -> Result<(), Error>;
    /// Reads the token for `account_id`, or `None` when there is none.
    fn get(&self, account_id: &str) -> Result<Option<String>, Error>;
    /// Removes the token for `account_id`. Removing an absent token is not an error.
    fn delete(&self, account_id: &str) -> Result<(), Error>;
    /// Which backing store this is.
    fn kind(&self) -> SecretStoreKind;
}

/// The OS credential store: Keychain, Credential Manager, or Secret Service.
#[derive(Debug)]
pub struct KeyringStore;

impl KeyringStore {
    /// Checks that the OS keyring works, by writing, reading, and deleting a probe entry.
    ///
    /// keyring 4 picks the native store itself on the first `Entry::new`, so there is no
    /// setup call. Any failure becomes [`Error::KeyringUnavailable`] and the caller falls
    /// back to [`FileStore`].
    pub fn probe() -> Result<KeyringStore, Error> {
        let unavailable = |err: keyring::Error| Error::KeyringUnavailable(err.to_string());
        let entry = keyring::Entry::new(SERVICE, PROBE_USER).map_err(unavailable)?;
        entry.set_password("probe").map_err(unavailable)?;
        entry.get_password().map_err(unavailable)?;
        entry.delete_credential().map_err(unavailable)?;
        Ok(KeyringStore)
    }

    /// Opens the entry for one account.
    fn entry(&self, account_id: &str) -> Result<keyring::Entry, Error> {
        keyring::Entry::new(SERVICE, account_id).map_err(|err| Error::Keyring(err.to_string()))
    }
}

impl SecretStore for KeyringStore {
    #[tracing::instrument(skip(self, refresh_token))]
    fn put(&self, account_id: &str, refresh_token: &str) -> Result<(), Error> {
        self.entry(account_id)?
            .set_password(refresh_token)
            .map_err(|err| Error::Keyring(err.to_string()))
    }

    #[tracing::instrument(skip(self))]
    fn get(&self, account_id: &str) -> Result<Option<String>, Error> {
        match self.entry(account_id)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(Error::Keyring(err.to_string())),
        }
    }

    #[tracing::instrument(skip(self))]
    fn delete(&self, account_id: &str) -> Result<(), Error> {
        match self.entry(account_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(Error::Keyring(err.to_string())),
        }
    }

    fn kind(&self) -> SecretStoreKind {
        SecretStoreKind::Keyring
    }
}

/// The fallback store: a JSON map of account id to token at `<root>/secrets.json`.
///
/// On unix the file is chmod 0600 after every write, so only the owner can read it.
#[derive(Debug, Clone)]
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    /// Builds a file store at `<root>/secrets.json`.
    pub fn new(root: &Root) -> Self {
        FileStore {
            path: root.path().join(FILE_NAME),
        }
    }

    /// Builds a file store at an exact path.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        FileStore { path: path.into() }
    }

    /// Where the tokens are written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the map. A missing file is an empty map.
    fn load(&self) -> Result<BTreeMap<String, String>, Error> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(source) => {
                return Err(Error::Io {
                    path: self.path.clone(),
                    source,
                });
            }
        };
        serde_json::from_str(&text).map_err(|source| Error::Json {
            path: self.path.clone(),
            source,
        })
    }

    /// Writes the map through a temp file and a rename, then restricts the file.
    fn save(&self, map: &BTreeMap<String, String>) -> Result<(), Error> {
        let text = serde_json::to_string_pretty(map).map_err(|source| Error::Json {
            path: self.path.clone(),
            source,
        })?;
        write_atomic(&self.path, text.as_bytes()).map_err(|err| match err {
            crate::paths::Error::Io { path, source } => Error::Io { path, source },
            other => Error::Io {
                path: self.path.clone(),
                source: std::io::Error::other(other.to_string()),
            },
        })?;
        self.restrict()
    }

    /// Sets mode 0600 on the file. A no-op off unix.
    #[cfg(unix)]
    fn restrict(&self) -> Result<(), Error> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600)).map_err(
            |source| Error::Io {
                path: self.path.clone(),
                source,
            },
        )
    }

    /// Windows and everything else rely on the user's profile directory instead.
    #[cfg(not(unix))]
    fn restrict(&self) -> Result<(), Error> {
        Ok(())
    }
}

impl SecretStore for FileStore {
    #[tracing::instrument(skip(self, refresh_token))]
    fn put(&self, account_id: &str, refresh_token: &str) -> Result<(), Error> {
        let mut map = self.load()?;
        map.insert(account_id.to_string(), refresh_token.to_string());
        self.save(&map)
    }

    #[tracing::instrument(skip(self))]
    fn get(&self, account_id: &str) -> Result<Option<String>, Error> {
        Ok(self.load()?.remove(account_id))
    }

    #[tracing::instrument(skip(self))]
    fn delete(&self, account_id: &str) -> Result<(), Error> {
        let mut map = self.load()?;
        if map.remove(account_id).is_none() {
            return Ok(());
        }
        self.save(&map)
    }

    fn kind(&self) -> SecretStoreKind {
        SecretStoreKind::File
    }
}

/// An in-process store that keeps nothing after the run. For tests.
#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<std::collections::HashMap<String, String>>);

impl MemoryStore {
    /// Builds an empty store.
    pub fn new() -> Self {
        MemoryStore::default()
    }

    /// Takes the lock, treating a poisoned mutex as the map it still holds.
    fn map(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, String>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SecretStore for MemoryStore {
    fn put(&self, account_id: &str, refresh_token: &str) -> Result<(), Error> {
        self.map()
            .insert(account_id.to_string(), refresh_token.to_string());
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<Option<String>, Error> {
        Ok(self.map().get(account_id).cloned())
    }

    fn delete(&self, account_id: &str) -> Result<(), Error> {
        self.map().remove(account_id);
        Ok(())
    }

    fn kind(&self) -> SecretStoreKind {
        SecretStoreKind::Memory
    }
}

/// Opens the OS keyring, or falls back to a restricted file and warns once.
#[tracing::instrument(skip_all)]
pub fn open_default(root: &Root, sink: &EventSink) -> Box<dyn SecretStore> {
    if !force_file_store() {
        match KeyringStore::probe() {
            Ok(store) => return Box::new(store),
            Err(err) => tracing::warn!(%err, "no OS keyring: falling back to a file"),
        }
    }
    let store = FileStore::new(root);
    let _ = sink.send(Event::Warning(format!(
        "no OS keyring available; refresh tokens are stored in {} with restricted permissions",
        store.path().display()
    )));
    Box::new(store)
}

/// True when `GCL_NO_KEYRING=1` asks for the file store. Debug builds only, like the
/// `GCL_*_BASE_URL` overrides: a shipped launcher always tries the real keyring.
#[cfg(debug_assertions)]
fn force_file_store() -> bool {
    std::env::var(NO_KEYRING_ENV).as_deref() == Ok("1")
}

/// A release build reads no environment here.
#[cfg(not(debug_assertions))]
fn force_file_store() -> bool {
    false
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
