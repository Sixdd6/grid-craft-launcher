---
name: msa-auth
description: Microsoft account device-code login chain, token refresh, keyring storage, offline accounts, and launch placeholders. Read before touching gcl-core auth/ or account-related launch arguments.
---

Full detail: `docs/research/2026-09-06-msa-auth-and-rust-crates.md` section A.

## Client id

`GCL_MSA_CLIENT_ID` env, then `config.toml` (`Config::msa_client_id`). Absent →
`Launcher::msa_available() == false`; UI hides the button, CLI reports `Error::Disabled`
("microsoft login: disabled (no GCL_MSA_CLIENT_ID)").

To register a client id, see the README's "Microsoft login" section: personal-account app
registration, redirect URI, public client flows, and the Minecraft launcher approval form.
Until that form is approved, `mc_login` (step 5) returns HTTP 403, mapped to
`Error::InvalidAppRegistration`.

## Chain

Built in `gcl-core/src/auth/msa.rs` (`Msa`), driven in order by
`gcl-core/src/auth/session.rs`.

1. `POST <device_code>` form `client_id`, `scope=XboxLive.signin offline_access` →
   `user_code`, `verification_uri`, `device_code`, `interval`, `expires_in`, `message`.
   `Msa::start_device_code` returns a `DeviceCode`; its `device_code` field is `#[serde(skip)]`
   so it is never the accidental output of a `--json` command.
2. Poll `POST <token>` form `grant_type=urn:ietf:params:oauth:grant-type:device_code`,
   `client_id`, `device_code` every `interval` seconds (`Msa::poll_device_code_once`).
   `authorization_pending` → `Poll::Pending`; `slow_down` → `Poll::SlowDown` (the caller adds
   five seconds to its wait); `expired_token` → `Error::DeviceCodeExpired`;
   `authorization_declined` → `Error::DeviceCodeDeclined`.
3. `POST <xbl>` JSON `{ Properties: { AuthMethod: "RPS", SiteName: "user.auth.xboxlive.com",
   RpsTicket: "d=<access_token>" }, RelyingParty: "http://auth.xboxlive.com", TokenType: "JWT" }`
   → `Token`, `DisplayClaims.xui[0].uhs` (`Msa::xbl`).
4. `POST <xsts>` JSON `{ Properties: { SandboxId: "RETAIL", UserTokens: [<xbl>] },
   RelyingParty: "rp://api.minecraftservices.com/", TokenType: "JWT" }` (`Msa::xsts`). On 401
   read `XErr` and map it (below); any other non-200 is `Error::Xsts { code: 0 }`.
5. `POST <mc_login>` JSON `{ identityToken: "XBL3.0 x=<uhs>;<xsts>" }` → `access_token`,
   `expires_in` (`Msa::mc_login`). A 403 is `Error::InvalidAppRegistration`.
6. `GET <profile>` Bearer → `id`, `name` (`Msa::profile`). A 404 is `Error::NoProfile`.

All six base URLs live in `MsaEndpoints`; `Default` is the production set. `Launcher` builds
one `Msa` per call from `Endpoints.msa`, which `GCL_MSA_DEVICE_URL`, `GCL_MSA_TOKEN_URL`,
`GCL_MSA_XBL_URL`, `GCL_MSA_XSTS_URL`, `GCL_MSA_MC_URL`, and `GCL_MSA_PROFILE_URL` can each
override — debug builds only, exactly like the other `GCL_*_BASE_URL` overrides, so a shipped
launcher always talks to the real endpoints.

## XErr

| Code | Error | Message to user |
|---|---|---|
| 2148916233 | `NoXboxProfile` | This Microsoft account has no Xbox profile. Create one at xbox.com, then retry. |
| 2148916235 | `XboxRegionUnavailable` | Xbox Live is not available in this region. |
| 2148916236, 2148916237 | `XboxAdultVerification` | This account needs adult verification before it can sign in. |
| 2148916238 | `XboxChildAccount` | This is a child account: an adult must add it to their family group. |
| anything else | `Xsts { code }` | Xbox XSTS error `<code>` |

## Storage

- `accounts.json` in the root (`auth::store::Accounts`, over `AccountsFile`):
  `{ "accounts": [Account, ...], "active": Option<String> }`. `Account` holds `id`, `name`,
  `kind` (`Offline` | `Msa`), `mc_token`, `mc_token_expires` (RFC 3339), `xuid`, and
  `refresh_store` (which `SecretStoreKind` holds this account's refresh token — omitted from
  JSON when `None`). The Minecraft token is cached here, never in the keyring, so a launch
  never has to touch it.
- Refresh tokens go through `SecretStore` (`gcl-core/src/auth/secrets.rs`), keyed by account
  id: `SecretStore::put/get/delete`. `open_default(root, sink)` picks the backing store:
  - `KeyringStore` (service name `grid-craft-launcher`, from `secrets::SERVICE`): the OS
    keychain, Credential Manager, or Secret Service. `KeyringStore::probe()` checks it works
    by writing, reading, and deleting a `__probe__` entry before anything real is stored.
  - `FileStore`, when the probe fails: `<root>/secrets.json`, a JSON map of account id to
    token, written through `paths::write_atomic_with_mode` at mode `0600` (owner read and
    write only, nobody else, and never briefly world-readable during the write). One
    `Event::Warning` is emitted the first time this happens, naming the file.
  - `MemoryStore`, for tests only: keeps nothing after the run.
  - `GCL_NO_KEYRING=1` forces the file store even when a real keyring is available — debug
    builds only, so tests never write to a developer's actual keyring.
- `Launcher::secrets()` opens the store lazily, on first use, and reuses it after that.
  `Launcher::with_secret_store` fills it ahead of time instead, for tests.
- `Accounts::add` inserts or replaces by id, and makes the account active only when no account
  is active yet. `select(id_or_name)` matches by id first, then by exact case-sensitive name.
  `remove(id)` clears `active` if the removed account was the active one.

## Refresh

- `session::ensure_fresh(ctx, account, now)` refreshes only when `mc_token_expires` is within
  `REFRESH_MARGIN` (5 minutes) of `now`, or unreadable/absent; otherwise it returns the account
  unchanged and makes no request. An offline account always passes through unchanged.
- `session::refresh_account` redeems the stored refresh token
  (`grant_type=refresh_token`, steps 3 to 6 again) and writes the *rotated* refresh token to
  the secret store as soon as the token endpoint answers — before Xbox Live, XSTS, or the
  Minecraft login run. Microsoft invalidates the old token at that point, so if a later step
  fails, the new token must already be saved: otherwise the next attempt would retry with a
  token that no longer works, and the account would need a fresh device-code sign-in.
- A missing stored token is `Error::NoRefreshToken`. A refresh that resolves to a different
  account than the one being refreshed is `Error::AccountMismatch`.
- Without `GCL_MSA_CLIENT_ID`, `Launcher::launch_instance` still launches an account whose
  cached token has not gone stale; a token that needs refreshing but has no client id to
  refresh it with is `Error::Disabled`.

## Offline

- `auth::offline::offline_uuid(name)`: UUID v3 from MD5 of `OfflinePlayer:<name>` bytes
  (`uuid::Uuid::new_v3` with the nil namespace is wrong; compute MD5 yourself and set version 3
  and variant bits, matching Java `nameUUIDFromBytes`).
- `auth::offline::offline_account(name)` builds the `Account` from that UUID, with no token,
  expiry, or xuid.
- `Account::launch_identity()` / `launch_identity_with(client_id)` build the `LaunchIdentity`
  for both kinds. Offline placeholders: `access_token = "0"`, `user_type = "legacy"`,
  `xuid = ""`, `client_id = ""` (the client id argument is ignored for an offline account).

## Placeholders for MSA

`Account::launch_identity_with(client_id)` on a Microsoft account fills: `auth_player_name =
name`, `auth_uuid = id without dashes`, `auth_access_token = mc_token` (or `"0"` when there is
none, so a launch fails inside the game rather than with an empty token), `user_type = "msa"`,
`auth_xuid = xuid` (or empty), `clientid = client_id` as given by the caller (`Launcher`
passes the base64 client id `${clientid}` expects; the field itself does no encoding).

## GUI threading

`Launcher::msa_login(on_code)` blocks the calling thread while it drives the login future on
the owned tokio runtime, so `on_code` runs on that same thread, synchronously, before
`msa_login` returns. It must not block (waiting on user input, for instance) and must not call
back into the `Launcher` — both would deadlock. Its only job is to hand the `DeviceCode` to
the UI so the code and link can be shown; the CLI's `show_code` (printing text or one JSON
line) is the reference implementation.

## Do not

- Do not log tokens, device codes, or the client id. Every function in `auth::msa` uses
  `#[tracing::instrument(skip_all)]`; every `Debug` impl in `auth::msa` and `auth::mod` redacts
  its secret fields.
- Do not print a token, device code, or client id in an error message or `--json` output.
  `Error::Oauth` and `Error::Xbl` carry only an error code, never the OAuth
  `error_description`, which can quote the request back, tokens included.
- Do not block launch on a failed refresh; offer offline mode.
- Do not store the MSA access token. Only the refresh token (secret store) and the Minecraft
  token (`accounts.json`) are kept.
