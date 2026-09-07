# Architecture

Three crates. Logic lives in `gcl-core`. The binaries render and forward.

| Crate | Binary | Role |
|---|---|---|
| `gcl-core` | none | all launcher logic; tokio runtime owner |
| `gcl-cli` | `gcl` | clap commands over core; prints text or JSON |
| `gcl-ui` | `grid-craft-launcher` | Slint screens over core; no logic |

## gcl-core modules

Each module owns one concern. Its public interface is documented at the top of its `mod.rs`.
Add a row here when you add a module.

| Module | Owns | Depends on |
|---|---|---|
| `paths` | root resolution, subdirectory constants | none |
| `config` | `config.toml` read and write, env overrides | none |
| `http` | shared reqwest client, User-Agent, retry, rate-limit backoff | none |
| `download` | content-addressed cache, parallel queue, hash checks, progress events | `http`, `paths`, `events` |
| `mojang` | version manifest, version JSON, rules, argument templating, assets, `inheritsFrom` merge | `download`, `http`, `paths`, `events` |
| `java` | detect runtimes, fetch Mojang runtimes, pick by major version. The runtime component comes from the version JSON (`javaVersion.component`); `component_for_major` only guesses when the version JSON names none | `download`, `http`, `paths` |
| `loaders` | `fabric`, `quilt`, `forge`, `neoforge`: version listing and headless install into `cache/versions/<id>.json`. `LoaderCtx` carries `http`, `root`, `dl`, `java`, `runner` (the `ProcessRunner` test seam; production uses `JavaRunner`), `mojang`. `LoaderEndpoints` holds the four base URLs | `download`, `mojang`, `java` |
| `sources` | `Source` trait (`search`, `project`, `versions`, `version`, `resolve_by_hash`, `resolve_by_fingerprint`); `modrinth::Modrinth` and `curseforge::CurseForge` clients; `fingerprint` (CurseForge murmur2); shared types in `types.rs` (`SourceId`, `SearchQuery`, `SearchHit`, `SearchPage`, `Project`, `Version`, `VersionFile`, `Dependency`, `ReleaseKind`, `VersionFilter`, `page_url`, `pack_page_url`) | `http`, `instances::model` (for `ContentKind`, `Loader`) |
| `content` | orchestration between `sources` and `instances::content`: `compatible_loaders`, `pick_version`, `add` (breadth-first required-dependency walk, depth 10), `check_updates`, `apply_update`, `import_manual`; `ManualDownload`, `AddOutcome`, `ContentCtx` | `sources`, `instances`, `download`, `paths`, `events` |
| `modpacks` | detect and parse `.mrpack` and CurseForge pack zips into a `PackPlan`; `import`/`import_plan` build a new instance from one; `mrpack` and `curseforge` submodules hold the per-format manifest parsers; `fetch_pack` downloads a pack's own archive from a source | `sources`, `content`, `instances`, `loaders` |
| `instances` | instance layout, `instance.toml`, `instances::content` (`place_file`, `place_world`, `set_enabled`, `remove`, `installed`, `file_path`, `target_dir`), `list` skips unparsable instances with a warning; instance creation preseeds `options.txt` through `settings` | `paths`, `settings`, `sources` (for `SourceId`), `download` (for `link_or_copy`) |
| `settings` | `options.txt` preseed (`apply_preseed`) and keyed overrides (`apply_overrides_to`), plus `validate_key`/`validate_value`. Takes a game directory and a map, so it does not depend on `instances` | `paths` |
| `auth` | offline accounts (`auth::offline`) and the account store (`auth::store`, `accounts.json`); `LaunchIdentity` placeholders. `auth::msa` (`Msa`, `MsaEndpoints`, the six-step login chain, `Error`); `auth::secrets` (`SecretStore` trait, `KeyringStore`, `FileStore`, `MemoryStore`, `open_default`); `auth::session` (`LoginCtx`, `login_device_code`, `complete_chain`, `refresh_account`, `ensure_fresh`) | `paths`, `http` |
| `launch` | `launch::command::build` turns an `InstallPlan`, account, instance, and JVM settings into a `LaunchCommand`; `launch::spawn` starts and streams it | `instances`, `mojang`, `auth` |
| `events` | `Progress` and `LogLine` event types and the channel | none |
| `launcher` | `Launcher` handle: owns the tokio runtime, root, config, HttpClient, event channel, and cancellation token; every binary entry point goes through it. Orchestrates `install_loader`, `install_instance`, `launch_instance`, `java_for_version`, `apply_settings_overrides`, and the content and modpack flows below (`sources`, `search`, `add_content`, `list_content`, `remove_content`, `set_content_enabled`, `check_updates`, `apply_updates`, `pending_manual`, `import_manual_file`, `import_modpack_file`, `import_modpack`, `configured_or_detected_java`), plus `msa_available`, `msa_login(on_code)`, `msa_refresh(id_or_name)`, and `secrets()` (lazy: opens the OS keyring or falls back to a file on first use; `with_secret_store` overrides it for tests). `sources()` builds Modrinth always and CurseForge only when a `CURSEFORGE_API_KEY` was found when this launcher opened; the list is built once and cached, so changing the key needs a new `Launcher`. `Endpoints::from_env()` reads `GCL_MOJANG_BASE_URL`, `GCL_FABRIC_BASE_URL`, `GCL_QUILT_BASE_URL`, `GCL_FORGE_META_BASE_URL`, `GCL_FORGE_MAVEN_BASE_URL`, `GCL_NEOFORGE_BASE_URL`, `GCL_MODRINTH_BASE_URL`, `GCL_CURSEFORGE_BASE_URL`, and the six `Endpoints.msa` overrides `GCL_MSA_DEVICE_URL`, `GCL_MSA_TOKEN_URL`, `GCL_MSA_XBL_URL`, `GCL_MSA_XSTS_URL`, `GCL_MSA_MC_URL`, and `GCL_MSA_PROFILE_URL` (all test-only, and read in a debug build only: a release build returns `Endpoints::default()` whatever the environment holds). `GCL_NO_KEYRING=1` is the matching override for `secrets()`: it forces the file store, also debug-only, so tests never write to a developer's real keyring. `open_with_endpoints` is the test seam that takes `Endpoints` directly and reads no environment | `config`, `http`, `events`, `paths`, `download`, `mojang`, `java`, `instances`, `loaders`, `auth`, `launch`, `settings`, `sources`, `content`, `modpacks` |

Rules:

- Only `launcher` depends on `launch`.
- `sources` never touches `instances::content` or `content`. Placing files is `instances::content::place_file` / `place_world`; deciding what to place is `content`.
- Blocking work (zip, hashing, processors) runs under `tokio::task::spawn_blocking`.
- Every remote JSON has a serde struct in the owning module and a fixture under `tests/fixtures/`.
- `launcher` is the only module binaries call directly.

## Launch flow

`Launcher::launch_instance(slug, account, offline_user, dry_run)` runs these steps in order:

1. Resolve the account: `offline_user` creates or reuses an offline account, `account` selects a
   saved one by id or name, otherwise the active account is used.
2. `install_loader`: resolve the instance's loader version (`recommended`/`latest`/unset become
   a concrete build) and install it into the shared version cache.
3. `install_instance`: resolve the merged version JSON and install libraries, assets, and
   natives.
4. `java_for_version` / `ensure_java_for`: pick or install the Java runtime the version JSON
   asks for, unless the instance or config names a Java path.
5. `apply_settings_overrides`: write the instance's `options.txt` overrides.
6. `launch::build`: build the `LaunchCommand`.
7. `launch::spawn` and `launch::wait`: start Java and stream its output, unless `dry_run` is
   set, in which case the command is returned unstarted.

## Login flow

`session::login_device_code(ctx, on_code, sleep)` drives the Microsoft sign-in:

1. `msa::start_device_code`: ask for a device code. `on_code` is called once with it, so the
   caller can show the code and the verification link.
2. Poll `msa::poll_device_code_once` in a loop, sleeping `interval_secs` between calls (via
   `sleep`, so a test can record the wait instead of really waiting). `SlowDown` adds five
   seconds to the interval; a summed wait past the code's `expires_in_secs` is
   `Error::DeviceCodeExpired`.
3. On approval, `session::complete_chain` runs Xbox Live, XSTS, the Minecraft login, and the
   profile read, then builds an `Account`.
4. The refresh token goes to the secret store (`ctx.secrets.put`); the account goes to
   `accounts.json` (`ctx.accounts.add`), becoming active only if it is the first account.

`session::refresh_account` redeems a saved refresh token the same way, but writes the rotated
refresh token as soon as the token endpoint answers, before the Xbox and Minecraft steps run:
Microsoft has already invalidated the old token by then, so a later failure must not lose the
new one. `session::ensure_fresh` calls it only when the cached Minecraft token expires within
`REFRESH_MARGIN` (5 minutes) of `now`, or carries no readable expiry.

## Launch identity

`Launcher::launch_instance` calls `ensure_fresh` before install, so a Microsoft account's
token is refreshed ahead of the game needing it. Without a configured client id,
`ensure_fresh` is skipped: a token that is still valid launches as-is, but one that is
expiring is `Error::Disabled`, since there is no client id to refresh it with. An offline
account is never touched. `Account::launch_identity_with(client_id)` then builds the launch
placeholders: a Microsoft account gets `user_type = "msa"`, its cached token (or `"0"` when
there is none) as `auth_access_token`, its xuid (or empty), and the given client id; an
offline account always gets `"0"`, `"legacy"`, and empty xuid and client id.

## Content flow

`content::add(ctx, instance, req)` installs a project and its required dependencies:

1. Resolve the project at the requested source (`Source::project`).
2. Skip a project already installed from the same source, unless a different version was
   pinned; its dependencies are still walked, from the version fetched again.
3. List versions with a `VersionFilter` for the instance's Minecraft version and, for a
   `Mod`, its `compatible_loaders`. `pick_version` then picks the file: `want` (a version
   id or number) wins when compatible, else the newest release by `ReleaseKind` then
   publish time.
4. Download the version's primary file into `cache/objects/tmp/<uuid>`, verify it by sha1
   when the source published one (else hash it after the fact), then move it to its
   content-addressed path. A jar both sources serve is fetched once.
5. Place the object into the instance (`instances::content::place_file` or `place_world`
   for a `World`) and record a `ContentEntry` in `instance.toml`.
6. Queue every `Required` dependency the version named, one hop deeper, up to depth 10;
   a data pack's dependency inherits its parent's world.

A file whose author opted out of third-party distribution has no download URL. That never
fails `add`: it is collected as a `ManualDownload` (source, project id, version id, file
name, page URL, expected fingerprint or sha1) and appended to
`instances/<slug>/pending-manual.json`. `content::import_manual` later verifies a
hand-fetched file against that fingerprint or sha1, copies it into the object store, and
places it the same way; `Launcher::import_manual_file` then drops the entry from the
pending file.

## Modpack import flow

`modpacks::import` (or `import_plan`, when the caller already read the manifest) builds a
new instance from a pack archive:

1. `detect` looks at the archive's root: `modrinth.index.json` is an `.mrpack`, a root
   `manifest.json` with `manifestType: "minecraftModpack"` is a CurseForge pack; anything
   else is `Error::UnknownFormat`.
2. `read_plan` parses the manifest into a `PackPlan` (name, Minecraft version, loader,
   loader version, files, override prefixes). An `.mrpack` file carries its own path, URL,
   and sha1; a CurseForge file carries only a `(source, project_id, file_id)` triple, since
   its folder comes from the project's class at the API.
3. The instance is created before anything downloads, so a mid-import failure leaves a
   half-built directory that is deleted again unless `keep_partial` is set.
4. The pack's loader installs through `loaders::install`.
5. Files install: an `.mrpack` file downloads straight from its URL into the object store
   and links into place; a CurseForge pack resolves its file ids with `files_batch` and
   its projects with `mods_batch` first, then downloads and places each by its project's
   class. A CurseForge file with no download URL becomes a `ManualDownload`, the same as in
   `content::add`, and does not fail the import.
6. Override folders (`overrides/`, then `client-overrides/` for `.mrpack`; the manifest's
   named folder, default `overrides/`, for CurseForge) are copied over the game directory
   last, later folders overwriting earlier ones.
7. `instance.toml`'s `[pack]` records where the pack came from (`fetch_pack` returns this
   `PackSource` when the pack was fetched from a source rather than a local file).

## Event flow

Core functions take an `EventSink` (an `mpsc::Sender<Event>`). The CLI prints events. The UI
forwards them to the Slint event loop with `slint::invoke_from_event_loop` and updates models.

## Threading

`gcl-core` builds one tokio runtime in `Launcher::new()`. Binaries call `Launcher` methods and
never create a runtime. The UI runs on the main thread and talks to core through a `Launcher`
handle that spawns tasks on the runtime.

## App root layout

```
<root>/
  config.toml
  accounts.json
  instances/<slug>/instance.toml
  instances/<slug>/.minecraft/
  cache/versions/<id>.json
  cache/libraries/<maven path>
  cache/assets/{indexes,objects}/
  cache/natives/<version>/
  cache/runtimes/<component>/<platform>/
  cache/installers/
  cache/objects/<sha1[0..2]>/<sha1>
  logs/
```
