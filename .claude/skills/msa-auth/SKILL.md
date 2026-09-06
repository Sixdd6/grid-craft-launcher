---
name: msa-auth
description: Microsoft account device-code login chain, token refresh, keyring storage, offline accounts, and launch placeholders. Read before touching gcl-core auth/ or account-related launch arguments.
---

Full detail: `docs/research/2026-09-06-msa-auth-and-rust-crates.md` section A.

## Client id

`GCL_MSA_CLIENT_ID` env, then `config.toml`. Absent → `auth::microsoft_available() == false`; UI hides the button, CLI prints a one-line reason.

## Chain

1. `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode` form `client_id`, `scope=XboxLive.signin offline_access` → `user_code`, `verification_uri`, `device_code`, `interval`. Show code and URI; open browser in the UI.
2. Poll `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/token` form `grant_type=urn:ietf:params:oauth:grant-type:device_code`, `client_id`, `device_code` every `interval` seconds. `authorization_pending` continues; `expired_token` fails.
3. `POST https://user.auth.xboxlive.com/user/authenticate` JSON `{ Properties: { AuthMethod: "RPS", SiteName: "user.auth.xboxlive.com", RpsTicket: "d=<access_token>" }, RelyingParty: "http://auth.xboxlive.com", TokenType: "JWT" }` → `Token`, `DisplayClaims.xui[0].uhs`.
4. `POST https://xsts.auth.xboxlive.com/xsts/authorize` JSON `{ Properties: { SandboxId: "RETAIL", UserTokens: [<xbl>] }, RelyingParty: "rp://api.minecraftservices.com/", TokenType: "JWT" }`. On 401 read `XErr`.
5. `POST https://api.minecraftservices.com/authentication/login_with_xbox` JSON `{ identityToken: "XBL3.0 x=<uhs>;<xsts>" }` → `access_token`, `expires_in`.
6. `GET https://api.minecraftservices.com/minecraft/profile` Bearer → `id`, `name`. 404 → `Error::NoProfile`.

## XErr

| Code | Message to user |
|---|---|
| 2148916233 | This Microsoft account has no Xbox profile. Create one at xbox.com, then retry. |
| 2148916235 | Xbox Live is not available in this region. |
| 2148916236, 2148916237 | Adult verification is required for this account. |
| 2148916238 | This is a child account. An adult in the family must add it to the family group. |

## Storage

Built so far (`gcl-core/src/auth/`): the store and offline accounts. The Microsoft device-code
chain above (steps 1 to 6) is not implemented yet; it lands in plan 4.

- `accounts.json` in the root (`auth::store::Accounts`, over `AccountsFile`):
  `{ "accounts": [Account, ...], "active": Option<String> }`. `active` holds the id of the
  selected account, or is absent when none is selected.
- `Account { id, name, kind: AccountKind::Offline | Msa, mc_token, mc_token_expires, xuid }`. The
  last three are `Option` and empty for an offline account. `Account`'s `Debug` impl redacts
  `mc_token` to `<set>`/`<unset>`, so a traced or logged account never prints a token.
- `Accounts::add` inserts or replaces by id, and makes the account active only when no account is
  active yet (adding a second account does not steal the active slot). `select(id_or_name)`
  matches by id first, then by exact case-sensitive name, and makes the match active.
  `remove(id)` clears `active` if the removed account was the active one.
- Refresh token (once MSA lands): keyring service `grid-craft-launcher`, user `<uuid>`. On
  keyring error, store in `accounts.json` with `refresh_token_in_file = true` and emit one
  warning event.
- Refresh when `mc_token_expires` is within 5 minutes: `grant_type=refresh_token` then steps 3 to 6.

## Offline

- `auth::offline::offline_uuid(name)`: UUID v3 from MD5 of `OfflinePlayer:<name>` bytes
  (`uuid::Uuid::new_v3` with the nil namespace is wrong; compute MD5 yourself and set version 3
  and variant bits, matching Java `nameUUIDFromBytes`).
- `auth::offline::offline_account(name)` builds the `Account` from that UUID, with no token,
  expiry, or xuid.
- `Account::launch_identity()` builds the `LaunchIdentity` for both kinds. Offline placeholders:
  `access_token = "0"`, `user_type = "legacy"`, `xuid = ""`, `client_id = ""`.

## Placeholders for MSA

`auth_player_name = name`, `auth_uuid = id without dashes`, `auth_access_token = mc_token`, `user_type = "msa"`, `auth_xuid = xuid`, `clientid = base64 of the client id`.

## Do not

- Do not log tokens, codes, or the client id.
- Do not block launch on a failed refresh; offer offline mode.
- Do not store the MSA access token; only the refresh token and the Minecraft token.
