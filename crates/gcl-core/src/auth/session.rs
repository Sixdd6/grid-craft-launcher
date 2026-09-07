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
use tokio_util::sync::CancellationToken;

use crate::events::{Event, EventSink, LogLevel};

use super::msa::{DeviceCode, Msa, MsaTokens, Poll};
use super::secrets::SecretStore;
use super::store::Accounts;
use super::{Account, AccountKind, Error, token_expires_soon};

/// The OAuth error code Microsoft returns for a refresh token it no longer accepts.
const INVALID_GRANT: &str = "invalid_grant";

/// How much sooner than its expiry a Minecraft token is treated as stale.
pub const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

/// Extra wait added to the poll interval when the endpoint answers `slow_down`.
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

/// Shortest wait between polls, whatever interval the endpoint asks for.
///
/// An `interval` of zero would busy-poll the token endpoint, which Microsoft answers with
/// `slow_down` at best and a block at worst.
const MIN_POLL_INTERVAL: Duration = Duration::from_secs(1);

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
    /// Cancels a sign-in that is still waiting for the user to approve the code.
    pub cancel: &'a CancellationToken,
}

impl LoginCtx<'_> {
    /// Reports one step of the sign-in on the event sink.
    ///
    /// Never carries a token, a device code, or the client id: the messages name the step
    /// reached, and nothing else. A closed sink is not an error, so progress reporting never
    /// fails a sign-in.
    fn step(&self, message: impl Into<String>) {
        let _ = self.sink.send(Event::Log {
            level: LogLevel::Info,
            message: message.into(),
        });
    }
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
/// approves it. `sleep` is awaited for the endpoint's interval before every poll, never for
/// less than one second, and the interval grows by five seconds each time the
/// endpoint asks to slow down. When the sleeps add up to the code's lifetime the sign-in
/// fails with [`Error::DeviceCodeExpired`]. Cancelling `ctx.cancel` while the loop waits
/// ends the sign-in with [`Error::Cancelled`], with no further poll.
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
    ctx.step("waiting for the code to be entered");

    let expires_in = Duration::from_secs(code.expires_in_secs);
    let mut interval = Duration::from_secs(code.interval_secs).max(MIN_POLL_INTERVAL);
    let mut waited = Duration::ZERO;
    loop {
        tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Err(Error::Cancelled),
            () = sleep(interval) => {}
        }
        waited += interval;
        match ctx.msa.poll_device_code_once(&code).await? {
            Poll::Ready(tokens) => {
                ctx.step("signed in to Microsoft");
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
    ctx.step("signed in to Xbox Live");
    let xsts = ctx.msa.xsts(&xbl).await?;
    ctx.step("authorized for Minecraft");
    let mc = ctx.msa.mc_login(&xsts).await?;
    ctx.step("signed in to Minecraft");
    let profile = ctx.msa.profile(&mc.access_token).await?;
    ctx.step(format!("loaded profile {}", profile.name));

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
/// account, with [`Error::SignInAgain`] when Microsoft has revoked the saved token, and
/// with [`Error::AccountMismatch`] when the refreshed profile belongs to someone else. The
/// rotated refresh token replaces the old one, and the refreshed account replaces the
/// stored one.
///
/// Microsoft invalidates the old token as soon as the token endpoint answers, so the
/// rotated one is written even when a later step of the chain fails: keeping it lets the
/// next attempt sign in again instead of asking the user for a fresh device code. The one
/// case that stores nothing is [`Error::AccountMismatch`], where the rotated token belongs
/// to a different account than the one being refreshed. A revoked token is deleted instead,
/// so the account stops retrying with something that can never work again.
#[tracing::instrument(skip_all)]
pub async fn refresh_account(ctx: &LoginCtx<'_>, account: &Account) -> Result<Account, Error> {
    let Some(refresh_token) = ctx.secrets.get(&account.id)? else {
        return Err(Error::NoRefreshToken);
    };
    ctx.step("refreshing sign-in");
    let tokens = match ctx.msa.refresh(&refresh_token).await {
        Ok(tokens) => tokens,
        Err(Error::Oauth(code)) if code == INVALID_GRANT => {
            ctx.secrets.delete(&account.id)?;
            return Err(Error::SignInAgain);
        }
        Err(err) => return Err(err),
    };
    ctx.step("signed in to Microsoft");
    let rotated = tokens.refresh_token.clone();
    let fresh = match complete_chain(ctx, tokens).await {
        Ok(fresh) => fresh,
        Err(err) => {
            // The old token is spent. Keep the rotated one so the next attempt can retry.
            ctx.secrets.put(&account.id, &rotated)?;
            return Err(err);
        }
    };
    if fresh.id != account.id {
        return Err(Error::AccountMismatch {
            expected: account.id.clone(),
            actual: fresh.id,
        });
    }
    ctx.secrets.put(&account.id, &rotated)?;
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
