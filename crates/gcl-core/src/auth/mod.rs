//! Accounts: offline players and Microsoft accounts, and launch placeholders.
//!
//! See the `msa-auth` skill, "Storage" and "Offline". [`msa`] holds the Microsoft device-code
//! chain, [`secrets`] the refresh-token store, [`store`] the accounts file, and [`offline`]
//! offline players.

pub mod msa;
pub mod offline;
pub mod secrets;
pub mod store;

use serde::{Deserialize, Serialize};

/// Errors loading, saving, or resolving accounts.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An I/O operation on the accounts file failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: std::path::PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The accounts file's contents could not be parsed, or an account could not be
    /// serialized, as JSON.
    #[error("could not parse json at {path}: {source}")]
    Json {
        /// The path being parsed.
        path: std::path::PathBuf,
        /// The underlying JSON error.
        source: serde_json::Error,
    },
    /// No account matches the given id or name.
    #[error("no account matching {0}")]
    NotFound(String),
    /// A launch was asked for with no account named, no offline user, and none active.
    #[error("no account selected: add one, or launch with an offline user name")]
    NoAccount,
    /// The device code was not approved before it expired.
    #[error("the login code expired before it was approved")]
    DeviceCodeExpired,
    /// The user declined the sign-in request.
    #[error("the sign-in request was declined")]
    DeviceCodeDeclined,
    /// The OAuth endpoint reported a failure. Carries the error code only, never the
    /// description, which can quote a token.
    #[error("Microsoft login failed: {0}")]
    Oauth(String),
    /// The Xbox Live authentication step failed.
    #[error("Xbox Live: {0}")]
    Xbl(String),
    /// XSTS reported that this Microsoft account has no Xbox profile.
    #[error("this Microsoft account has no Xbox profile: create one at xbox.com, then retry")]
    NoXboxProfile,
    /// XSTS reported that Xbox Live is not available in this region.
    #[error("Xbox Live is not available in this region")]
    XboxRegionUnavailable,
    /// XSTS reported that the account needs adult verification.
    #[error("this account needs adult verification before it can sign in")]
    XboxAdultVerification,
    /// XSTS reported a child account that is not in a family group.
    #[error("this is a child account: an adult must add it to their family group")]
    XboxChildAccount,
    /// XSTS failed with a code this launcher has no message for.
    #[error("Xbox XSTS error {code}")]
    Xsts {
        /// The `XErr` code XSTS reported, or 0 when the body carried none.
        code: u64,
    },
    /// The account is signed in but owns no Minecraft profile.
    #[error("this account owns no Minecraft profile")]
    NoProfile,
    /// The Minecraft services API rejected this launcher's app registration.
    #[error("this launcher's Microsoft app registration is not approved for Minecraft")]
    InvalidAppRegistration,
    /// A request in the login chain failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// There is no usable OS keyring on this machine.
    #[error("no OS keyring available: {0}")]
    KeyringUnavailable(String),
    /// The OS keyring rejected a read or a write. Carries its message only.
    #[error("keyring error: {0}")]
    Keyring(String),
    /// A response in the login chain was not the JSON shape expected.
    #[error("could not read the {what} response: {detail}")]
    Parse {
        /// Which step's response failed to parse.
        what: &'static str,
        /// What was wrong. Never contains a token.
        detail: String,
    },
}

/// How an account authenticates: a local offline player, or a Microsoft account.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    /// A local offline player with no real authentication.
    Offline,
    /// A Microsoft account, authenticated through the Xbox/Minecraft services chain.
    Msa,
}

/// One saved account: an offline player or a Microsoft account.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    /// The account's UUID, dashed and lowercase.
    pub id: String,
    /// The player's display name.
    pub name: String,
    /// Which kind of account this is.
    pub kind: AccountKind,
    /// The short-lived Minecraft services token, when signed in.
    #[serde(default)]
    pub mc_token: Option<String>,
    /// RFC 3339 expiry timestamp for `mc_token`.
    #[serde(default)]
    pub mc_token_expires: Option<String>,
    /// The Xbox user id, for a Microsoft account.
    #[serde(default)]
    pub xuid: Option<String>,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field(
                "mc_token",
                if self.mc_token.is_some() {
                    &"<set>"
                } else {
                    &"<unset>"
                },
            )
            .field("mc_token_expires", &self.mc_token_expires)
            .field("xuid", &self.xuid)
            .finish()
    }
}

/// Placeholders substituted into a launch command line for the signed-in player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchIdentity {
    /// The player's display name.
    pub name: String,
    /// The account UUID with no dashes.
    pub uuid_undashed: String,
    /// The access token to launch with. `"0"` for offline play.
    pub access_token: String,
    /// `"legacy"` for offline play, `"msa"` for a Microsoft account.
    pub user_type: String,
    /// The Xbox user id, empty for offline play.
    pub xuid: String,
    /// Base64 of the MSA client id, empty for offline play.
    pub client_id: String,
}

impl Account {
    /// Builds the launch placeholders for this account.
    ///
    /// Offline accounts get the fixed placeholders from the `msa-auth` skill: token `"0"`,
    /// user type `"legacy"`, and empty xuid and client id. A Microsoft account's fields are
    /// filled in from its stored token and xuid (the client id is filled in by the caller
    /// that holds it, in a later plan).
    pub fn launch_identity(&self) -> LaunchIdentity {
        let uuid_undashed = self.id.replace('-', "");
        match self.kind {
            AccountKind::Offline => LaunchIdentity {
                name: self.name.clone(),
                uuid_undashed,
                access_token: "0".to_string(),
                user_type: "legacy".to_string(),
                xuid: String::new(),
                client_id: String::new(),
            },
            AccountKind::Msa => LaunchIdentity {
                name: self.name.clone(),
                uuid_undashed,
                access_token: self.mc_token.clone().unwrap_or_default(),
                user_type: "msa".to_string(),
                xuid: self.xuid.clone().unwrap_or_default(),
                client_id: String::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline_account(id: &str, name: &str) -> Account {
        Account {
            id: id.to_string(),
            name: name.to_string(),
            kind: AccountKind::Offline,
            mc_token: None,
            mc_token_expires: None,
            xuid: None,
        }
    }

    #[test]
    fn debug_redacts_a_set_token() {
        let account = Account {
            mc_token: Some("secret-token".to_string()),
            ..offline_account("b50ad385-829d-3141-a216-7e7d7539ba7f", "Notch")
        };
        let debug = format!("{account:?}");
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("<set>"));
    }

    #[test]
    fn debug_shows_unset_when_there_is_no_token() {
        let account = offline_account("b50ad385-829d-3141-a216-7e7d7539ba7f", "Notch");
        let debug = format!("{account:?}");
        assert!(debug.contains("<unset>"));
    }

    #[test]
    fn offline_launch_identity_uses_the_fixed_placeholders() {
        let account = offline_account("b50ad385-829d-3141-a216-7e7d7539ba7f", "Notch");
        let identity = account.launch_identity();
        assert_eq!(identity.name, "Notch");
        assert_eq!(identity.uuid_undashed, "b50ad385829d3141a2167e7d7539ba7f");
        assert_eq!(identity.access_token, "0");
        assert_eq!(identity.user_type, "legacy");
        assert_eq!(identity.xuid, "");
        assert_eq!(identity.client_id, "");
    }
}
