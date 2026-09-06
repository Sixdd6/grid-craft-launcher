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

- `accounts.json` in the root: `[{ id (uuid), name, kind: msa|offline, mc_token, mc_token_expires, xuid }]`. Tokens here are short-lived.
- Refresh token: keyring service `grid-craft-launcher`, user `<uuid>`. On keyring error, store in `accounts.json` with `refresh_token_in_file = true` and emit one warning event.
- Refresh when `mc_token_expires` is within 5 minutes: `grant_type=refresh_token` then steps 3 to 6.

## Offline

- UUID v3 from MD5 of `OfflinePlayer:<name>` bytes (`uuid::Uuid::new_v3` with the nil namespace is wrong; compute MD5 yourself and set version 3 and variant bits, matching Java `nameUUIDFromBytes`).
- Placeholders: `auth_access_token = "0"`, `user_type = "legacy"`, `clientid = ""`, `auth_xuid = ""`.

## Placeholders for MSA

`auth_player_name = name`, `auth_uuid = id without dashes`, `auth_access_token = mc_token`, `user_type = "msa"`, `auth_xuid = xuid`, `clientid = base64 of the client id`.

## Do not

- Do not log tokens, codes, or the client id.
- Do not block launch on a failed refresh; offer offline mode.
- Do not store the MSA access token; only the refresh token and the Minecraft token.
