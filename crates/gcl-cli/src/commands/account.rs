//! `gcl account`: offline accounts, Microsoft sign-in, the active account, and removal.

use anyhow::Result;
use clap::Subcommand;
use gcl_core::Launcher;
use gcl_core::auth::msa::DeviceCode;
use gcl_core::auth::offline::offline_account;
use gcl_core::auth::{Account, AccountKind};
use serde::Serialize;

use crate::output::{Format, print_json, print_json_line, print_table};

/// Subcommands under `gcl account`.
#[derive(Subcommand)]
pub enum AccountCommand {
    /// Add an offline account. It becomes active when no account is active yet.
    AddOffline {
        /// In-game name.
        name: String,
    },
    /// Sign in to a Microsoft account. Prints a code to enter, then waits for approval.
    AddMsa,
    /// Sign a saved Microsoft account in again from its stored refresh token.
    Refresh {
        /// Account id, or its exact name.
        id_or_name: String,
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
    /// RFC 3339 expiry of the Minecraft token. `None` for an offline account, which has
    /// no token to expire.
    expires: Option<String>,
}

/// The first line `account add-msa --json` prints: what to show the user, and nothing else.
///
/// The device code itself is a secret and is never printed. [`DeviceCode`] skips it when it
/// serializes, and this struct does not carry it at all.
#[derive(Serialize)]
struct DeviceCodeRow<'a> {
    user_code: &'a str,
    verification_uri: &'a str,
}

impl AccountRow {
    /// Builds a row, marking the account active when its id matches `active`.
    fn new(account: &Account, active: Option<&str>) -> AccountRow {
        AccountRow {
            id: account.id.clone(),
            name: account.name.clone(),
            kind: kind_name(account.kind).to_string(),
            active: active == Some(account.id.as_str()),
            expires: account.mc_token_expires.clone(),
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
        AccountCommand::AddMsa => {
            // The closure runs on the login thread, before the sign-in finishes, so it
            // prints on its own instead of returning anything.
            let added = launcher.msa_login(&move |code: &DeviceCode| show_code(format, code))?;
            let active = accounts.active()?.map(|a| a.id);
            let row = AccountRow::new(&added, active.as_deref());
            match format {
                // One line, so the code line above and this one are two JSON documents a
                // caller can read line by line.
                Format::Json => print_json_line(&row),
                Format::Text => {
                    println!("{} {}{}", row.id, row.name, suffix(row.active));
                    Ok(())
                }
            }
        }
        AccountCommand::Refresh { id_or_name } => {
            let refreshed = launcher.msa_refresh(&id_or_name)?;
            let active = accounts.active()?.map(|a| a.id);
            let row = AccountRow::new(&refreshed, active.as_deref());
            match format {
                Format::Json => print_json(&row),
                Format::Text => {
                    println!("refreshed {} (expires {})", row.name, expires_cell(&row));
                    Ok(())
                }
            }
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
                                expires_cell(r).to_string(),
                            ]
                        })
                        .collect();
                    print_table(&["ACTIVE", "ID", "NAME", "KIND", "EXPIRES"], &cells);
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

/// Shows the device code the user has to enter, then reports it to the terminal at once.
///
/// A print failure would leave the user waiting with no code, so it is reported on stderr
/// rather than ending the sign-in. Writing goes through [`write_code`], which flushes: the
/// poll loop blocks right after this, and a buffered stdout, which is what a pipe gives,
/// would otherwise hold the code until the sign-in finished.
fn show_code(format: Format, code: &DeviceCode) {
    let mut out = std::io::stdout().lock();
    if let Err(err) = write_code(&mut out, format, code) {
        eprintln!("error: could not print the login code: {err}");
    }
}

/// Writes the device code to `out` and flushes it.
///
/// Text writes the link and the code on one line, then Microsoft's own instruction text.
/// JSON writes one object, so it is the first of the two lines `--json` prints.
fn write_code(
    out: &mut dyn std::io::Write,
    format: Format,
    code: &DeviceCode,
) -> std::io::Result<()> {
    match format {
        Format::Json => {
            let row = DeviceCodeRow {
                user_code: &code.user_code,
                verification_uri: &code.verification_uri,
            };
            let line = serde_json::to_string(&row)?;
            writeln!(out, "{line}")?;
        }
        Format::Text => {
            writeln!(
                out,
                "Open {} and enter code {}",
                code.verification_uri, code.user_code
            )?;
            writeln!(out, "{}", code.message)?;
        }
    }
    out.flush()
}

/// The `EXPIRES` cell of a row: the timestamp, or empty for an account with no token.
fn expires_cell(row: &AccountRow) -> &str {
    row.expires.as_deref().unwrap_or_default()
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
            refresh_store: None,
        };
        assert!(AccountRow::new(&account, Some("abc")).active);
        assert!(!AccountRow::new(&account, Some("other")).active);
        assert!(!AccountRow::new(&account, None).active);
    }

    #[test]
    fn an_account_with_no_token_has_an_empty_expires_cell() {
        let mut account = Account {
            id: "abc".to_string(),
            name: "alice".to_string(),
            kind: AccountKind::Offline,
            mc_token: None,
            mc_token_expires: None,
            xuid: None,
            refresh_store: None,
        };
        let row = AccountRow::new(&account, None);
        assert_eq!(expires_cell(&row), "");

        account.kind = AccountKind::Msa;
        account.mc_token_expires = Some("2026-09-06T00:00:00Z".to_string());
        let row = AccountRow::new(&account, None);
        assert_eq!(expires_cell(&row), "2026-09-06T00:00:00Z");
        assert_eq!(row.kind, "msa");
    }

    /// A writer that records what was written and how often it was flushed.
    #[derive(Default)]
    struct Recorder {
        written: Vec<u8>,
        flushes: usize,
    }

    impl std::io::Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    /// A pending sign-in, with a secret device code no output may carry.
    fn device_code() -> DeviceCode {
        DeviceCode {
            user_code: "ABCD-EFGH".to_string(),
            verification_uri: "https://microsoft.com/link".to_string(),
            message: "Sign in at the link.".to_string(),
            interval_secs: 5,
            expires_in_secs: 900,
            device_code: "dev-secret".to_string(),
        }
    }

    #[test]
    fn the_text_code_names_the_link_and_the_code_and_is_flushed() {
        let mut out = Recorder::default();
        write_code(&mut out, Format::Text, &device_code()).expect("write");
        let text = String::from_utf8(out.written).expect("utf-8");
        assert_eq!(
            text,
            "Open https://microsoft.com/link and enter code ABCD-EFGH\nSign in at the link.\n"
        );
        assert_eq!(out.flushes, 1, "the code is flushed before the poll loop");
    }

    #[test]
    fn the_json_code_is_one_line_without_the_secret_and_is_flushed() {
        let mut out = Recorder::default();
        write_code(&mut out, Format::Json, &device_code()).expect("write");
        let text = String::from_utf8(out.written).expect("utf-8");
        assert_eq!(text.lines().count(), 1, "one json line, got {text}");
        assert!(
            !text.contains("dev-secret"),
            "the secret leaked into {text}"
        );
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(parsed["user_code"], "ABCD-EFGH");
        assert_eq!(parsed["verification_uri"], "https://microsoft.com/link");
        assert_eq!(out.flushes, 1, "the code is flushed before the poll loop");
    }
}
