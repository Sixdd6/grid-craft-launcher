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
| `paths` | root resolution, subdirectory constants (including `icons_dir`, `cache/icons`, and `images_dir`, `cache/images`) | none |
| `config` | `config.toml` read and write, env overrides; `jvm.gc` seeds a new instance | `instances::model` (for `GcPreset`) |
| `http` | shared reqwest client, User-Agent, retry, rate-limit backoff | none |
| `download` | content-addressed cache, parallel queue, hash checks, progress events; `download::icons` (`IconCache`) is the URL-keyed side cache for project icons: `cache/icons/<sha1(url)>.<ext>`, a 2 MiB body cap, and a CDN host allowlist (`cdn.modrinth.com`, `media.forgecdn.net`, `edge.forgecdn.net`), with one in-flight request per URL; `download::images` (`ImageCache`) is the same cache for description images — `cache/images/<sha1(url)>.<ext>`, a 5 MiB cap, **any public** `https://` host (a description names whatever image host its author chose, so no allowlist applies — private hosts are still refused); both are policies over `download::media` (`MediaCache::new(policy)` plus `Policy { allowed_hosts, max_bytes, refuse_private, dir }`), which holds the streaming, staging, capping, and single-flight code once. `media::url_allowed` is the one scheme/host/private-host check, run before the first request and again on every redirect hop through a client built with `http::HttpClient::with_redirect_guard` (at most `http::MAX_REDIRECTS` = 5 hops, a refused hop answered as `DisallowedHost`); URLs are parsed with the `url` crate, and the body cap is applied while the body streams (`stream_to_file`'s `max_bytes`), so nothing over it is written | `http`, `paths`, `events` |
| `mojang` | version manifest, version JSON, rules, argument templating, assets, `inheritsFrom` merge | `download`, `http`, `paths`, `events` |
| `java` | detect runtimes, fetch Mojang runtimes, pick by major version. The runtime component comes from the version JSON (`javaVersion.component`); `component_for_major` only guesses when the version JSON names none. `java::gc` answers which garbage collectors one JVM was built with: `probe` runs it once with `-XX:+PrintFlagsFinal -version` and keeps the flag names in `GcSupport`, and `ProbeCache::get_or_probe` remembers the answer in memory and in `cache/runtimes/gc-probe.json`, keyed by the binary's path and modification time. `Error::UnsupportedPreset` is the failure for a preset that JVM lacks | `download`, `http`, `paths` |
| `loaders` | `fabric`, `quilt`, `forge`, `neoforge`: version listing and headless install into `cache/versions/<id>.json`. `LoaderCtx` carries `http`, `root`, `dl`, `java`, `runner` (the `ProcessRunner` test seam; production uses `JavaRunner`), `mojang`. `LoaderEndpoints` holds the four base URLs | `download`, `mojang`, `java` |
| `sources` | `Source` trait (`search`, `search_packs`, `project`, `description`, `changelog` — one version's release notes, Modrinth's `changelog` field from `GET /version/{id}`, CurseForge's `GET /v1/mods/{id}/files/{fileId}/changelog` (VERIFY: envelope unconfirmed) —, `versions`, `version`, `resolve_by_hash`, `resolve_by_fingerprint`); `modrinth::Modrinth` and `curseforge::CurseForge` clients; `fingerprint` (CurseForge murmur2); `richtext` (a project description or a version's release notes in the source's markup — Modrinth markdown, CurseForge HTML — converted to `Vec<Block>` of `Heading`/`Paragraph`/`Bullet`/`Code`/`Table`/`Rule`/`Image`/`Quote` for display; five files under `sources/richtext/`: `mod.rs` holds `Block` and the output caps, `markdown.rs` the line scanner and the tag stripper that feeds it, `inline.rs` links and emphasis, `html.rs` the block builder, `tokenize.rs` tags, attributes, and entities, with `from_markdown` and `from_html` re-exported so callers still name `sources::richtext::…`); shared types in `types.rs` (`SourceId`, `SearchQuery`, `SearchHit`, `SearchPage`, `Project`, `Version`, `VersionFile`, `Dependency`, `ReleaseKind`, `VersionFilter`, `LatestFileIndex`, `page_url`, `pack_page_url`). `search_packs` is the modpack search, since a modpack is no `ContentKind`: its hits carry `SearchHit.is_pack` and a `pack_page_url` link, and the default trait body answers `Error::UnsupportedPacks` | `http`, `instances::model` (for `ContentKind`, `Loader`) |
| `content` | orchestration between `sources` and `instances::content`: `compatible_loaders`, `pick_version`, `add` (breadth-first required-dependency walk, depth 10), `check_updates` (takes `&mut Instance`; it also backfills a missing `ContentEntry.title` from the source, at most `MAX_TITLE_BACKFILL` = 25 per call, and writes `instance.toml` once at the end through `save_titles`, which re-reads the file first so a concurrent writer's edit survives; nothing backfilled means nothing written), `apply_update`, `import_manual`; `ManualDownload`, `AddOutcome` (with `conflicts`: a dependency never replaces an installed project, matched by project id and, for a pack file with no project id, by the picked file's sha1 or name), `ContentCtx` | `sources`, `instances`, `download`, `paths`, `events` |
| `modpacks` | detect and parse `.mrpack` and CurseForge pack zips into a `PackPlan`; `import`/`import_plan` build a new instance from one (an `.mrpack` file's sha1 is resolved at Modrinth first, so its content entry names a real project); `mrpack` and `curseforge` submodules hold the per-format manifest parsers; `fetch_pack` downloads a pack's own archive from a source | `sources`, `content`, `instances`, `loaders` |
| `instances` | instance layout, `instance.toml`, `instances::content` (`place_file`, `place_world`, `set_enabled`, `remove`, `installed`, `file_path`, `target_dir`), `list` skips unparsable instances with a warning; instance creation preseeds `options.txt` through `settings` | `paths`, `settings`, `sources` (for `SourceId`), `download` (for `link_or_copy`) |
| `settings` | `options.txt` preseed (`apply_preseed`) and keyed overrides (`apply_overrides_to`), plus `validate_key`/`validate_value`. Takes a game directory and a map, so it does not depend on `instances`. `settings::catalog` holds the known keys: label, group, control (slider, toggle, choice, text), default, and `parse_value`/`format_value` for the stored string. `settings::doc::merged` builds the editor's row list: one row per catalog key in catalog order, then unknown keys alphabetically, each row naming the layer that won. Layers rank override, then `options.txt`, then preseed, then catalog default — the file outranks the preseed because `apply_preseed` writes only when the file is absent. `settings::doc::validate` checks one value against its catalog entry | `paths` |
| `auth` | offline accounts (`auth::offline`) and the account store (`auth::store`, `accounts.json`); `LaunchIdentity` placeholders. `auth::msa` (`Msa`, `MsaEndpoints`, the six-step login chain, `Error`); `auth::secrets` (`SecretStore` trait, `KeyringStore`, `FileStore`, `MemoryStore`, `open_default`); `auth::session` (`LoginCtx`, `login_device_code`, `complete_chain`, `refresh_account`, `ensure_fresh`) | `paths`, `http` |
| `launch` | `launch::command::build` turns an `InstallPlan`, account, instance, and JVM settings into a `LaunchCommand`, inserting `JvmSettings.gc`'s flags right after `-Xms`/`-Xmx` and rejecting a hand-written `-XX:` collector flag in the extra arguments that names a different collector (`Error::GcConflict`) or a non-`Default` preset under a Java major below 8 (`Error::MissingGcMajor`); `launch::spawn` starts and streams it, returning a `RunningGame` that holds the child in a shared `ChildHandle` (`Arc<tokio::sync::Mutex<Option<Child>>>`) plus its `pid`; `launch::wait` polls the child every 100 ms so the handle is free between checks, and `launch::request_stop`/`force_stop` send `SIGTERM`/`SIGKILL` on unix and kill through the handle elsewhere | `instances`, `mojang`, `auth` |
| `launch/log4j` | `EventParser`: fed one line at a time (`push`, `flush` at end of stream), turns Minecraft's log4j XML events into plain `[HH:MM:SS] [thread/LEVEL]: message` lines, keeping every later message line and throwable line as its own record, verbatim. A line outside an event passes through unchanged. `spawn.rs::read_lines` runs both pipes through it before the log file, the event channel, and `crash_hint` see any output. `launch::init_local_offset()` reads the machine's UTC offset once, before any other thread starts, so timestamps are local; an unreadable offset falls back to UTC | none |
| `events` | `Progress` and `LogLine` event types and the channel | none |
| `launcher` | `Launcher` handle: owns the tokio runtime, root, config, HttpClient, event channel, and cancellation token; every binary entry point goes through it. `Launcher` is `Send + Sync`, so a GUI can hold it in an `Arc` and call it from any worker thread. Orchestrates `install_loader`, `install_instance`, `launch_instance`, `stop_instance` (asks the running game to exit, waits `STOP_GRACE` = 10 s, then kills it; `NotRunning` when this launcher started no game for that slug), `running_slugs`, `search_packs`, `launch_instance_async` (starts the game and returns a `RunningLaunch { pid, log_path, slug, wait }` at once, instead of blocking until it exits, so a caller can show the game as running and keep working; `wait` is a runtime task the caller awaits or drops), `java_for_version`, `apply_settings_overrides`, and the content and modpack flows below (`sources`, `search`, `add_content`, `list_content`, `remove_content`, `set_content_enabled`, `check_updates`, `apply_updates`, `pending_manual`, `import_manual_file`, `import_modpack_file`, `import_modpack`, `configured_or_detected_java`), plus `msa_available`, `msa_login(on_code)` (blocks on the shared cancel token), `msa_login_with_cancel(on_code, cancel)` (the same login watching a caller-supplied token, so a GUI's "cancel sign-in" button can end one login without cancelling every other download), `msa_refresh(id_or_name)`, and `secrets()` (lazy: opens the OS keyring or falls back to a file on first use; `with_secret_store` overrides it for tests). `config()` returns a `ConfigRead` guard (derefs to `Config`); hold it no longer than the read needs and never across another `Launcher` call, since a call that needs the write lock (`update_config`, most instance and content methods) would deadlock against a guard the caller is still holding. `update_config(f)` runs `f` under the write lock, saves `config.toml`, and clears the cached source list before returning, so a GUI settings screen never opens the file itself. `set_instance_override`/`unset_instance_override` edit one `options.txt` override and save `instance.toml`, and `set_game_default`/`unset_game_default` do the same for the preseed in `config.toml`; all four validate the key and value, and a known catalog key is also range- and choice-checked, so the CLI and the GUI reject the same values. `settings_rows_for_defaults` and `settings_rows_for_instance` return the merged `settings::doc::Row` list a settings screen renders; `set_instance_jvm` replaces an instance's JVM overrides (rejects `min > max`); `gc_support(slug)` resolves the Java that instance launches through `launch_java`, the one helper `prepare_launch` uses too — the instance's `java_path`, then `config.toml`'s, then the runtime the *launched version's* plan asks for, which is the loader profile merged over vanilla and not vanilla alone, installed with the usual progress events when none is present — probes it through the cached `java::gc::ProbeCache`, and returns a `GcSupportView { java_path, label, major, presets, saved }`, whose `saved` is the instance's preset as that Java runs it; `gc_support_for_java(path, saved)` is the same view for a java binary named directly, for a caller about to save that path. `set_instance_jvm_gc(slug, jvm)` writes a whole JVM block and its preset in one go, checking the preset against the java that block will save; `set_instance_gc(slug, preset)` is that call with the other fields left alone, and `prepare_launch` runs the same check on a non-`Default` preset before it builds a command, then passes the preset and the probed major into `JvmSettings`; `instance_options` reads back what `options.txt` currently holds; `list_worlds` lists an instance's `saves/` folders; `instance_summary` returns everything a detail screen needs in one call (`InstanceSummary`: whether the resolved version is installed, the Java path that would be used, and so on). `sources()` builds Modrinth always and CurseForge only when a `CURSEFORGE_API_KEY` was found when this launcher opened; the list is cached, and `update_config` clears that cache, so a key saved from the CLI or the settings screen takes effect on the next call. `Endpoints::from_env()` reads `GCL_MOJANG_BASE_URL`, `GCL_FABRIC_BASE_URL`, `GCL_QUILT_BASE_URL`, `GCL_FORGE_META_BASE_URL`, `GCL_FORGE_MAVEN_BASE_URL`, `GCL_NEOFORGE_BASE_URL`, `GCL_MODRINTH_BASE_URL`, `GCL_CURSEFORGE_BASE_URL`, and the six `Endpoints.msa` overrides `GCL_MSA_DEVICE_URL`, `GCL_MSA_TOKEN_URL`, `GCL_MSA_XBL_URL`, `GCL_MSA_XSTS_URL`, `GCL_MSA_MC_URL`, and `GCL_MSA_PROFILE_URL` (all test-only, and read in a debug build only: a release build returns `Endpoints::default()` whatever the environment holds). `GCL_NO_KEYRING=1` is the matching override for `secrets()`: it forces the file store, also debug-only, so tests never write to a developer's real keyring. `open_with_endpoints` is the test seam that takes `Endpoints` directly and reads no environment. `fetch_icon(url)` returns the cached path of a project icon, downloading it through `download::icons` on first ask; `with_icon_hosts` is its test seam, adding hosts to the icon allowlist. `fetch_image(url)` does the same for a description image through `download::images`, with no host allowlist and a 5 MiB cap; `with_image_hosts` is its test seam, which excuses the `https://` rule only. `version_notes(source, project_id, version_id)` answers one version's release notes as `richtext::Block`s, dispatching to the markdown or HTML converter the way `project_details` does. `project_details(source, project_id)` answers a `ProjectDetails { project, blocks }` — the project plus its description as `sources::richtext::Block`s, markdown at Modrinth and HTML at CurseForge — and `project_versions(source, project_id, filter)` lists a project's versions. `latest_versions(source, hits, target)` answers the newest version of each search hit for a `VersionTarget { minecraft, loader }`, four lookups in flight, every list kept in an in-memory cache keyed by source, project and filter for the launcher's lifetime; a CurseForge hit whose `latest_files` index names a file for the target is answered by one `POST /v1/mods/files` instead of a version listing, and a hit that fails answers `version: None` rather than failing the page. `install_state(slug, source, project_id, target, latest)` compares that answer with what `instance.toml` records: `NotInstalled`, `Installed`, or `Older`, the publish times read from the same cached list. Neither knows which version an instance has installed: the Versions tab's "installed" marker is computed in `gcl-ui` from `list_content`, so core stays pure | `config`, `http`, `events`, `paths`, `download`, `mojang`, `java`, `instances`, `loaders`, `auth`, `launch`, `settings`, `sources`, `content`, `modpacks` |

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
                                 SettingRow, SettingsEditor, TabBar, Rail, ToastHost, Dialog and
                                 its Confirm/Prompt/Choice/DeviceCode/CreateInstance variants,
                                 BlockList (renders a `Block` list: heading, paragraph, bullet,
                                 code, table, rule, quote, image — shared by the Description tab
                                 and the Notes modal), NotesDialog (a version's changelog in a
                                 modal built on `Dialog`, mounted only while open)
  ui/screens/                   instances.slint, instance.slint, browser.slint, accounts.slint,
                                 settings.slint, project.slint — pure layout, no logic
  src/lib.rs                    `slint::include_modules!()` plus every module below, so an
                                 integration test builds the same AppWindow the binary does
  src/main.rs                   parses args (`--smoke`, `--screenshot <path>`, `-h`, `-V`), opens
                                 `Launcher::new`, starts logging, builds the window, runs the loop
  src/logging.rs                tracing to `<root>/logs/gui.log.<date>`, daily, at `info`, plus a
                                 stderr layer when `GCL_LOG` is set
  src/app.rs                    builds AppWindow, wires App/Shell callbacks, starts the forwarder
                                 and the one-second ticker
  src/bridge.rs                 Bridge: runs a Launcher call off the UI thread, posts the result back
  src/events.rs                 forwarder thread: batches core Events, updates the task list and log
  src/state.rs                  RunState: which instance slugs have a game running, shared by screens
  src/keys.rs                   pure keyboard rules: key_to_screen, move_selection
  src/toasts.rs                 the toast queue: push, prune, sync to the App.toasts model
  src/models/                   pure converters from gcl-core structs to the Slint structs in types.slint
  src/models/settings.rs        the settings view: one editor row per catalog key, grouped and searched
  src/screens/*.rs               one module per screen; each exposes `wire(&window, &bridge, ...)`
  src/screens/settings_editor.rs the typed game-settings editor, shared by the settings screen
                                 (launcher preseed) and the instance Settings tab (overrides)
  tests/support/mod.rs          the GUI flow harness: a TestApp on a temp root with wiremock hosts
                                 and a stand-in java, click/type_into/select_combo, wait_until, run
  tests/flow_*.rs               one test binary per flow group: instances, settings, content,
                                 accounts, project
```

### State globals

Each screen sits behind `if App.screen == Screen.x: XScreen { }` in `app.slint`, so Rust cannot
reach a mounted screen's properties directly — there is no handle to call `get_x`/`set_x` on. The
fix is a global per screen (`InstancesState`, `InstanceState`, `BrowserState`, `AccountsState`,
`SettingsState`, `SettingsEditorState`, `ProjectState`, declared in `ui/state.slint`, the screen's
own file, or `ui/app.slint`) that both the screen and
`src/screens/*.rs` can
reach: the screen binds its layout to the global's properties, and Rust calls
`window.global::<XState>()` to read and write them and to answer its callbacks. `Shell` is the one
global every screen may reach directly, for one cross-cutting service:
`Shell.move_selection(current, delta, len)`. It lives in `state.slint` rather than `app.slint`
because `app.slint` imports the screens, so a screen cannot import a global declared there.

Every `*State` property starts empty, and each screen's `open`/`load` path in
`src/screens/*.rs` sets every property it owns, the empty case included, so a screen never shows
the project or instance before it. Sample content for `just ui-preview screens/x.slint` lives in
that file's own `Preview*` component, never in a global default.

There is no navigation back stack. `App.navigate` sets `App.screen` and nothing else, which is
enough for five of the six screens. The project details screen is the one that has to come back,
so `ProjectState.return_to` carries the screen that opened it — the browser row's title sets
`Screen.browser`, an installed row's source button sets `Screen.instance` — and
`ProjectState.back()` navigates there. A screen that needs the same thing copies this, rather
than growing a stack nothing else uses.

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

### `--screenshot <path>`

`grid-craft-launcher --screenshot <path>` opens the real window, writes one PNG of it, and
quits. The snapshot has to be taken inside the renderer's `AfterRendering` callback: the FemtoVG
renderer reads the OpenGL back buffer, which holds the frame only between the draw and the
buffer swap, so a plain timer gets a blank image. A 700 ms timer therefore raises a flag and
asks for a redraw, and the callback takes `Window::take_snapshot` on the frame that redraw
produces. It needs a display, and a compositor that actually draws the window: a Wayland session
gives an unmapped or occluded window no frame callback, so the notifier never fires and the run
hangs. `xvfb-run -a grid-craft-launcher --screenshot <path>` always draws.

### Logging

`logging::init(root)` builds the GUI's `tracing` subscriber: a `tracing_appender` daily file in
`<root>/logs/`, named `gui.log.<date>` in UTC, at `info`. It returns a `LogGuard` that `main`
holds until the event loop returns; dropping it stops the non-blocking writer's worker thread
and loses the tail. Setting `GCL_LOG` adds a stderr layer with that value as its `EnvFilter`
directive. `Bridge` writes one line per job — the label, the elapsed milliseconds, and the error
chain on failure — and the error dialog ends with `Details: <root>/logs/gui.log.<date>`, which
`logging::log_file(root)` rebuilds rather than hard-codes.

### GUI flow tests

`gcl-ui` is a library plus a thin binary, so `crates/gcl-ui/tests/` can build the same
`AppWindow` `main` does and click through it. `tests/support/mod.rs` is the harness: a temp root
under `GCL_ROOT`, wiremock hosts for Mojang, Fabric, Modrinth and Microsoft, a stand-in `java`
shell script, `Launcher::open_with_endpoints(...)` with a `MemoryStore`, and `gcl_ui::app::build`.
It exposes `click`, `type_into`, `select_combo`, `drag_slider`, `el`/`el_nth`, `wait_until`, and
`run(flow)`, which starts the event loop, runs the flow, quits, and re-raises any panic. `click`
and `click_nth` send real pointer events through Slint's own hit-testing rather than firing a
callback directly, so a click that lands outside an element's visible rectangle now misses it the
way a real user's would; `activate`/`activate_nth` keep the old accessible-action path for a row
inside a `ComboBox` popup, which hit-testing cannot reach. `scroll_to` scrolls an element fully
inside every viewport it overlaps before a click is attempted. Elements are addressed by the
`<Component>::<name>` ids in `docs/research/2026-09-07-ui-element-ids.md`; `crates/gcl-ui/build.rs`
emits those names when `PROFILE` is `debug`, so a release binary carries none. One Slint backend
may exist per process, so each flow group is its own test binary with one `#[test]` inside it, and
`.config/nextest.toml` gives every `flow_*` binary a 5 × 60 s timeout. No flow test opens a display
or reaches the network.

### Real-input smoke: `scripts/ui-xtest.py`

The flow tests above drive the window without a display. `scripts/ui-xtest.py` instead drives the
built `grid-craft-launcher` binary with real X pointer and keyboard events (XTest) against a
running Xvfb server, so a defect that only shows up under real hit-testing or real focus handling
— as opposed to the harness's own click path — has a second, independent check. It takes a list of
steps (`focus`, `click:x,y`, `key:name`) and reads the window back with ImageMagick's `import` and
`xdpyinfo`. The `just ui-xtest` recipe starts Xvfb on a free display (`-displayfd`), runs the app
over a throwaway root, drives two legs — create an instance through the dialog and close a dialog
with Escape, then search Modrinth in the browser and open a hit's project details from its title
— prints PASS or FAIL, and tears Xvfb down; the app's own GUI log (`job ok label=...` lines) is what proves a step actually
happened, not just that a key or click was sent. It needs Xvfb, `xdpyinfo`, ImageMagick's
`import`, and the `python3-xlib` package.

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
  cache/runtimes/gc-probe.json
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
