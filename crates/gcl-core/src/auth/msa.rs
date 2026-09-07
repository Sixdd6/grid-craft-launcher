//! The Microsoft account device-code login chain: device code, OAuth token, Xbox Live,
//! XSTS, Minecraft services, and the player profile.
//!
//! The steps and the `XErr` table live in the `msa-auth` skill. This module builds the
//! requests, maps the answers onto the types below, and turns every failure into
//! [`Error`].
//!
//! Everything here is a secret. No function takes a `#[tracing::instrument]` field: they
//! all use `skip_all`, so no token, device code, or client id ever reaches a span. The
//! [`std::fmt::Debug`] impls of [`Msa`], [`DeviceCode`], [`MsaTokens`], [`XblToken`],
//! [`XstsToken`], and [`McToken`] redact the same values.

use serde::{Deserialize, Serialize};

use crate::http::HttpClient;

use super::Error;

/// Production device-code endpoint.
pub const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";

/// Production OAuth token endpoint.
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";

/// Production Xbox Live user authentication endpoint.
pub const XBL_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";

/// Production XSTS authorization endpoint.
pub const XSTS_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";

/// Production Minecraft services Xbox login endpoint.
pub const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";

/// Production Minecraft services profile endpoint.
pub const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// The only scope this launcher asks for.
pub const SCOPE: &str = "XboxLive.signin offline_access";

/// The device-code grant type, sent when polling the token endpoint.
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Relying party for the Xbox Live user token.
const XBL_RELYING_PARTY: &str = "http://auth.xboxlive.com";

/// Relying party for the XSTS token Minecraft services accepts.
const MC_RELYING_PARTY: &str = "rp://api.minecraftservices.com/";

/// Sent on every JSON step of the chain.
const ACCEPT_JSON: (&str, &str) = ("accept", "application/json");

/// Every base URL the login chain uses. [`Default`] is the production set.
///
/// Tests build one pointing at a mock server; nothing else overrides it.
#[derive(Clone, Debug)]
pub struct MsaEndpoints {
    /// Where the device code is requested.
    pub device_code: String,
    /// Where the device code is exchanged for tokens, and refresh tokens redeemed.
    pub token: String,
    /// Xbox Live user authentication.
    pub xbl: String,
    /// XSTS authorization.
    pub xsts: String,
    /// Minecraft services Xbox login.
    pub mc_login: String,
    /// Minecraft services player profile.
    pub profile: String,
}

impl Default for MsaEndpoints {
    fn default() -> Self {
        MsaEndpoints {
            device_code: DEVICE_CODE_URL.to_string(),
            token: TOKEN_URL.to_string(),
            xbl: XBL_URL.to_string(),
            xsts: XSTS_URL.to_string(),
            mc_login: MC_LOGIN_URL.to_string(),
            profile: PROFILE_URL.to_string(),
        }
    }
}

/// A pending device-code sign-in: what to show the user, and the secret to poll with.
#[derive(Clone, Serialize)]
pub struct DeviceCode {
    /// The short code the user types on the verification page.
    pub user_code: String,
    /// The page the user opens to enter the code.
    pub verification_uri: String,
    /// Microsoft's own instruction text, safe to show as-is.
    pub message: String,
    /// Seconds to wait between polls.
    pub interval_secs: u64,
    /// Seconds until the code stops working.
    pub expires_in_secs: u64,
    /// The secret this launcher polls with. Never shown, never serialized.
    #[serde(skip)]
    pub device_code: String,
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("message", &self.message)
            .field("interval_secs", &self.interval_secs)
            .field("expires_in_secs", &self.expires_in_secs)
            .field("device_code", &format_args!("<secret>"))
            .finish()
    }
}

/// The Microsoft OAuth tokens: one short-lived access token and one long-lived refresh token.
#[derive(Clone)]
pub struct MsaTokens {
    /// The access token, used only for the Xbox Live step. Never stored.
    pub access_token: String,
    /// The refresh token, stored in the secret store.
    pub refresh_token: String,
}

impl std::fmt::Debug for MsaTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsaTokens")
            .field("access_token", &format_args!("<secret>"))
            .field("refresh_token", &format_args!("<secret>"))
            .finish()
    }
}

/// An Xbox Live user token and its user hash.
#[derive(Clone)]
pub struct XblToken {
    /// The Xbox Live token.
    pub token: String,
    /// The user hash from `DisplayClaims.xui[0].uhs`.
    pub uhs: String,
}

impl std::fmt::Debug for XblToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XblToken")
            .field("token", &format_args!("<secret>"))
            .field("uhs", &self.uhs)
            .finish()
    }
}

/// An XSTS token for Minecraft services, its user hash, and the Xbox user id.
#[derive(Clone)]
pub struct XstsToken {
    /// The XSTS token.
    pub token: String,
    /// The user hash from `DisplayClaims.xui[0].uhs`.
    pub uhs: String,
    /// The Xbox user id from `DisplayClaims.xui[0].xid`, when XSTS returns one.
    pub xuid: Option<String>,
}

impl std::fmt::Debug for XstsToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XstsToken")
            .field("token", &format_args!("<secret>"))
            .field("uhs", &self.uhs)
            .field("xuid", &self.xuid)
            .finish()
    }
}

/// The Minecraft services access token and how long it lasts.
#[derive(Clone)]
pub struct McToken {
    /// The token launch uses as `${auth_access_token}`.
    pub access_token: String,
    /// Seconds until the token expires, as reported at login.
    pub expires_in_secs: u64,
}

impl std::fmt::Debug for McToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McToken")
            .field("access_token", &format_args!("<secret>"))
            .field("expires_in_secs", &self.expires_in_secs)
            .finish()
    }
}

/// The player's Minecraft profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McProfile {
    /// The player UUID, without dashes, as Minecraft services returns it.
    pub id: String,
    /// The player's display name.
    pub name: String,
}

/// What one poll of the device-code flow found.
#[derive(Debug)]
pub enum Poll {
    /// The user has not finished signing in yet. Wait and poll again.
    Pending,
    /// The endpoint asked for a longer wait between polls.
    SlowDown,
    /// Sign-in finished; here are the tokens.
    Ready(MsaTokens),
}

/// Client for the Microsoft login chain.
///
/// One instance per launcher. Every base URL comes from its [`MsaEndpoints`].
pub struct Msa {
    http: HttpClient,
    ep: MsaEndpoints,
    client_id: String,
}

impl std::fmt::Debug for Msa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Msa")
            .field("ep", &self.ep)
            .field("client_id", &format_args!("<set>"))
            .finish()
    }
}

impl Msa {
    /// Builds a client for `client_id` against the given endpoints.
    pub fn new(http: HttpClient, ep: MsaEndpoints, client_id: String) -> Self {
        Msa {
            http,
            ep,
            client_id,
        }
    }

    /// Asks Microsoft for a device code to show the user.
    #[tracing::instrument(skip_all)]
    pub async fn start_device_code(&self) -> Result<DeviceCode, Error> {
        let body: DeviceCodeResponse = self
            .http
            .post_form(
                &self.ep.device_code,
                &[("client_id", &self.client_id), ("scope", SCOPE)],
            )
            .await?;
        Ok(DeviceCode {
            user_code: body.user_code,
            verification_uri: body.verification_uri,
            message: body.message,
            interval_secs: body.interval,
            expires_in_secs: body.expires_in,
            device_code: body.device_code,
        })
    }

    /// Polls the token endpoint once for a device code that is waiting for approval.
    ///
    /// The caller sleeps for `code.interval_secs` between calls, and longer after
    /// [`Poll::SlowDown`].
    #[tracing::instrument(skip_all)]
    pub async fn poll_device_code_once(&self, code: &DeviceCode) -> Result<Poll, Error> {
        let (status, body) = self
            .http
            .post_form_raw(
                &self.ep.token,
                &[
                    ("grant_type", DEVICE_CODE_GRANT),
                    ("client_id", &self.client_id),
                    ("device_code", &code.device_code),
                ],
            )
            .await?;
        if status == 200 {
            return Ok(Poll::Ready(parse_tokens(&body)?));
        }
        match oauth_error_code(status, &body).as_str() {
            "authorization_pending" => Ok(Poll::Pending),
            "slow_down" => Ok(Poll::SlowDown),
            "expired_token" => Err(Error::DeviceCodeExpired),
            "authorization_declined" => Err(Error::DeviceCodeDeclined),
            other => Err(Error::Oauth(other.to_string())),
        }
    }

    /// Redeems a refresh token for a fresh pair of Microsoft tokens.
    #[tracing::instrument(skip_all)]
    pub async fn refresh(&self, refresh_token: &str) -> Result<MsaTokens, Error> {
        let (status, body) = self
            .http
            .post_form_raw(
                &self.ep.token,
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", &self.client_id),
                    ("refresh_token", refresh_token),
                    ("scope", SCOPE),
                ],
            )
            .await?;
        if status != 200 {
            return Err(Error::Oauth(oauth_error_code(status, &body)));
        }
        parse_tokens(&body)
    }

    /// Exchanges a Microsoft access token for an Xbox Live user token.
    #[tracing::instrument(skip_all)]
    pub async fn xbl(&self, access_token: &str) -> Result<XblToken, Error> {
        let request = serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={access_token}"),
            },
            "RelyingParty": XBL_RELYING_PARTY,
            "TokenType": "JWT",
        });
        let (status, body) = self
            .http
            .post_json_raw_with_headers(&self.ep.xbl, &request, &[ACCEPT_JSON])
            .await?;
        if status != 200 {
            return Err(Error::Xbl(format!("HTTP {status}")));
        }
        let parsed: XboxResponse = parse(&body, "Xbox Live")?;
        let claim = parsed.first_claim("Xbox Live")?;
        Ok(XblToken {
            token: parsed.token,
            uhs: claim.uhs,
        })
    }

    /// Exchanges an Xbox Live token for an XSTS token for Minecraft services.
    ///
    /// A 401 carries an `XErr` code, mapped to the matching account error.
    #[tracing::instrument(skip_all)]
    pub async fn xsts(&self, xbl: &XblToken) -> Result<XstsToken, Error> {
        let request = serde_json::json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [xbl.token],
            },
            "RelyingParty": MC_RELYING_PARTY,
            "TokenType": "JWT",
        });
        let (status, body) = self
            .http
            .post_json_raw_with_headers(&self.ep.xsts, &request, &[ACCEPT_JSON])
            .await?;
        if status == 401 {
            let code = serde_json::from_slice::<XstsError>(&body)
                .ok()
                .and_then(|e| e.xerr)
                .unwrap_or(0);
            return Err(xsts_error(code));
        }
        if status != 200 {
            return Err(Error::Xbl(format!("HTTP {status}")));
        }
        let parsed: XboxResponse = parse(&body, "XSTS")?;
        let claim = parsed.first_claim("XSTS")?;
        Ok(XstsToken {
            token: parsed.token,
            uhs: claim.uhs,
            xuid: claim.xid,
        })
    }

    /// Exchanges an XSTS token for a Minecraft services access token.
    #[tracing::instrument(skip_all)]
    pub async fn mc_login(&self, xsts: &XstsToken) -> Result<McToken, Error> {
        let request = serde_json::json!({
            "identityToken": format!("XBL3.0 x={};{}", xsts.uhs, xsts.token),
        });
        let body: McTokenResponse = self
            .http
            .post_json_with_headers(&self.ep.mc_login, &request, &[ACCEPT_JSON])
            .await
            .map_err(|err| match err {
                crate::http::Error::Status { status: 403, .. } => Error::InvalidAppRegistration,
                other => Error::Http(other),
            })?;
        Ok(McToken {
            access_token: body.access_token,
            expires_in_secs: body.expires_in,
        })
    }

    /// Reads the player's Minecraft profile. HTTP 404 means the account owns no copy.
    #[tracing::instrument(skip_all)]
    pub async fn profile(&self, mc_token: &str) -> Result<McProfile, Error> {
        let auth = format!("Bearer {mc_token}");
        let body: ProfileResponse = self
            .http
            .get_json_with_headers(
                &self.ep.profile,
                &[ACCEPT_JSON, ("authorization", auth.as_str())],
            )
            .await
            .map_err(|err| match err {
                crate::http::Error::Status { status: 404, .. } => Error::NoProfile,
                other => Error::Http(other),
            })?;
        Ok(McProfile {
            id: body.id,
            name: body.name,
        })
    }
}

/// Maps an XSTS `XErr` code onto the error the user sees.
fn xsts_error(code: u64) -> Error {
    match code {
        2148916233 => Error::NoXboxProfile,
        2148916235 => Error::XboxRegionUnavailable,
        2148916236 | 2148916237 => Error::XboxAdultVerification,
        2148916238 => Error::XboxChildAccount,
        code => Error::Xsts { code },
    }
}

/// Reads the OAuth `error` code out of a failed token response.
///
/// The `error_description` is never read: it can quote the request, tokens included.
/// A body with no `error` field falls back to the status code.
fn oauth_error_code(status: u16, body: &[u8]) -> String {
    serde_json::from_slice::<OauthError>(body)
        .ok()
        .map(|e| e.error)
        .unwrap_or_else(|| format!("HTTP {status}"))
}

/// Parses a token response, keeping only the two tokens.
fn parse_tokens(body: &[u8]) -> Result<MsaTokens, Error> {
    let parsed: TokenResponse = parse(body, "token")?;
    Ok(MsaTokens {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
    })
}

/// Parses a response body, reporting which step failed without quoting the body.
fn parse<T: serde::de::DeserializeOwned>(body: &[u8], what: &'static str) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|source| Error::Parse {
        what,
        detail: source.to_string(),
    })
}

/// `POST /devicecode` response.
#[derive(Deserialize)]
struct DeviceCodeResponse {
    user_code: String,
    device_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
    #[serde(default)]
    message: String,
}

/// A successful `POST /token` response.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
}

/// A failed `POST /token` response. `error_description` is deliberately not read.
#[derive(Deserialize)]
struct OauthError {
    error: String,
}

/// The shape both Xbox Live and XSTS answer with.
#[derive(Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims", default)]
    display_claims: Option<DisplayClaims>,
}

impl XboxResponse {
    /// Takes the first `xui` claim, or reports which step returned none.
    fn first_claim(&self, what: &'static str) -> Result<Xui, Error> {
        self.display_claims
            .as_ref()
            .and_then(|c| c.xui.first())
            .cloned()
            .ok_or(Error::Parse {
                what,
                detail: "no DisplayClaims.xui entry".to_string(),
            })
    }
}

/// `DisplayClaims` in an Xbox Live or XSTS response.
#[derive(Deserialize)]
struct DisplayClaims {
    #[serde(default)]
    xui: Vec<Xui>,
}

/// One `xui` claim: the user hash, and the Xbox user id when XSTS sends it.
#[derive(Deserialize, Clone)]
struct Xui {
    uhs: String,
    #[serde(default)]
    xid: Option<String>,
}

/// An XSTS 401 body.
#[derive(Deserialize)]
struct XstsError {
    #[serde(rename = "XErr")]
    xerr: Option<u64>,
}

/// `POST /authentication/login_with_xbox` response.
#[derive(Deserialize)]
struct McTokenResponse {
    access_token: String,
    expires_in: u64,
}

/// `GET /minecraft/profile` response.
#[derive(Deserialize)]
struct ProfileResponse {
    id: String,
    name: String,
}

#[cfg(test)]
#[path = "msa_tests.rs"]
mod tests;
