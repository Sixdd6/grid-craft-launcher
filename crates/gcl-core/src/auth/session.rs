//! Signing in with a Microsoft account, and keeping the Minecraft token fresh.
//!
//! [`msa`](super::msa) makes the six requests of the login chain; this module drives them in
//! order and writes the results down. [`login_device_code`] runs the device-code flow from
//! the first code to a stored account, [`complete_chain`] turns Microsoft tokens into an
//! [`Account`], [`refresh_account`] redeems a saved refresh token, and [`ensure_fresh`]
//! refreshes only when the cached Minecraft token is about to expire.
//!
//! Nothing here sleeps or reads the clock on its own behalf: the caller passes a `sleep`
//! function, so a test drives the whole flow with no real waiting.

use std::time::Duration;

use futures_util::future::BoxFuture;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::events::EventSink;

use super::msa::{DeviceCode, Msa, MsaTokens, Poll};
use super::secrets::SecretStore;
use super::store::Accounts;
use super::{Account, AccountKind, Error, token_expires_soon};

/// How much sooner than its expiry a Minecraft token is treated as stale.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

/// Extra wait added to the poll interval when the endpoint answers `slow_down`.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

/// Everything a sign-in needs: the login client, the secret store, the accounts file, and
/// where to report progress.
pub struct LoginCtx<'a> {
    /// The Microsoft login client, already carrying the client id and endpoints.
    pub msa: &'a Msa,
    /// Where the refresh token is kept.
    pub secrets: &'a dyn SecretStore,
    /// The accounts file the finished account is written to.
    pub accounts: &'a Accounts,
    /// Where progress and warnings are sent.
    pub sink: &'a EventSink,
}

impl std::fmt::Debug for LoginCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginCtx")
            .field("msa", &self.msa)
            .field("secrets", &self.secrets.kind())
            .field("accounts", &self.accounts)
            .finish()
    }
}

/// A function the caller supplies to wait between polls.
///
/// The caller decides what waiting means: real time in the launcher, a recorded duration in
/// a test.
pub type SleepFn = dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync;

/// A function called once with the code and link to show the user.
pub type OnCodeFn = dyn Fn(&DeviceCode) + Send + Sync;

/// Signs in with the device-code flow and saves the account.
///
/// Asks for a device code, hands it to `on_code` for display, then polls until the user
/// approves it. `sleep` is awaited for the endpoint's interval before every poll, and the
/// interval grows by five seconds each time the endpoint asks to slow down. When the sleeps
/// add up to the code's lifetime the sign-in fails with [`Error::DeviceCodeExpired`].
///
/// On success the refresh token goes to the secret store and the account to `accounts.json`,
/// where it becomes active if it is the first account. Returns the stored account.
#[tracing::instrument(skip_all)]
pub async fn login_device_code(
    ctx: &LoginCtx<'_>,
    on_code: &OnCodeFn,
    sleep: &SleepFn,
) -> Result<Account, Error> {
    let code = ctx.msa.start_device_code().await?;
    on_code(&code);

    let expires_in = Duration::from_secs(code.expires_in_secs);
    let mut interval = Duration::from_secs(code.interval_secs);
    let mut waited = Duration::ZERO;
    loop {
        sleep(interval).await;
        waited += interval;
        match ctx.msa.poll_device_code_once(&code).await? {
            Poll::Ready(tokens) => {
                let refresh_token = tokens.refresh_token.clone();
                let account = complete_chain(ctx, tokens).await?;
                ctx.secrets.put(&account.id, &refresh_token)?;
                return ctx.accounts.add(account);
            }
            Poll::SlowDown => interval += SLOW_DOWN_STEP,
            Poll::Pending => {}
        }
        if waited >= expires_in {
            return Err(Error::DeviceCodeExpired);
        }
    }
}

/// Turns a pair of Microsoft tokens into an account, running the Xbox and Minecraft steps.
///
/// Runs Xbox Live, XSTS, the Minecraft login, and the profile read, then builds the
/// [`Account`]. Nothing is saved here: the caller stores the refresh token and the account,
/// because only the caller knows whether this is a new sign-in or a refresh.
#[tracing::instrument(skip_all)]
pub async fn complete_chain(ctx: &LoginCtx<'_>, tokens: MsaTokens) -> Result<Account, Error> {
    let xbl = ctx.msa.xbl(&tokens.access_token).await?;
    let xsts = ctx.msa.xsts(&xbl).await?;
    let mc = ctx.msa.mc_login(&xsts).await?;
    let profile = ctx.msa.profile(&mc.access_token).await?;

    let id = uuid::Uuid::parse_str(&profile.id).map_err(|source| Error::Parse {
        what: "profile",
        detail: format!("id is not a uuid: {source}"),
    })?;
    let expires = OffsetDateTime::now_utc() + Duration::from_secs(mc.expires_in_secs);
    Ok(Account {
        id: id.hyphenated().to_string(),
        name: profile.name,
        kind: AccountKind::Msa,
        mc_token: Some(mc.access_token),
        mc_token_expires: Some(format_rfc3339(expires)?),
        xuid: xsts.xuid,
        refresh_store: Some(ctx.secrets.kind()),
    })
}

/// Signs an account in again from its saved refresh token.
///
/// Fails with [`Error::NoRefreshToken`] when the secret store holds nothing for this
/// account, and with [`Error::AccountMismatch`] when the refreshed profile belongs to
/// someone else. The rotated refresh token replaces the old one, and the refreshed account
/// replaces the stored one.
///
/// The rotated token is written as soon as the token endpoint answers, before the Xbox and
/// Minecraft steps run. Microsoft has already invalidated the old token by then, so a later
/// failure in the chain must not throw the new one away: keeping it lets the next attempt
/// sign in again instead of asking the user for a fresh device code.
#[tracing::instrument(skip_all)]
pub async fn refresh_account(ctx: &LoginCtx<'_>, account: &Account) -> Result<Account, Error> {
    let Some(refresh_token) = ctx.secrets.get(&account.id)? else {
        return Err(Error::NoRefreshToken);
    };
    let tokens = ctx.msa.refresh(&refresh_token).await?;
    ctx.secrets.put(&account.id, &tokens.refresh_token)?;
    let fresh = complete_chain(ctx, tokens).await?;
    if fresh.id != account.id {
        return Err(Error::AccountMismatch {
            expected: account.id.clone(),
            actual: fresh.id,
        });
    }
    ctx.accounts.add(fresh)
}

/// Returns an account with a usable Minecraft token, refreshing it only when it is stale.
///
/// An offline account is returned unchanged. A Microsoft account whose token expires within
/// [`REFRESH_MARGIN`] of `now`, or that carries no readable expiry, is refreshed; otherwise
/// it is returned unchanged and no request is made.
#[tracing::instrument(skip_all)]
pub async fn ensure_fresh(
    ctx: &LoginCtx<'_>,
    account: Account,
    now: OffsetDateTime,
) -> Result<Account, Error> {
    if account.kind == AccountKind::Offline {
        return Ok(account);
    }
    if token_expires_soon(account.mc_token_expires.as_deref(), now, REFRESH_MARGIN) {
        return refresh_account(ctx, &account).await;
    }
    Ok(account)
}

/// Formats a timestamp as RFC 3339, reporting a formatting failure as a parse error.
fn format_rfc3339(at: OffsetDateTime) -> Result<String, Error> {
    at.format(&Rfc3339).map_err(|source| Error::Parse {
        what: "token expiry",
        detail: source.to_string(),
    })
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
