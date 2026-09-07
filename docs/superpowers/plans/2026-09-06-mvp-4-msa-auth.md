# MVP Plan 4: Microsoft Account Login Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sign in with a Microsoft account through the device-code flow, keep the refresh token in the OS keyring (file fallback with a warning), refresh Minecraft tokens before launch, and launch online with the real identity. Offline mode stays the fallback when login or refresh fails.

**Architecture:** `auth::msa` is a pure HTTP client for the six-step chain with every base URL injectable (wiremock in tests). `auth::secrets` hides token storage behind a `SecretStore` trait (`KeyringStore`, `FileStore`, `MemoryStore`). `auth::session` orchestrates login and refresh and produces an `Account { kind: Msa }`. `Launcher` exposes start/wait/refresh and refreshes before `launch_instance`. The CLI adds `account add-msa` and `account refresh`.

**Tech Stack:** as before, plus `keyring` 4 (default features: native stores), `base64` 0.23 (not needed unless a placeholder needs it; see rulings), `time` for expiry math.

**Spec:** `docs/SPEC.md` R6.1, R6.3, R6.4, R6.5, R11.1 (identity placeholders), R12.1 (account); skill `msa-auth`; research `docs/research/2026-09-06-msa-auth-and-rust-crates.md` section A.

## Global Constraints

- Everything from plans 1-3 (thiserror/anyhow split, no unwrap outside tests, workspace deps, `just check` per task, commit per task with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`, no network in tests, `write_atomic` for root files, secrets never logged).
- `GCL_MSA_CLIENT_ID` from `Config::msa_client_id()` (env first, then config; blank = unset). Absent → Microsoft login is unavailable: `Launcher::msa_available() == false`, CLI prints `microsoft login: disabled (no GCL_MSA_CLIENT_ID)`, offline mode unaffected (R6.5).
- Tokens: the MSA refresh token goes to the `SecretStore` (keyring service `grid-craft-launcher`, user = account uuid). The Minecraft access token and its expiry, plus `xuid`, live in `accounts.json` (already modeled). The MSA access token is never stored. No token or code appears in logs, `Debug`, errors, or `--json` output except the device `user_code` the user must type.
- Endpoints (all injectable via an `MsaEndpoints` struct; defaults are production):
  - device code: `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode` (form: `client_id`, `scope=XboxLive.signin offline_access`)
  - token: `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/token` (form; `grant_type=urn:ietf:params:oauth:grant-type:device_code` + `device_code` + `client_id`, or `grant_type=refresh_token` + `refresh_token` + `client_id` + `scope`)
  - XBL: `POST https://user.auth.xboxlive.com/user/authenticate` JSON `{ Properties: { AuthMethod: "RPS", SiteName: "user.auth.xboxlive.com", RpsTicket: "d=<access_token>" }, RelyingParty: "http://auth.xboxlive.com", TokenType: "JWT" }` → `Token`, `DisplayClaims.xui[0].uhs`
  - XSTS: `POST https://xsts.auth.xboxlive.com/xsts/authorize` JSON `{ Properties: { SandboxId: "RETAIL", UserTokens: [xbl] }, RelyingParty: "rp://api.minecraftservices.com/", TokenType: "JWT" }` → `Token`, `DisplayClaims.xui[0].{uhs,xid}`; HTTP 401 body `{ XErr }` → typed error
  - MC login: `POST https://api.minecraftservices.com/authentication/login_with_xbox` JSON `{ identityToken: "XBL3.0 x=<uhs>;<xsts>" }` → `access_token`, `expires_in`
  - profile: `GET https://api.minecraftservices.com/minecraft/profile` Bearer → `id` (undashed uuid), `name`; 404 → `NoProfile`
- XErr mapping: 2148916233 no Xbox profile; 2148916235 region unavailable; 2148916236/2148916237 adult verification; 2148916238 child account. Others → `Xsts { code }`.
- Refresh when `mc_token_expires` is absent or within 5 minutes of now; refresh runs the token endpoint with the stored refresh token, then steps XBL → XSTS → MC login → profile, and stores the rotated refresh token.
- `LaunchIdentity` for `Msa`: `access_token = mc_token`, `user_type = "msa"`, `xuid`, `client_id` = the Azure client id string, `uuid_undashed = id without dashes`, `name`.
- Device-code polling honors `interval` (seconds) and `expires_in`; `authorization_pending` and `slow_down` (add 5 s) continue; `expired_token` and `authorization_declined` fail.
- Live verification is impossible without a client id; `debug verify-source msa` prints `SKIP msa (no GCL_MSA_CLIENT_ID)`; the plan's tests are wiremock-only.

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `crates/gcl-core/src/auth/msa.rs` | endpoints struct, device code start/poll, refresh grant, XBL, XSTS, MC login, profile, typed errors | 1 |
| `crates/gcl-core/src/auth/secrets.rs` | `SecretStore` trait, `KeyringStore`, `FileStore`, `MemoryStore` | 2 |
| `crates/gcl-core/src/auth/session.rs`, modify `auth/mod.rs`, `auth/store.rs` | login orchestration, refresh, identity for Msa, `Account` fields | 3 |
| `crates/gcl-core/src/launcher/mod.rs` | `msa_available`, `msa_login_start/wait`, `msa_refresh`, refresh before launch | 4 |
| `crates/gcl-cli/src/commands/{account,debug,launch}.rs` | `add-msa`, `refresh`, list columns, verify-source msa | 5 |
| docs | ARCHITECTURE.md, `msa-auth` skill, SPEC notes, README | 6 |

---

### Task 1: msa client

**Files:** `crates/gcl-core/src/auth/msa.rs`

**Interfaces:**
```rust
#[derive(Clone, Debug)] pub struct MsaEndpoints { pub device_code: String, pub token: String, pub xbl: String, pub xsts: String, pub mc_login: String, pub profile: String }  // Default = production URLs above
#[derive(Clone, Serialize)] pub struct DeviceCode { pub user_code: String, pub verification_uri: String, pub message: String, pub interval_secs: u64, pub expires_in_secs: u64, #[serde(skip)] pub device_code: String }
// Debug for DeviceCode prints device_code as <secret>
pub struct MsaTokens { pub access_token: String, pub refresh_token: String }   // Debug redacts both
pub struct XblToken { pub token: String, pub uhs: String }
pub struct XstsToken { pub token: String, pub uhs: String, pub xuid: Option<String> }
pub struct McToken { pub access_token: String, pub expires_in_secs: u64 }
pub struct McProfile { pub id: String /* undashed */, pub name: String }
pub struct Msa { http: HttpClient, ep: MsaEndpoints, client_id: String }
impl Msa {
    pub fn new(http: HttpClient, ep: MsaEndpoints, client_id: String) -> Self;
    pub async fn start_device_code(&self) -> Result<DeviceCode, Error>;
    pub async fn poll_device_code_once(&self, code: &DeviceCode) -> Result<Poll, Error>;   // Poll::Pending | Poll::SlowDown | Poll::Ready(MsaTokens)
    pub async fn refresh(&self, refresh_token: &str) -> Result<MsaTokens, Error>;
    pub async fn xbl(&self, access_token: &str) -> Result<XblToken, Error>;
    pub async fn xsts(&self, xbl: &XblToken) -> Result<XstsToken, Error>;
    pub async fn mc_login(&self, xsts: &XstsToken) -> Result<McToken, Error>;
    pub async fn profile(&self, mc_token: &str) -> Result<McProfile, Error>;
}
pub enum Poll { Pending, SlowDown, Ready(MsaTokens) }
// errors (add to auth::Error): DeviceCodeExpired, DeviceCodeDeclined, #[error("Microsoft login failed: {0}")] Oauth(String /* error code only, never the description with tokens */), #[error("Xbox Live: {0}")] Xbl(String), NoXboxProfile, XboxRegionUnavailable, XboxAdultVerification, XboxChildAccount, #[error("Xbox XSTS error {code}")] Xsts { code: u64 }, #[error("this account owns no Minecraft profile")] NoProfile, InvalidAppRegistration (HTTP 403 from mc_login), Http(#[from] http::Error), Json { what: &'static str, detail: String }
```
The token endpoint needs form encoding: add `HttpClient::post_form<T>(url, &[(&str, &str)])` (same retry policy; never retry a 400 — OAuth uses 400 for `authorization_pending`, so `poll_device_code_once` must read the JSON body of a 400 response: add `HttpClient::post_form_raw(url, form) -> (u16 status, bytes)` that does not treat 4xx as an error).

- [ ] **Step 1: Tests** (wiremock, `#[cfg(test)]`): `start_device_code` parses `{ user_code, device_code, verification_uri, expires_in, interval, message }` and the form carries `client_id` and the exact scope; `poll_device_code_once` maps 400 `{ "error": "authorization_pending" }` → `Pending`, `slow_down` → `SlowDown`, `expired_token` → `DeviceCodeExpired`, `authorization_declined` → `DeviceCodeDeclined`, 200 → `Ready` with both tokens; `refresh` sends `grant_type=refresh_token`; `xbl`/`xsts` build the documented JSON bodies (matcher `body_partial_json`) and parse `uhs`/`xid`; XSTS 401 with `XErr: 2148916233` → `NoXboxProfile`, unknown code → `Xsts { code }`; `mc_login` builds `XBL3.0 x=<uhs>;<xsts>`; 403 → `InvalidAppRegistration`; `profile` 404 → `NoProfile`; `Debug` of `DeviceCode`/`MsaTokens` hides secrets.
- [ ] **Step 2: Implement, commit** `feat(core): microsoft auth chain client`

---

### Task 2: secret store

**Files:** `crates/gcl-core/src/auth/secrets.rs`; `Cargo.toml` (`keyring = "4"` default features)

**Interfaces:**
```rust
pub trait SecretStore: Send + Sync {
    fn put(&self, account_id: &str, refresh_token: &str) -> Result<(), Error>;
    fn get(&self, account_id: &str) -> Result<Option<String>, Error>;
    fn delete(&self, account_id: &str) -> Result<(), Error>;
    fn kind(&self) -> SecretStoreKind;   // Keyring | File | Memory
}
pub const SERVICE: &str = "grid-craft-launcher";
pub struct KeyringStore;               // keyring::Entry::new(SERVICE, account_id); NoEntry → Ok(None)
impl KeyringStore { pub fn probe() -> Result<KeyringStore, Error>; }  // once: keyring::use_native_store(true) if the crate requires it, then a round-trip set/get/delete on a probe entry `__probe__`; failure → Error::KeyringUnavailable(reason)
pub struct FileStore { path: PathBuf /* <root>/secrets.json, mode 0600 on unix */ }  // JSON map id → token via write_atomic; set permissions after write
pub struct MemoryStore(Mutex<HashMap<String,String>>);
pub fn open_default(root: &Root, sink: &EventSink) -> Box<dyn SecretStore>;  // KeyringStore::probe() else FileStore with one Event::Warning("no OS keyring available; refresh tokens are stored in <path> with restricted permissions")
```
Errors: `KeyringUnavailable(String)`, `Keyring(String)` (message only), `Io { path, source }`, `Json { path, source }`.

- [ ] **Step 1: Tests**: `MemoryStore` round-trip; `FileStore` round-trip in a tempdir, file mode 0600 on unix, `get` of unknown → `None`, `delete` idempotent; `open_default` falls back to `FileStore` when `KeyringStore::probe` fails (test by an env `GCL_NO_KEYRING=1` respected in debug builds only, like the base-URL overrides) and emits exactly one Warning. Do not touch the real keyring in tests (`GCL_NO_KEYRING` set in every test that calls `open_default`).
- [ ] **Step 2: Implement, commit** `feat(core): secret store with keyring and file fallback`

---

### Task 3: session orchestration and Account changes

**Files:** `crates/gcl-core/src/auth/session.rs`, modify `auth/mod.rs`, `auth/store.rs`

**Interfaces:**
```rust
// mod.rs additions
impl Account { /* existing */ }  // add field `#[serde(default, skip_serializing_if = "Option::is_none")] pub refresh_store: Option<SecretStoreKind>` recording where the refresh token lives
impl Account { pub fn launch_identity_with(&self, client_id: &str) -> LaunchIdentity; } // Msa: user_type "msa", access_token = mc_token.unwrap_or("0"), xuid, client_id; Offline unchanged; keep `launch_identity()` delegating with client_id ""
pub fn token_expires_soon(expires_rfc3339: Option<&str>, now: OffsetDateTime, margin: Duration) -> bool; // None → true; parse failure → true
// session.rs
pub struct LoginCtx<'a> { pub msa: &'a Msa, pub secrets: &'a dyn SecretStore, pub accounts: &'a Accounts, pub sink: &'a EventSink }
pub async fn login_device_code(ctx: &LoginCtx<'_>, on_code: &(dyn Fn(&DeviceCode) + Send + Sync), sleep: &(dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync)) -> Result<Account, Error>;
// start → on_code(code) → loop: sleep(interval) → poll; SlowDown adds 5 s; stop at expires_in → DeviceCodeExpired; Ready → complete_chain(tokens) → account; secrets.put(id, refresh); accounts.add(account) (active if first); return
pub async fn complete_chain(ctx: &LoginCtx<'_>, tokens: MsaTokens) -> Result<Account, Error>;  // xbl → xsts → mc_login → profile → Account { id: dashed uuid from profile.id, name, kind: Msa, mc_token, mc_token_expires: now + expires_in, xuid, refresh_store: Some(kind) }
pub async fn refresh_account(ctx: &LoginCtx<'_>, account: &Account) -> Result<Account, Error>; // secrets.get(id) else Error::NoRefreshToken; msa.refresh → complete_chain → secrets.put(rotated) → accounts.add (replace) → account
pub async fn ensure_fresh(ctx: &LoginCtx<'_>, account: Account, now: OffsetDateTime) -> Result<Account, Error>; // Offline → as is; Msa → if token_expires_soon(5 min) refresh_account else as is
```
Errors: `NoRefreshToken`, `Secrets(#[from] secrets::Error)`.

- [ ] **Step 1: Tests** (wiremock for all six endpoints, `MemoryStore`, tempdir `Accounts`, a `sleep` that records durations and returns immediately): full login with two `Pending` polls then `Ready` → account stored active, refresh token in the store, `on_code` called once with the user code, sleeps recorded [interval, interval, ...]; `SlowDown` adds 5 s; expiry → `DeviceCodeExpired`; `refresh_account` rotates the stored token and updates `mc_token`; `ensure_fresh` on a fresh token makes no request, on an expiring one refreshes; Offline account passthrough; `launch_identity_with` for Msa.
- [ ] **Step 2: Implement, commit** `feat(core): microsoft login session and refresh`

---

### Task 4: Launcher wiring

**Files:** modify `crates/gcl-core/src/launcher/mod.rs`, `tests/launcher_flow.rs`

```rust
impl Launcher {
    pub fn msa_available(&self) -> bool;                       // config.msa_client_id().is_some()
    pub fn msa_endpoints(&self) -> MsaEndpoints;                // Endpoints gains msa_* fields with debug-only env overrides GCL_MSA_DEVICE_URL, GCL_MSA_TOKEN_URL, GCL_MSA_XBL_URL, GCL_MSA_XSTS_URL, GCL_MSA_MC_URL, GCL_MSA_PROFILE_URL
    pub fn secrets(&self) -> &dyn SecretStore;                  // opened lazily once (OnceCell) via open_default; `with_secret_store(Box<dyn SecretStore>)` builder for tests
    pub fn msa_login(&self, on_code: &(dyn Fn(&DeviceCode) + Send + Sync)) -> Result<Account, crate::Error>;  // Disabled error when unavailable; real tokio sleep
    pub fn msa_refresh(&self, id_or_name: &str) -> Result<Account, crate::Error>;
    // launch_instance: after resolving the account, if kind == Msa: ensure_fresh (refresh on demand); on refresh failure return crate::Error::Auth(e) — the CLI prints "use --offline-user <name> to play offline"; identity = account.launch_identity_with(client_id)
}
```

- [ ] **Step 1: Tests** (`launcher_flow.rs`, hermetic seam + `MemoryStore` + wiremock MSA endpoints via `Endpoints` fields): `msa_login` with an immediate `Ready` poll creates an active Msa account; `launch_instance --dry-run` with that account produces `--userType msa`, `--xuid`, `--clientId <client id>`, and the redacted access token; an expired `mc_token` triggers one refresh request; `msa_available()` false without a client id and `msa_login` → `Disabled`.
- [ ] **Step 2: Implement, commit** `feat(core): launcher microsoft login and refresh before launch`

---

### Task 5: CLI

**Files:** `crates/gcl-cli/src/commands/{account,debug,launch}.rs`, tests

```
gcl account add-msa            -> prints "Open <verification_uri> and enter code <user_code>" then waits; --json emits {"user_code","verification_uri"} as the first line and the account as the second
gcl account refresh <id|name>  -> refreshes now; prints new expiry
gcl account list               -> columns active id name kind expires (expires blank for offline)
gcl launch ...                 -> on an auth error prints the error and "use --offline-user <name> to play offline"; exit 1
gcl debug verify-source msa    -> SKIP when unavailable; otherwise start a device code and print the user code and uri, then exit 0 without waiting (a smoke test that the app registration accepts the scope)
```

- [ ] **Step 1: Tests**: `account add-msa` without a client id exits 1 with `disabled`; with `GCL_MSA_CLIENT_ID=test` and the six `GCL_MSA_*_URL` env overrides pointing at a wiremock server that returns `Ready` on the first poll: `add-msa --json` prints two JSON lines and `account list --json` shows kind `msa`; `GCL_NO_KEYRING=1` in every test (file store in the temp root).
- [ ] **Step 2: Implement, `just check`, commit** `feat(cli): microsoft login and refresh`

---

### Task 6: Docs

- [ ] Update `ARCHITECTURE.md` (`auth` row: msa, secrets, session; launcher msa methods; the six env overrides and `GCL_NO_KEYRING`, debug-only), `.claude/skills/msa-auth/SKILL.md` (match the implemented API; the "Storage" section: keyring service name, `secrets.json` fallback with 0600 and a warning; the launch identity fields; the approval-form note), `.claude/skills/testing/SKILL.md` (MSA wiremock pattern, `MemoryStore`, `GCL_NO_KEYRING`), `docs/SPEC.md` (R6.1 note on the file fallback; R6.4 note on the CLI hint), README (Microsoft login setup: Azure app registration steps and the approval form link, `GCL_MSA_CLIENT_ID` in `.env`). Run `just check && just deny && just lint-claude`. Commit `docs: microsoft login`.
