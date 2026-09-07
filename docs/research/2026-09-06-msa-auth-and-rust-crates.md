# Microsoft account auth and Rust crate choices

Researched 2026-09-06. Source of truth for the `msa-auth` skill and the crate list in
`rust-conventions`.

## A. Microsoft account authentication

**Implementation status (plan 4, 2026-09-06):** implemented in `gcl-core/src/auth/` (`msa`,
`secrets`, `session`) and `gcl-cli`'s `account` and `debug verify-source msa` commands. Every
step of the chain is covered by wiremock tests against a mock server, including the XErr
table, the poll loop's slow-down and expiry handling, and refresh-token rotation. It has not
been verified against the real Microsoft, Xbox Live, and Minecraft services endpoints: this
development machine has no Azure app registration. The keyring storage uses the `keyring`
crate's 4.x API, which resolves the native backend on the first `Entry::new` call rather than
needing a separate setup step.

### App registration

1. Azure Portal → Microsoft Entra ID → App registrations → New registration.
2. Supported account types: personal Microsoft accounts (any org plus personal).
3. Platform: Mobile and desktop applications. Redirect URI
   `https://login.microsoftonline.com/common/oauth2/nativeclient`. No client secret.
4. Enable "Allow public client flows" for device code.
5. Submit the Minecraft launcher approval form (linked from
   https://help.minecraft.net/hc/en-us/articles/16254801392141). Until approved, the
   `login_with_xbox` step returns HTTP 403 "Invalid app registration".

The client ID is not a secret but is app-specific. The launcher reads it from
`GCL_MSA_CLIENT_ID` env or its config; the repo ships a placeholder only.

### Flow choice

Use the device code flow for the MVP. It needs no local redirect server, works from the CLI,
and supports 2FA. The UI shows the code and opens the browser.

### Token chain

1. Device code: `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode`
   body `client_id`, `scope=XboxLive.signin offline_access`.
   Poll `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/token`
   with `grant_type=urn:ietf:params:oauth:grant-type:device_code`, `device_code`, `client_id`
   until it returns `access_token` and `refresh_token`.
2. Xbox Live: `POST https://user.auth.xboxlive.com/user/authenticate`
   ```json
   { "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com",
                     "RpsTicket": "d=<msa access token>" },
     "RelyingParty": "http://auth.xboxlive.com", "TokenType": "JWT" }
   ```
   Response: `Token`, `DisplayClaims.xui[0].uhs`.
3. XSTS: `POST https://xsts.auth.xboxlive.com/xsts/authorize`
   ```json
   { "Properties": { "SandboxId": "RETAIL", "UserTokens": ["<xbl token>"] },
     "RelyingParty": "rp://api.minecraftservices.com/", "TokenType": "JWT" }
   ```
4. Minecraft: `POST https://api.minecraftservices.com/authentication/login_with_xbox`
   body `{ "identityToken": "XBL3.0 x=<uhs>;<xsts token>" }` → `access_token`, `expires_in`.
5. Entitlements: `GET https://api.minecraftservices.com/entitlements/mcstore` with
   `Authorization: Bearer <mc token>`. Empty `items` may still be a Game Pass owner; treat the
   profile call as the real ownership check.
6. Profile: `GET https://api.minecraftservices.com/minecraft/profile` → `id` (UUID without
   dashes), `name`. HTTP 404 means the account owns no profile.

### XSTS errors (`XErr`)

| Code | Meaning |
|---|---|
| 2148916233 | No Xbox account. Send the user to https://www.xbox.com to create one. |
| 2148916235 | Xbox Live unavailable in this country. |
| 2148916236 / 2148916237 | Adult verification needed (South Korea). |
| 2148916238 | Child account not in a family. |

### Lifetimes

- Minecraft access token: 24 hours.
- MSA refresh token: about 90 days. Refresh with `grant_type=refresh_token` then rerun steps 2 to 6.
- Store the refresh token in the OS keyring. Cache the Minecraft token and expiry in the
  accounts file so launch does not touch the keyring.

### Offline mode

- UUID: version 3 UUID from MD5 of `OfflinePlayer:<name>` (case-sensitive), same as Java's
  `UUID.nameUUIDFromBytes`.
- Launch args: `--username <name> --uuid <uuid> --accessToken 0 --userType legacy`.
  Set `${clientid}` and `${auth_xuid}` to empty strings.

### Launch argument placeholders

`${auth_player_name}`, `${auth_uuid}`, `${auth_access_token}`, `${user_type}` (`msa` for
Microsoft accounts), `${clientid}`, `${auth_xuid}`, `${version_name}`, `${version_type}`,
`${game_directory}`, `${assets_root}`, `${assets_index_name}`, `${natives_directory}`,
`${launcher_name}`, `${launcher_version}`, `${classpath}`, `${library_directory}`,
`${classpath_separator}`, `${resolution_width}`, `${resolution_height}`, `${quickPlayPath}`,
`${quickPlaySingleplayer}`, `${quickPlayMultiplayer}`, `${quickPlayRealms}`.

## B. Rust crates

Versions were current on 2026-09-06. Check crates.io before pinning.

| Need | Crate | Notes |
|---|---|---|
| async runtime | `tokio` 1.x (`rt-multi-thread`, `fs`, `process`, `macros`, `sync`, `time`) | |
| HTTP | `reqwest` 0.12 (`rustls-tls`, `stream`, `json`, `gzip`), no default features | one shared client with User-Agent |
| retries | `backon` or hand-written exponential backoff | keep it small |
| hashing | `sha1`, `sha2`, `md-5` (RustCrypto), `murmur2` | CurseForge fingerprint needs whitespace stripping |
| zip/jar | `zip` 2.x with `deflate` | read installer jars, mrpack, CF packs |
| JSON/TOML | `serde`, `serde_json`, `toml` 0.8 | |
| paths | `directories` 5 | XDG on Linux |
| secrets | `keyring` 3.x (`linux-native-sync-persistent` or `sync-secret-service`) | fall back to file with warning when no secret service |
| uuid | `uuid` (`v3`, `v4`, `serde`) | offline UUIDs |
| errors | `thiserror` in the library, `anyhow`/`miette` in binaries | |
| logging | `tracing`, `tracing-subscriber`, `tracing-appender` | one log file per launch |
| CLI | `clap` 4 (`derive`) | |
| UI | `slint` 1.15 (`backend-winit`, `renderer-femtovg`, `compat-1-2`), `slint-build` | `slint-viewer`, `slint-lsp` as dev tools |
| tests | `cargo-nextest`, `wiremock` 0.6, `insta` 1.x, `tempfile`, `assert_cmd` | |
| lint/supply chain | `clippy`, `cargo-deny`, `cargo-audit` | |
| release | `cargo-dist`, `linuxdeploy` for AppImage | |

### Reference launchers to read

| Project | License | URL |
|---|---|---|
| Modrinth App (theseus) | GPL-3.0 | https://github.com/modrinth/code |
| Prism Launcher | GPL-3.0 (C++) | https://github.com/PrismLauncher/PrismLauncher |
| Lyceris | MIT | https://github.com/BatuhanAksoyy/lyceris |
| minecraft-msa-auth | MIT | https://github.com/minecraft-rs/minecraft-msa-auth |
| minecraft-launcher-lib (Python, docs are good) | BSD-2 | https://minecraft-launcher-lib.readthedocs.io/ |

GPL-3.0 code may be ported into this project. MIT code may be used with attribution.

### Gotchas

- Slint: keep the UI in its own crate so `slint-build` reruns only when `.slint` files change.
- Slint falls back from Skia to FemtoVG silently; log the active renderer at startup.
- `keyring` on Linux needs a running Secret Service (GNOME Keyring or KWallet). Detect failure
  and fall back to a file in the config directory with a warning.
- `zip` 2.x changed its API from 0.6; read the current docs.
- `insta` pending snapshots fail CI. Run `cargo insta review` before committing.

## Sources

- https://minecraft.wiki/w/Microsoft_authentication
- https://wiki.vg/Microsoft_Authentication_Scheme
- https://github.com/dscalzi/HeliosLauncher/blob/master/docs/MicrosoftAuth.md
- https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app
- https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code
- https://minecraft.wiki/w/UUID
- https://docs.rs/slint/latest/slint/docs/cargo_features/index.html
- https://docs.rs/keyring/latest/keyring/
- https://nexte.st/
- https://docs.rs/wiremock/latest/wiremock/
- https://crates.io/crates/cargo-dist
