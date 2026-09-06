//! `gcl account`: offline accounts, the active account, and removal.

use anyhow::Result;
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::auth::offline::offline_account;
use gcl_core::auth::{Account, AccountKind};
use serde::Serialize;

use crate::output::{Format, print_json, print_table};

/// Subcommands under `gcl account`.
#[derive(Subcommand)]
pub enum AccountCommand {
    /// Add an offline account. It becomes active when no account is active yet.
    AddOffline {
        /// In-game name.
        name: String,
    },
    /// List every saved account.
    List,
    /// Make one account the active account.
    Select {
        /// Account id, or its exact name.
        id_or_name: String,
    },
    /// Remove one account.
    Remove {
        /// Account id, or its exact name.
        id_or_name: String,
    },
}

/// One account as the CLI reports it. The token fields are never printed.
#[derive(Serialize)]
struct AccountRow {
    id: String,
    name: String,
    kind: String,
    active: bool,
}

impl AccountRow {
    /// Builds a row, marking the account active when its id matches `active`.
    fn new(account: &Account, active: Option<&str>) -> AccountRow {
        AccountRow {
            id: account.id.clone(),
            name: account.name.clone(),
            kind: kind_name(account.kind).to_string(),
            active: active == Some(account.id.as_str()),
        }
    }
}

/// Runs one `gcl account` subcommand.
pub fn run(launcher: &Launcher, format: Format, command: AccountCommand) -> Result<()> {
    let accounts = launcher.accounts();
    match command {
        AccountCommand::AddOffline { name } => {
            let added = accounts.add(offline_account(&name))?;
            let active = accounts.active()?.map(|a| a.id);
            report_one(format, &AccountRow::new(&added, active.as_deref()))
        }
        AccountCommand::List => {
            let active = accounts.active()?.map(|a| a.id);
            let rows: Vec<AccountRow> = accounts
                .list()?
                .iter()
                .map(|a| AccountRow::new(a, active.as_deref()))
                .collect();
            match format {
                Format::Json => print_json(&rows),
                Format::Text => {
                    let cells: Vec<Vec<String>> = rows
                        .iter()
                        .map(|r| {
                            vec![
                                marker(r.active).to_string(),
                                r.id.clone(),
                                r.name.clone(),
                                r.kind.clone(),
                            ]
                        })
                        .collect();
                    print_table(&["ACTIVE", "ID", "NAME", "KIND"], &cells);
                    Ok(())
                }
            }
        }
        AccountCommand::Select { id_or_name } => {
            let selected = accounts.select(&id_or_name)?;
            report_one(format, &AccountRow::new(&selected, Some(&selected.id)))
        }
        AccountCommand::Remove { id_or_name } => {
            // `remove` takes an id, so a name is resolved against the saved list first.
            let saved = accounts.list()?;
            let found = saved
                .iter()
                .find(|a| a.id == id_or_name)
                .or_else(|| saved.iter().find(|a| a.name == id_or_name))
                .ok_or_else(|| gcl_core::auth::Error::NotFound(id_or_name.clone()))?;
            let id = found.id.clone();
            // Removing the active account leaves none active, which the report says so the
            // next launch's "no account selected" is not a surprise.
            let was_active = accounts.active()?.map(|a| a.id) == Some(id.clone());
            accounts.remove(&id)?;
            match format {
                Format::Json => print_json(&serde_json::json!({
                    "removed": id, "was_active": was_active,
                })),
                Format::Text => {
                    match was_active {
                        true => println!("removed {id} (was active; no active account now)"),
                        false => println!("removed {id}"),
                    }
                    Ok(())
                }
            }
        }
    }
}

/// Prints one account, as JSON or as an `id name` line with an active marker.
fn report_one(format: Format, row: &AccountRow) -> Result<()> {
    match format {
        Format::Json => print_json(row),
        Format::Text => {
            println!("{} {}{}", row.id, row.name, suffix(row.active));
            Ok(())
        }
    }
}

/// Lowercase name of an account kind, matching how it is stored on disk.
fn kind_name(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Offline => "offline",
        AccountKind::Msa => "msa",
    }
}

/// The marker shown in the `ACTIVE` column.
fn marker(active: bool) -> &'static str {
    if active { "*" } else { " " }
}

/// The word appended to a one-line account report.
fn suffix(active: bool) -> &'static str {
    if active { " (active)" } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_active_only_when_the_ids_match() {
        let account = Account {
            id: "abc".to_string(),
            name: "alice".to_string(),
            kind: AccountKind::Offline,
            mc_token: None,
            mc_token_expires: None,
            xuid: None,
        };
        assert!(AccountRow::new(&account, Some("abc")).active);
        assert!(!AccountRow::new(&account, Some("other")).active);
        assert!(!AccountRow::new(&account, None).active);
    }
}
