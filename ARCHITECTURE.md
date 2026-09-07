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
| `sources` | `Source` trait (`search`, `search_packs`, `project`, `versions`, `version`, `resolve_by_hash`, `resolve_by_fingerprint`); `modrinth::Modrinth` and `curseforge::CurseForge` clients; `fingerprint` (CurseForge murmur2); shared types in `types.rs` (`SourceId`, `SearchQuery`, `SearchHit`, `SearchPage`, `Project`, `Version`, `VersionFile`, `Dependency`, `ReleaseKind`, `VersionFilter`, `page_url`, `pack_page_url`). `search_packs` is the modpack search, since a modpack is no `ContentKind`: its hits carry `SearchHit.is_pack` and a `pack_page_url` link, and the default trait body answers `Error::UnsupportedPacks` | `http`, `instances::model` (for `ContentKind`, `Loader`) |
| `content` | orchestration between `sources` and `instances::content`: `compatible_loaders`, `pick_version`, `add` (breadth-first required-dependency walk, depth 10), `check_updates`, `apply_update`, `import_manual`; `ManualDownload`, `AddOutcome`, `ContentCtx` | `sources`, `instances`, `download`, `paths`, `events` |
| `modpacks` | detect and parse `.mrpack` and CurseForge pack zips into a `PackPlan`; `import`/`import_plan` build a new instance from one; `mrpack` and `curseforge` submodules hold the per-format manifest parsers; `fetch_pack` downloads a pack's own archive from a source | `sources`, `content`, `instances`, `loaders` |
| `instances` | instance layout, `instance.toml`, `instances::content` (`place_file`, `place_world`, `set_enabled`, `remove`, `installed`, `file_path`, `target_dir`), `list` skips unparsable instances with a warning; instance creation preseeds `options.txt` through `settings` | `paths`, `settings`, `sources` (for `SourceId`), `download` (for `link_or_copy`) |
| `settings` | `options.txt` preseed (`apply_preseed`) and keyed overrides (`apply_overrides_to`), plus `validate_key`/`validate_value`. Takes a game directory and a map, so it does not depend on `instances`. `settings::catalog` holds the known keys: label, group, control (slider, toggle, choice, text), default, and `parse_value`/`format_value` for the stored string. `settings::doc::merged` builds the editor's row list: one row per catalog key in catalog order, then unknown keys alphabetically, each row naming the layer that won. Layers rank override, then `options.txt`, then preseed, then catalog default — the file outranks the preseed because `apply_preseed` writes only when the file is absent. `settings::doc::validate` checks one value against its catalog entry | `paths` |
| `auth` | offline accounts (`auth::offline`) and the account store (`auth::store`, `accounts.json`); `LaunchIdentity` placeholders. `auth::msa` (`Msa`, `MsaEndpoints`, the six-step login chain, `Error`); `auth::secrets` (`SecretStore` trait, `KeyringStore`, `FileStore`, `MemoryStore`, `open_default`); `auth::session` (`LoginCtx`, `login_device_code`, `complete_chain`, `refresh_account`, `ensure_fresh`) | `paths`, `http` |
| `launch` | `launch::command::build` turns an `InstallPlan`, account, instance, and JVM settings into a `LaunchCommand`; `launch::spawn` starts and streams it, returning a `RunningGame` that holds the child in a shared `ChildHandle` (`Arc<tokio::sync::Mutex<Option<Child>>>`) plus its `pid`; `launch::wait` polls the child every 100 ms so the handle is free between checks, and `launch::request_stop`/`force_stop` send `SIGTERM`/`SIGKILL` on unix and kill through the handle elsewhere | `instances`, `mojang`, `auth` |
| `events` | `Progress` and `LogLine` event types and the channel | none |
| `launcher` | `Launcher` handle: owns the tokio runtime, root, config, HttpClient, event channel, and cancellation token; every binary entry point goes through it. `Launcher` is `Send + Sync`, so a GUI can hold it in an `Arc` and call it from any worker thread. Orchestrates `install_loader`, `install_instance`, `launch_instance`, `stop_instance` (asks the running game to exit, waits `STOP_GRACE` = 10 s, then kills it; `NotRunning` when this launcher started no game for that slug), `running_slugs`, `search_packs`, `launch_instance_async` (starts the game and returns a `RunningLaunch { pid, log_path, slug, wait }` at once, instead of blocking until it exits, so a caller can show the game as running and keep working; `wait` is a runtime task the caller awaits or drops), `java_for_version`, `apply_settings_overrides`, and the content and modpack flows below (`sources`, `search`, `add_content`, `list_content`, `remove_content`, `set_content_enabled`, `check_updates`, `apply_updates`, `pending_manual`, `import_manual_file`, `import_modpack_file`, `import_modpack`, `configured_or_detected_java`), plus `msa_available`, `msa_login(on_code)` (blocks on the shared cancel token), `msa_login_with_cancel(on_code, cancel)` (the same login watching a caller-supplied token, so a GUI's "cancel sign-in" button can end one login without cancelling every other download), `msa_refresh(id_or_name)`, and `secrets()` (lazy: opens the OS keyring or falls back to a file on first use; `with_secret_store` overrides it for tests). `config()` returns a `ConfigRead` guard (derefs to `Config`); hold it no longer than the read needs and never across another `Launcher` call, since a call that needs the write lock (`update_config`, most instance and content methods) would deadlock against a guard the caller is still holding. `update_config(f)` runs `f` under the write lock, saves `config.toml`, and clears the cached source list before returning, so a GUI settings screen never opens the file itself. `set_instance_override`/`unset_instance_override` edit one `options.txt` override and save `instance.toml`, and `set_game_default`/`unset_game_default` do the same for the preseed in `config.toml`; all four validate the key and value, and a known catalog key is also range- and choice-checked, so the CLI and the GUI reject the same values. `settings_rows_for_defaults` and `settings_rows_for_instance` return the merged `settings::doc::Row` list a settings screen renders; `set_instance_jvm` replaces an instance's JVM overrides (rejects `min > max`); `instance_options` reads back what `options.txt` currently holds; `list_worlds` lists an instance's `saves/` folders; `instance_summary` returns everything a detail screen needs in one call (`InstanceSummary`: whether the resolved version is installed, the Java path that would be used, and so on). `sources()` builds Modrinth always and CurseForge only when a `CURSEFORGE_API_KEY` was found when this launcher opened; the list is cached, and `update_config` clears that cache, so a key saved from the CLI or the settings screen takes effect on the next call. `Endpoints::from_env()` reads `GCL_MOJANG_BASE_URL`, `GCL_FABRIC_BASE_URL`, `GCL_QUILT_BASE_URL`, `GCL_FORGE_META_BASE_URL`, `GCL_FORGE_MAVEN_BASE_URL`, `GCL_NEOFORGE_BASE_URL`, `GCL_MODRINTH_BASE_URL`, `GCL_CURSEFORGE_BASE_URL`, and the six `Endpoints.msa` overrides `GCL_MSA_DEVICE_URL`, `GCL_MSA_TOKEN_URL`, `GCL_MSA_XBL_URL`, `GCL_MSA_XSTS_URL`, `GCL_MSA_MC_URL`, and `GCL_MSA_PROFILE_URL` (all test-only, and read in a debug build only: a release build returns `Endpoints::default()` whatever the environment holds). `GCL_NO_KEYRING=1` is the matching override for `secrets()`: it forces the file store, also debug-only, so tests never write to a developer's real keyring. `open_with_endpoints` is the test seam that takes `Endpoints` directly and reads no environment | `config`, `http`, `events`, `paths`, `download`, `mojang`, `java`, `instances`, `loaders`, `auth`, `launch`, `settings`, `sources`, `content`, `modpacks` |

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

## gcl-ui structure

```
crates/gcl-ui/
  build.rs                     slint_build::compile_with_config, fluent style, embeds resources
  ui/app.slint                 AppWindow: rail, one screen mounted at a time, every dialog, toasts
  ui/theme.slint                global Theme: colors, radius, gap, pad, font sizes, row-height
  ui/types.slint                exported structs crossing the Rust boundary (InstanceRow, ContentRow, ...)
  ui/state.slint                Shell global plus the per-screen *State globals (below)
  ui/components/                Button, Card, ListRow, ProgressBar, ProgressPanel, SearchBox,
                                 TabBar, Rail, ToastHost, Dialog and its Confirm/Prompt/Choice/
                                 DeviceCode/CreateInstance variants
  ui/screens/                   instances.slint, instance.slint, browser.slint, accounts.slint,
                                 settings.slint — pure layout, no logic
  src/main.rs                   parses args (`--smoke`, `-h`, `-V`), opens `Launcher::new`, builds
                                 the window, runs the event loop
  src/app.rs                    builds AppWindow, wires App/Shell callbacks, starts the forwarder
                                 and the one-second ticker
  src/bridge.rs                 Bridge: runs a Launcher call off the UI thread, posts the result back
  src/events.rs                 forwarder thread: batches core Events, updates the task list and log
  src/state.rs                  RunState: which instance slugs have a game running, shared by screens
  src/keys.rs                   pure keyboard rules: key_to_screen, move_selection
  src/toasts.rs                 the toast queue: push, prune, sync to the App.toasts model
  src/models/                   pure converters from gcl-core structs to the Slint structs in types.slint
  src/screens/*.rs               one module per screen; each exposes `wire(&window, &bridge, ...)`
```

### State globals

Each screen sits behind `if App.screen == Screen.x: XScreen { }` in `app.slint`, so Rust cannot
reach a mounted screen's properties directly — there is no handle to call `get_x`/`set_x` on. The
fix is a global per screen (`InstancesState`, `InstanceState`, `AccountsState`, `SettingsState`,
declared in `ui/state.slint` and `ui/app.slint`) that both the screen and `src/screens/*.rs` can
reach: the screen binds its layout to the global's properties, and Rust calls
`window.global::<XState>()` to read and write them and to answer its callbacks. `Shell` is the one
global every screen may reach directly for two cross-cutting services: `Shell.toast(text, kind)`
and `Shell.move_selection(current, delta, len)`. It lives in `state.slint` rather than `app.slint`
because `app.slint` imports the screens, so a screen cannot import a global declared there.

Each `*State` default carries realistic preview content, so `just ui-preview screens/x.slint`
shows a filled screen with no Rust running.

### Bridge and threading

`Bridge` (`src/bridge.rs`) holds an `Arc<Launcher>` and a `slint::Weak<AppWindow>`. `Bridge::run`
spawns a plain OS thread, calls the given closure with `&Launcher` there, and on completion posts
the result to the UI thread with `weak.upgrade_in_event_loop`: `done` runs on success, the shared
error dialog opens on failure. `Bridge::run_with_error` is the same shape but always calls `done`
— with the whole `Result` — so a screen that set a "loading" flag can clear it on both paths; the
error dialog still opens first. No core call ever runs on the UI thread, and a `Weak` is captured
only inside a closure, never a strong `AppWindow` handle (that would be a reference cycle).

### Events and toasts

Core sends `Event`s (`TaskStarted`, `TaskProgress`, `TaskFinished`, `TaskFailed`, `Log`, `Warning`)
on an `mpsc` channel. `events::start_forwarder` runs one thread that blocks for the first event of
a batch, sleeps 50 ms collecting whatever else arrived, then posts one closure to the UI thread
that folds the whole batch into the `App.tasks` and `App.app_log` models in a single redraw. A
`Warning` both appends to the log and stacks a toast. A finished or failed row is not dropped by
the batch that ended it: it is stamped in a `thread_local!` age map, and the one-second `Timer` in
`src/app.rs` prunes anything older than `KEEP_DONE` (5 s). Toasts follow the same pattern in
`src/toasts.rs`, aged out after `TTL` (6 s) and capped at `MAX` (3) live at once. `RunState`
(`src/state.rs`) is the one piece of cross-screen state outside a `*State` global: an
`Arc<Mutex<HashSet<String>>>` of slugs whose game is up, so the instances list and the detail
screen agree on which rows show "Running" no matter which one started the launch.

### Keyboard

Slint delivers a key to the focused element first; only what it rejects bubbles up to the
`FocusScope` in `app.slint`. That scope calls `App.key_pressed`, which is wired to `keys::key_to_screen`
— digits 1 to 5 pick a screen, matching the numbers printed in the rail — but only while
`any_dialog_open` is false, so a modal keeps the keyboard. `keys::move_selection` is the pure
function behind every arrow-navigable list: it clamps to the list rather than wrapping, so holding
an arrow key stops at an end. Both functions are plain Rust with no Slint instance involved, so
they are covered by ordinary unit tests; the `.slint` wiring itself is verified by compiling, not
by a keyboard-driving test.

### `--smoke`

`grid-craft-launcher --smoke` opens the real window, sends three synthetic events through the
launcher's own event sink (a task that finishes, one that fails, and a warning), waits 500 ms so
the forwarder and the ticker both run at least once, then calls `slint::quit_event_loop()`. It
exercises the forwarder, the models, and window creation with no test harness of its own; `just
run-ui -- --smoke` runs it and exits 0 on success. It still needs a display — see the `testing`
skill for what that means in CI.

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

## Packaging

Two packaging paths. `packaging/build-appimage.sh` builds the Linux AppImage here on a
developer machine. cargo-dist builds the Windows and macOS archives in CI.

### AppImage layout

`packaging/build-appimage.sh` builds `gcl-ui` and `gcl-cli` in release mode, then assembles
`AppDir/`:

```
AppDir/
  AppRun                                                    entry point
  grid-craft-launcher.desktop                               plus X-AppImage-Version=<version>
  grid-craft-launcher.png                                   256 px icon
  usr/bin/grid-craft-launcher                               the Slint UI
  usr/bin/gcl                                               the CLI
  usr/share/icons/hicolor/256x256/apps/grid-craft-launcher.png
```

`AppRun` resolves its own directory, then execs `usr/bin/grid-craft-launcher` with the
arguments it got. The one exception is a first argument of `--cli`: `AppRun` drops it and
execs `usr/bin/gcl` instead. One file therefore carries both binaries, and
`./grid-craft-launcher-<version>-x86_64.AppImage --cli version list` runs the CLI.

The script runs `appimagetool` with `--appimage-extract-and-run`, so the build needs no FUSE.
It downloads the tool into `packaging/tools/` on the first run. It passes update information
(`gh-releases-zsync|sixdd6|grid-craft-launcher|latest|grid-craft-launcher-*-x86_64.AppImage.zsync`),
which lets an AppImage updater find the next GitHub release. The companion `.zsync` file needs
`zsyncmake`; the script prints a note and continues without it. The output is
`dist/grid-craft-launcher-<version>-x86_64.AppImage`.

The version comes from `[workspace.package].version` in `Cargo.toml` and nowhere else.

### cargo-dist

`dist-workspace.toml` configures cargo-dist 0.32:

- Members: the workspace root, which gives the two binary packages `gcl-ui` and `gcl-cli`.
- Targets: `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`,
  `aarch64-apple-darwin`.
- apt dependencies for the Linux runner: `libfontconfig1-dev`, `libfreetype-dev`,
  `libxkbcommon-dev`, `libwayland-dev`, `libxcb1-dev`. Slint (winit + FemtoVG) needs them.
- Installers: none. `pr-run-mode = "plan"`, so a pull request only plans.

cargo-dist writes one archive per package, not per workspace. A release therefore carries a
`gcl-ui-<target>` archive and a `gcl-cli-<target>` archive, each holding one binary. The
AppImage is the only artifact that ships both. `just dist-plan` prints what a release builds.

### Release flow

1. `just bump-version x.y.z` sets `[workspace.package].version`, refreshes `Cargo.lock`, and
   opens a new changelog section under `## Unreleased`. It runs no git command.
2. Commit, then `git tag vx.y.z` and push the tag.
3. The tag starts `.github/workflows/release.yml`, generated by cargo-dist. It builds the four
   targets and creates the GitHub release with the archives.
4. cargo-dist does not build the AppImage. Run `just appimage` locally and attach
   `dist/grid-craft-launcher-<version>-x86_64.AppImage` (and its `.zsync`, when present) to
   that release by hand.

`AppDir/`, `dist/`, and `packaging/tools/` are build output and stay out of git.
