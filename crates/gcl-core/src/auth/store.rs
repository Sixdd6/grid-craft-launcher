//! `accounts.json`: the saved accounts and which one is active.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths::{Root, write_atomic_with_mode};

use super::{Account, Error};

/// The on-disk shape of `accounts.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AccountsFile {
    /// Every saved account.
    pub accounts: Vec<Account>,
    /// The id of the active account, if one is selected.
    pub active: Option<String>,
}

/// Unix permissions `accounts.json` is written with: owner read and write only.
const ACCOUNTS_MODE: u32 = 0o600;

/// Reads and writes the account store at `<root>/accounts.json`.
#[derive(Debug, Clone)]
pub struct Accounts {
    path: PathBuf,
}

impl Accounts {
    /// Builds an account store over the given app root.
    pub fn new(root: &Root) -> Self {
        Accounts {
            path: root.accounts_file(),
        }
    }

    /// Loads `accounts.json`. A missing file yields [`AccountsFile::default`].
    pub fn load(&self) -> Result<AccountsFile, Error> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AccountsFile::default());
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

    /// Writes `f` to `accounts.json`, through a temp file and a rename.
    ///
    /// The file is created at mode `0600` on unix: it holds a live Minecraft token, so no
    /// other user on the machine may read it, not even for the moment between the temp write
    /// and the rename.
    pub fn save(&self, f: &AccountsFile) -> Result<(), Error> {
        let text = serde_json::to_string_pretty(f).map_err(|source| Error::Json {
            path: self.path.clone(),
            source,
        })?;
        write_atomic_with_mode(&self.path, text.as_bytes(), ACCOUNTS_MODE).map_err(
            |err| match err {
                crate::paths::Error::Io { path, source } => Error::Io { path, source },
                other => Error::Io {
                    path: self.path.clone(),
                    source: std::io::Error::other(other.to_string()),
                },
            },
        )
    }

    /// Adds or replaces an account by id. Sets it active when no account is active yet.
    ///
    /// Returns the stored account.
    pub fn add(&self, a: Account) -> Result<Account, Error> {
        let mut file = self.load()?;
        match file
            .accounts
            .iter_mut()
            .find(|existing| existing.id == a.id)
        {
            Some(existing) => *existing = a.clone(),
            None => file.accounts.push(a.clone()),
        }
        if file.active.is_none() {
            file.active = Some(a.id.clone());
        }
        self.save(&file)?;
        Ok(a)
    }

    /// Removes the account with the given id. Clears `active` if it was the active account.
    pub fn remove(&self, id: &str) -> Result<(), Error> {
        let mut file = self.load()?;
        file.accounts.retain(|a| a.id != id);
        if file.active.as_deref() == Some(id) {
            file.active = None;
        }
        self.save(&file)
    }

    /// Finds an account by id, or by exact (case-sensitive) name if no id matches.
    ///
    /// Reads only: a one-off `--account` launch must not change which account is active.
    pub fn find(&self, id_or_name: &str) -> Result<Option<Account>, Error> {
        let file = self.load()?;
        Ok(file
            .accounts
            .iter()
            .find(|a| a.id == id_or_name)
            .or_else(|| file.accounts.iter().find(|a| a.name == id_or_name))
            .cloned())
    }

    /// Selects an account by id, or by exact (case-sensitive) name if no id matches, and
    /// makes it the active account.
    pub fn select(&self, id_or_name: &str) -> Result<Account, Error> {
        let mut file = self.load()?;
        let found = file
            .accounts
            .iter()
            .find(|a| a.id == id_or_name)
            .or_else(|| file.accounts.iter().find(|a| a.name == id_or_name))
            .cloned()
            .ok_or_else(|| Error::NotFound(id_or_name.to_string()))?;
        file.active = Some(found.id.clone());
        self.save(&file)?;
        Ok(found)
    }

    /// The active account, if one is selected and still present.
    pub fn active(&self) -> Result<Option<Account>, Error> {
        let file = self.load()?;
        let Some(id) = file.active else {
            return Ok(None);
        };
        Ok(file.accounts.into_iter().find(|a| a.id == id))
    }

    /// Every saved account.
    pub fn list(&self) -> Result<Vec<Account>, Error> {
        Ok(self.load()?.accounts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AccountKind;

    fn account(id: &str, name: &str) -> Account {
        Account {
            id: id.to_string(),
            name: name.to_string(),
            kind: AccountKind::Offline,
            mc_token: None,
            mc_token_expires: None,
            xuid: None,
            refresh_store: None,
        }
    }

    fn store() -> (tempfile::TempDir, Accounts) {
        let dir = tempfile::tempdir().unwrap();
        let root = Root::from_path(dir.path());
        (dir, Accounts::new(&root))
    }

    #[test]
    fn load_of_a_missing_file_is_default() {
        let (_dir, accounts) = store();
        assert_eq!(accounts.load().unwrap(), AccountsFile::default());
    }

    #[test]
    fn add_list_select_remove_round_trip() {
        let (_dir, accounts) = store();
        let a = account("id-1", "Alice");
        let b = account("id-2", "Bob");
        accounts.add(a.clone()).unwrap();
        accounts.add(b.clone()).unwrap();

        let listed = accounts.list().unwrap();
        assert_eq!(listed, vec![a.clone(), b.clone()]);

        let selected = accounts.select("Bob").unwrap();
        assert_eq!(selected, b);
        assert_eq!(accounts.active().unwrap(), Some(b.clone()));

        accounts.remove("id-2").unwrap();
        assert_eq!(accounts.list().unwrap(), vec![a]);
        assert_eq!(accounts.active().unwrap(), None);
    }

    #[test]
    fn add_of_an_existing_id_replaces_it() {
        let (_dir, accounts) = store();
        accounts.add(account("id-1", "Alice")).unwrap();
        let renamed = account("id-1", "Alice Renamed");
        accounts.add(renamed.clone()).unwrap();
        let listed = accounts.list().unwrap();
        assert_eq!(listed, vec![renamed]);
    }

    #[test]
    fn first_add_becomes_active_but_a_second_does_not_steal_it() {
        let (_dir, accounts) = store();
        let a = accounts.add(account("id-1", "Alice")).unwrap();
        accounts.add(account("id-2", "Bob")).unwrap();
        assert_eq!(accounts.active().unwrap(), Some(a));
    }

    #[test]
    fn select_by_id_matches_before_name() {
        let (_dir, accounts) = store();
        let a = account("Bob", "Alice");
        let b = account("id-2", "Bob");
        accounts.add(a.clone()).unwrap();
        accounts.add(b).unwrap();
        assert_eq!(accounts.select("Bob").unwrap(), a);
    }

    #[test]
    fn select_is_case_sensitive_on_name() {
        let (_dir, accounts) = store();
        accounts.add(account("id-1", "Alice")).unwrap();
        assert!(matches!(accounts.select("alice"), Err(Error::NotFound(_))));
    }

    #[test]
    fn select_of_an_unknown_name_is_not_found() {
        let (_dir, accounts) = store();
        assert!(matches!(accounts.select("nobody"), Err(Error::NotFound(_))));
    }

    #[test]
    fn active_after_removing_the_active_account_is_none() {
        let (_dir, accounts) = store();
        accounts.add(account("id-1", "Alice")).unwrap();
        assert!(accounts.active().unwrap().is_some());
        accounts.remove("id-1").unwrap();
        assert_eq!(accounts.active().unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn accounts_json_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, accounts) = store();
        accounts.add(account("id-1", "Alice")).unwrap();
        let mode = std::fs::metadata(dir.path().join("accounts.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "accounts.json holds a live Minecraft token");
    }

    #[test]
    fn save_leaves_no_temp_file_beside_accounts_json() {
        let (dir, accounts) = store();
        accounts.add(account("id-1", "Alice")).unwrap();
        let root_dir = dir.path();
        let leftovers: Vec<String> = std::fs::read_dir(root_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
