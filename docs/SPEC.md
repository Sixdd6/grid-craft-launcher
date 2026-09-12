# GRID Craft Launcher MVP specification

Each requirement has an id. Tests and plans cite ids. "Must" means MVP. "Later" means after MVP.

## R1 Root and configuration

- R1.1 The launcher stores everything under one root directory. Default: the platform data dir plus `grid-craft-launcher` (`~/.local/share/grid-craft-launcher` on Linux).
- R1.2 The user can change the root in settings. The launcher moves nothing; it starts using the new root and tells the user the old one still exists.
- R1.3 `config.toml` in the root holds: root path override, default JVM min and max memory, default Java path override, API keys when not in env, default game settings preseed.
- R1.4 `GCL_MSA_CLIENT_ID` comes from env first, then `config.toml`. The CurseForge key comes
  from the `CURSEFORGE_API_KEY` env var, else the `GCL_CURSEFORGE_API_KEY` compiled into the
  build; `config.toml` never holds it.
- R1.5 `GCL_ROOT` env overrides the root. Used by tests and e2e.
- R1.6 `accounts.json` in the root holds the account list with cached short-lived tokens. Refresh tokens live in the OS keyring (R6.1).

## R2 Instances

- R2.1 Create an instance with a name, Minecraft version, and optional loader and loader version.
- R2.2 List, rename, delete instances. Delete asks for confirmation in the UI; the CLI needs `--yes`. Rename is in the GUI too: the detail header's Rename button opens a prompt with the current name in it, and the slug never changes.
- R2.3 Each instance has its own game directory (`.minecraft/`) with mods, resourcepacks, shaderpacks, saves, options.txt, logs.
- R2.4 `instance.toml` records name, Minecraft version, loader, loader version, JVM min and max memory, Java path override, settings overrides, and the installed content list.
- R2.5 The installed content list stores per item: source (modrinth, curseforge, file), project id, version or file id, file name, sha1, content type, enabled flag, and the project title when one is known.
- R2.6 Enable or disable a mod by renaming to `.jar.disabled`.

## R3 Minecraft versions and caching

- R3.1 Fetch the version manifest from piston-meta and cache it with ETag.
- R3.2 Cache version JSON, client jar, libraries, asset index, and asset objects under `cache/` and share them across instances.
- R3.3 Verify every download by sha1 when provided, else by size. Redownload on mismatch, three attempts.
- R3.4 Downloads run in parallel, default 8, configurable.
- R3.5 Never download a file whose hash already exists in the cache.

## R4 Mod loaders

- R4.1 Support Fabric, Quilt, Forge (1.13+), and NeoForge. NeoForge has no build for Minecraft 1.20.1 upstream; that release line shipped as `net.neoforged:forge` 47.1.x instead.
- R4.2 List available loader versions for a given Minecraft version.
- R4.3 Install a loader version into the version cache once and share it across instances.
- R4.4 Forge and NeoForge install headlessly by running the installer's processors with the launcher's Java; no GUI installer.
- R4.5 Cached installers and processor outputs are reused. A second instance on the same loader version downloads nothing.

## R5 Java

- R5.1 Detect installed Java runtimes and their major version.
- R5.2 Download the Mojang runtime component the version JSON asks for when no matching Java exists.
- R5.3 An instance may override the Java path.

## R6 Accounts

- R6.1 Microsoft login with the device-code flow. Refresh tokens live in the OS keyring, with a file fallback and a warning when no keyring is present. The fallback file is `<root>/secrets.json`, created at permission `0600` (owner read and write only) so a refresh token on disk is never readable by another user.
- R6.2 Offline account with a username; UUID derived from `OfflinePlayer:<name>`.
- R6.3 Multiple accounts, one selected as active.
- R6.4 If Microsoft login fails or no network, the user can still launch with an offline account. `gcl launch` prints a one-line hint (`use --offline-user <name> to play offline`) on any `Auth` error, so this path is discoverable from the failure itself.
- R6.5 Without `GCL_MSA_CLIENT_ID`, Microsoft login is hidden and offline mode stays available. A saved Microsoft account whose cached token has not gone stale still launches without a client id configured; only a refresh (a stale token, or a client id-driven sign-in) needs one.

## R7 Content sources

- R7.1 Search Modrinth and CurseForge by text, content type, Minecraft version, and loader. A hit opens a details screen with the project's description and its version list. The description comes from the source's own markup — Modrinth's `body` markdown, CurseForge's `GET /v1/mods/{id}/description` HTML — and is converted to headings, paragraphs, bullets, code blocks, tables, rules, quotes, and images for display. Each version row also has a Notes button that opens that version's release notes (Modrinth's `changelog` field, CurseForge's `GET /v1/mods/{id}/files/{fileId}/changelog`) in a modal, rendered the same way.
- R7.2 Content types: mods, modpacks, resource packs, shaders, data packs, worlds. Each source exposes the types it supports. Verified: Modrinth supports mod, resourcepack, shader, and datapack; it has no `world` project type, so worlds are Modrinth-unsupported. CurseForge supports mods, modpacks, resource packs, and worlds; it also supports shaders and data packs when its `/v1/categories` response has those classes, which is unverified on this codebase's development machine, which has no CurseForge API key.
- R7.3 Install a chosen version into the right instance folder: mods, resourcepacks, shaderpacks, saves/<world>/datapacks, saves.
- R7.4 Required dependencies are installed with the item.
- R7.5 CurseForge files with no download URL show the file's web page and accept a manually dropped file, verified by fingerprint. CLI convention: a command that leaves one or more manual downloads pending (`content add`, `content update --apply`, `modpack install`, `modpack install-file`) exits with code 3, not 0, even though it installed everything it could. Text and JSON output both still list what was installed and what needs a hand download.
- R7.6 With no CurseForge key — neither in the environment nor in the build — CurseForge is
  hidden and Modrinth works. A user is never asked for a key.
- R7.7 Update check: for each installed item, find the newest compatible version. The same pass fills in a missing display title on an installed item, at most 25 per run.

## R8 Modpacks

- R8.1 Install a Modrinth modpack (`.mrpack`) from the catalog or a local file into a new instance.
- R8.2 Install a CurseForge modpack (zip with `manifest.json`) from the catalog or a local file into a new instance.
- Both are reachable from the GUI: the browser's modpack panel installs by project id or slug, and its "From file" row imports an archive already on disk. Both name the new instance through the same prompt.
- R8.3 The new instance gets the pack's Minecraft version and loader, all files, and overrides.

## R9 Game settings preseed and overrides

- R9.1 The launcher holds a default `options.txt` preseed as key-value pairs in `config.toml`.
- R9.2 New instances get the preseed written to `options.txt`.
- R9.3 Each instance holds an override map of `options.txt` keys. On launch, the launcher writes those keys into the instance's `options.txt`, replacing existing lines with the same key and appending missing ones. Other lines are untouched.
- R9.4 The user can edit the preseed and per-instance overrides in the UI and CLI. Delivered: a typed editor over `settings::catalog` — a slider, switch, choice box, or text field per known key, grouped and searchable, with a Reset that shows on a value this layer holds. Layers rank override, `options.txt`, preseed, catalog default. A key the catalog does not know keeps a raw row and is never dropped. `Launcher::set_game_default` / `set_instance_override` validate every write, so `gcl settings set` rejects what the editor rejects.

## R10 JVM settings

- R10.1 Global default min and max memory in MiB.
- R10.2 Per-instance min and max memory overrides.
- R10.3 Extra JVM arguments per instance.

## R11 Launch

- R11.1 Build the classpath and arguments from the merged version JSON, the account, the instance, and the JVM settings.
- R11.2 Apply settings overrides (R9.3) before starting the game.
- R11.3 Spawn Java, stream stdout and stderr to a log file and to the UI or terminal.
- R11.4 `--dry-run` prints the full command and environment without starting Java.
- R11.5 Report the exit code and a crash hint when the game exits non-zero. A stop the user asked for is not a crash: `Launcher::stop_instance(slug)` asks the game to exit, waits `STOP_GRACE` (10 s), then kills it, and the outcome carries `stopped`, which the GUI shows as "Stopped" with no warning.

## R12 CLI

- R12.1 Every requirement above is reachable from `gcl` subcommands: `instance`, `version`, `loader`, `java`, `account`, `content`, `modpack`, `settings`, `launch`, `debug`.
- R12.2 Output is plain text by default and JSON with `--json`.

## R13 GUI

- R13.1 Screens: instances list, instance detail (content, settings, JVM, logs), content browser (search across sources with type and version filters), accounts, launcher settings. All five are delivered (`crates/gcl-ui`). A sixth screen, project details, sits behind the browser and the instance content list: it shows the description and a Versions tab, and returns to whichever screen opened it.
- R13.2 Progress for downloads and installs is visible per task. Delivered: `App.tasks` shows one row per active task with a fraction and status, in the bottom panel.
- R13.3 Dark theme by default. Native window, no web view. Delivered through `winit` + `renderer-femtovg`. `std-widgets` controls (combo boxes, text fields, spin boxes) follow the dark shell as well: `AppWindow`'s `init` sets `Palette.color-scheme = ColorScheme.dark`.
- R13.4 Keyboard: every list is arrow-navigable, Enter activates a row, Escape closes the open dialog, digits 1-5 jump to a screen. Home, End, PageUp, and PageDown move the selection on every list, and the rail shows no digits. Delivered and unit-tested at the pure-function level (`gcl-ui/src/keys.rs`); the `.slint` wiring is verified by compiling, not by a keyboard-driving test.
- R13.5 Every screen is driven end to end by a headless flow test: `crates/gcl-ui/tests/flow_{instances,settings,content,accounts,project}.rs` build the real `AppWindow` over a real `Launcher` on a temp root and click through create, install, launch, stop, the settings editor, content add and remove, modpack search and install, and the account flows. The GUI writes `<root>/logs/gui.log.<date>`, and `--screenshot <path>` saves a PNG of the window.
- Known gaps: the file picker is now partly there — the app root has a folder chooser, through `rfd` and the desktop portal; a pack archive on disk and a Java path are still named by typing. The CurseForge search and a real Microsoft sign-in are unverified against the live services on this machine; both are driven against mocks by the flow tests.

## Later

- Import from other launchers.
- Server instances.
- Per-instance Java download from Adoptium.
- Skins and capes.

## MVP status (2026-09-06, GUI rows updated 2026-09-07)

Status values: **Done** (works in the CLI and the GUI), **Done (CLI only)** (no GUI surface),
**Partial** (part of the requirement is missing), **Not verified live** (the code and its unit
tests are there, but no live run has exercised it here).

| Id | Status | Note |
|---|---|---|
| R1.1 | Done | Root defaults to the platform data dir plus `grid-craft-launcher`. |
| R1.2 | Done | `gcl config root <dir>` and the settings screen change it; nothing moves, and both say the old root stays. |
| R1.3 | Done | `config.toml` holds the root, JVM defaults, Java path, API keys, and the `options.txt` preseed. |
| R1.4 | Done | `Config::msa_client_id` reads env first, then `config.toml`; `Config::curseforge_api_key` reads env, then the build-time key. |
| R1.5 | Done | `GCL_ROOT` overrides the root; both e2e scripts run under it. |
| R1.6 | Done | `accounts.json` holds accounts and cached tokens; refresh tokens go to the keyring. |
| R2.1 | Done | `instance create --minecraft --loader --loader-version`; the GUI has the same dialog. |
| R2.2 | Done | List, rename, delete. The CLI needs `--yes` to delete; the GUI confirms. The slug never changes. |
| R2.3 | Done | Each instance owns `.minecraft/` with mods, resourcepacks, shaderpacks, saves, options.txt, logs. |
| R2.4 | Done | `instance.toml` records every field the spec lists. |
| R2.5 | Done | A `ContentEntry` carries source, project id, version id, file name, sha1, kind, the enabled flag, and an optional `title`. An entry written before that key existed has none until `content update` backfills it. |
| R2.6 | Done | Enable and disable rename the file to and from `.disabled`. |
| R3.1 | Done | The piston-meta manifest is fetched and cached with its ETag. |
| R3.2 | Done | Version JSON, client jar, libraries, asset index, and assets live under `cache/` and are shared. |
| R3.3 | Done | Every download verifies sha1, else size, and retries three times. |
| R3.4 | Done | The queue runs 8 downloads at once; `config.toml` changes the number. |
| R3.5 | Done | The object store is content-addressed; a hash already present is never fetched again. |
| R4.1 | Done | Fabric, Quilt, Forge (1.13+), and NeoForge install. All four ran live today. |
| R4.2 | Done | `loader list <mc> --loader <name>` lists the builds, newest first. |
| R4.3 | Done | A loader writes `cache/versions/<id>.json` once and every instance reuses it. |
| R4.4 | Done | Forge and NeoForge run the installer processors headlessly with the launcher's Java. |
| R4.5 | Done | Installers and processor outputs stay in `cache/installers/` and the version cache. |
| R5.1 | Done | `java list` detects runtimes and reports each major version. |
| R5.2 | Done | The Mojang runtime component named by the version JSON downloads when no Java matches. |
| R5.3 | Done | `instance.toml` takes a Java path override; the GUI JVM tab edits it. |
| R6.1 | Not verified live | The device-code chain and the keyring store are written and unit-tested. No live sign-in has run here: the repo ships no `GCL_MSA_CLIENT_ID`. |
| R6.2 | Done | Offline accounts derive the UUID from `OfflinePlayer:<name>`. |
| R6.3 | Done | Many accounts, one active; `account select` and the accounts screen switch. |
| R6.4 | Done | An `Auth` failure prints `use --offline-user <name> to play offline`. |
| R6.5 | Done | Without a client id, Microsoft login is hidden and offline mode works. |
| R7.1 | Partial | Text, type, version, and loader filters work at both sources, and a hit opens a details screen with the description and the version list. The description renders tables, rules, quotes, nested bullets, and images, from any https host. A version's Notes button opens its release notes in a modal, rendered the same way. Modrinth verified live, description and single-version changelog included; CurseForge not verified live (no key), and both its description and changelog envelopes are unverified. |
| R7.2 | Partial | Mods, modpacks, resource packs, shaders, data packs, and worlds are modeled. Modrinth has no world project type. The CurseForge class list is unverified. |
| R7.3 | Done | Files land in `mods/`, `resourcepacks/`, `shaderpacks/`, `saves/<world>/datapacks/`, and `saves/`. |
| R7.4 | Done | `content add` walks required dependencies breadth-first, depth 10. |
| R7.5 | Not verified live | A file with no download URL becomes a pending manual download; `content import-file` verifies it and the command exits 3. Only CurseForge serves such files, so this path has no live run. |
| R7.6 | Done | With no key in the environment or the build, CurseForge is hidden and Modrinth works; nothing asks a user for one. |
| R7.7 | Done | `content update` lists newer compatible versions; `--apply` installs them. The same pass backfills a missing entry title, at most 25 per run, and writes `instance.toml` once, only when something changed. |
| R8.1 | Done | `.mrpack` imports from the catalog and from a file. `just e2e-modpack` imported Fabulously Optimized live today: 50 content entries. |
| R8.2 | Not verified live | The CurseForge pack parser and importer are written and unit-tested against synthetic fixtures. No key here, so no live import. |
| R8.3 | Done | The new instance takes the pack's Minecraft version, loader, files, and overrides. |
| R9.1 | Done | `config.toml` holds the preseed as key-value pairs. |
| R9.2 | Done | A new instance gets the preseed written to `options.txt`. |
| R9.3 | Done | Launch writes the instance's overrides into `options.txt`, replacing matching keys and appending the rest. |
| R9.4 | Done | A typed editor over `settings::catalog` — slider, switch, choice box, or text field per key, grouped and searchable, offering Reset on a value this layer holds. `settings set` and `settings defaults set` route through the same validating `Launcher` methods. The catalog is checked against a real 26.2 Fabric `options.txt`: `fov` shows in degrees over a stored float, choice tokens keep the quotes the file writes, and `gcl settings set` accepts a bare choice token. |
| R10.1 | Done | `config jvm --min --max` sets the launcher-wide heap bounds. |
| R10.2 | Done | Per-instance min and max override the defaults; the JVM tab edits them. |
| R10.3 | Done | `instance.toml` carries extra JVM arguments per instance. |
| R11.1 | Done | The classpath and arguments come from the merged version JSON, the account, the instance, and the JVM settings. |
| R11.2 | Done | Overrides are applied before Java starts. |
| R11.3 | Done | Java's stdout and stderr stream to `<root>/logs/<slug>-<timestamp>.log` and to events. Minecraft writes log4j XML; a streaming parser turns it into plain `[HH:MM:SS] [thread/LEVEL]: message` lines, in local time, before either destination sees it. |
| R11.4 | Done | `--dry-run` prints the program, working directory, and every argument. |
| R11.5 | Done | A non-zero game exit is reported with a crash hint. A stop the user asked for reports "Stopped" and raises no warning. The hint scans the whole log tail for each marker, in priority order, from the end, so a stack trace's real cause wins over an earlier benign error. |
| R12.1 | Done | `instance`, `version`, `loader`, `java`, `account`, `content`, `modpack`, `settings`, `launch`, `config`, and `debug` cover the requirements above. |
| R12.2 | Done | Every command prints text by default and JSON with `--json`. |
| R13.1 | Done | All five screens ship: instances, instance detail, browser, accounts, settings. A project details screen (Description and Versions tabs) opens from a search row's title and from an installed row's source button, and returns to whichever screen opened it. |
| R13.2 | Done | The bottom panel shows one row per task with a fraction and a status. |
| R13.3 | Done | The shell is dark and native (winit + FemtoVG), and `std-widgets` controls follow it: `Palette.color-scheme = ColorScheme.dark` in `AppWindow`'s `init`. |
| R13.4 | Done | Digits 1-5, arrows, Home, End, PageUp, PageDown, Enter, and Escape are wired; the rail shows no digits. `keys.rs` is unit-tested. The wiring is verified by compile and Slint's documented event routing, not by a live keyboard. |
| R13.5 | Done | Five headless flow binaries drive the real window over a real launcher: instances (create, install, launch, stop, rename, delete), settings, content, accounts, and project (search, open details, install a version over another one). Each click goes through real pointer hit-testing rather than an accessible action. The GUI logs to `<root>/logs/gui.log.<date>`; `--screenshot <path>` saves a PNG. `just ui-xtest` drives the built app with real X input under Xvfb; verified: create via the dialog, a never-launched instance's Logs tab starts empty, a click after closing a dialog with Escape still lands, a number-key shortcut works after a screen change past a text field, the browser starts empty on open, and a search row's title opens the project details screen. |

### Known limitations

- **CurseForge is unverified live.** This machine has no `CURSEFORGE_API_KEY`. Search, install,
  fingerprint lookup, and pack import compile and pass unit tests against synthetic fixtures
  built from the public docs. `debug verify-source curseforge` prints SKIP.
- **The CurseForge description envelope is unverified.** `GET /v1/mods/{id}/description` is read
  as `{"data": "<html string>"}`, which is what the public docs describe and what every other
  endpoint this client parses looks like. No live response has confirmed it, so
  `tests/fixtures/curseforge/get_mod_description.json` is synthetic. Modrinth's side is verified:
  its description is the `body` of `GET /project/{id}`.
- **The CurseForge changelog envelope is unverified.** `GET /v1/mods/{id}/files/{fileId}/changelog`
  is read as the same `{"data": "<html string>"}` shape as `description`, on the same assumption
  and with the same caveat: `tests/fixtures/curseforge/get_file_changelog.json` is synthetic.
  Modrinth's side is verified: a version's release notes are the `changelog` field of
  `GET /version/{id}`.
- **A description image is fetched from any `https://` host, with no allowlist.** This is a
  deliberate choice: a description names whatever image host its author picked, so the icon
  cache's CDN allowlist does not apply here. The guards that remain are `https` only, a 5 MiB
  streaming cap, and a refusal of IP literals and `localhost`/`*.internal`/`*.local` hosts.
- **`cache/icons` and `cache/images` are never pruned.** A project icon or description image is
  cached by its URL and kept. Nothing evicts either — `cleanup_partials` sweeps only `*.part`
  files — so both directories grow with the number of distinct URLs the browser has shown.
- **Microsoft login is unverified live.** The repo ships no `GCL_MSA_CLIENT_ID`, so the six-step
  device-code chain has run against wiremock only. `debug verify-source msa` prints SKIP.
- **No clipboard.** Slint 1.17 exposes no clipboard call here. Text a user may want to copy sits
  in a read-only, selectable `TextEdit`, so Ctrl+C on a selection is the whole copy story.
- **No file picker.** A pack archive already on disk is named by typing its path into a field.
  Modpack search itself works, in the browser's modpack kind.
- **Keyboard routing is verified by compile, plus one real-input pass.** The pure functions have
  unit tests, the flow tests press keys to drive a `ComboBox`, and `just ui-xtest` has driven a
  number-key shortcut through real X input after a screen change; no automated run covers the
  whole window from a keyboard on every path.
- **A debounce cannot be driven from a flow test.** `pump()` hands the event loop no time, and
  the system-time backend refuses `mock_elapsed_time` with a real duration, so a flow test drags
  a slider (which saves on release) and leaves the debounced path to a unit test.
- **`--screenshot` needs a compositor that draws.** A Wayland session gives an unmapped or
  occluded window no frame callback, so the `AfterRendering` notifier never fires. Run it under
  `xvfb-run -a`.
- **Forge before 1.13 is unsupported.** The installer format changed at 1.13; older Forge
  versions are out of scope for the MVP.
- **NeoForge has no 1.20.1 build.** That release line shipped as `net.neoforged:forge` 47.1.x.
  `verify-source neoforge` and the NeoForge e2e run use 1.20.2, the first version the `neoforge`
  artifact covers.
- **CurseForge pack data packs are skipped.** A pack file whose project class is a data pack is
  skipped with a warning: a data pack needs `saves/<world>/datapacks/`, and a pack manifest names
  no world.

### Plan 7 status (2026-09-07)

The usable-app plan closed on 2026-09-07 on the same machine. What it added or fixed:

- The game settings editor is typed (R9.4): a control per catalog key, layers ranked
  override > `options.txt` > preseed > default, and the CLI validating the same way.
- The instance detail screen can stop a running game (R11.5).
- The browser searches modpacks, not only installs one by id (R8.1).
- Every screen is driven by a headless flow test (R13.5), and the GUI writes
  `<root>/logs/gui.log.<date>`.
- `std-widgets` controls render dark, which closes the R13.3 gap.

| Command | Result | Key line |
|---|---|---|
| `just check` | PASS | `Summary [3.876s] 766 tests run: 766 passed, 0 skipped` |
| `just deny` | PASS | `advisories ok, bans ok, licenses ok, sources ok` |
| `just lint-claude` | PASS | `PASS: claude files have frontmatter` |
| `just e2e` (Fabric, MC 1.20.1) | PASS | `PASS dry-run launch`, `PASS check classpath files exist (60 entries)` |
| `gcl instance create demo7 --minecraft 1.20.1 --loader fabric` (real root) | PASS | `created demo7 (demo7)`, 0.21 s |
| `gcl launch demo7 --offline-user Player --dry-run` (real root) | PASS | `program: <root>/cache/runtimes/java-runtime-gamma/linux/bin/java`, 34.52 s |
| `grid-craft-launcher --screenshot <path>` under `xvfb-run` | PASS | a 1200 × 760 PNG of the dark shell with the instance row |

The dry-run downloaded `java-runtime-gamma` rather than using this machine's OpenJDK 25, which
is the behaviour Task 4 fixed: 1.20.1 asks for Java 17, and a higher local JVM is no longer
accepted. The screenshot run needs `xvfb-run`: under Wayland the window gets no frame callback,
so the `AfterRendering` notifier never fires.

### Plan 9 status (2026-09-07)

The mod details plan closed on 2026-09-07 on the same machine. What it added:

- A project details screen with a Description tab and a Versions tab (R7.1, R13.1). The
  description is the source's own markup converted to display blocks; the Versions tab marks the
  installed version and installs another one over it.
- `ContentEntry.title` (R2.5), set on install and backfilled by `content update` (R7.7). The
  instance's content list shows the title over the file name, sorted by title.
- Project icons on browser search rows, cached under `cache/icons` by URL.
- A fifth GUI flow binary, `flow_project` (R13.5), and a browse leg in `just ui-xtest` that
  opens a details page with real X input.

| Command | Result | Key line |
|---|---|---|
| `just check` | PASS | `Summary [4.177s] 907 tests run: 907 passed, 0 skipped` |
| `just deny` | PASS | `advisories ok, bans ok, licenses ok, sources ok` |
| `just lint-claude` | PASS | `PASS: claude files have frontmatter` |
| `just ui-xtest` | PASS | `ok: job "Search"`, `ok: job "Project details"` |
| `just e2e` (Fabric, MC 1.20.1) | PASS | `PASS dry-run launch`, `PASS check classpath files exist, none twice (61 entries)` |
| `just verify-api modrinth` | PASS | `PASS project sodium (AANobbMI)`, `PASS versions sodium 1.20.1 fabric (13)` |
| `just verify-api curseforge` | SKIP | no `CURSEFORGE_API_KEY`; the description envelope stays unverified |

### Plan 10 status (2026-09-08)

The description-rendering plan closed on 2026-09-08 on the same machine. What it added (R7.1):

- Description blocks: `Block` gained `Table`, `Rule`, `Image`, and `Quote`, and `Bullet` gained
  a nesting `depth`. `BlockList` renders all of them — a table as a header row over its body
  rows, a rule as a divider line, a quote with a left bar, a bullet indented per level, and an
  image sized to the column. `sources::richtext` is now five files (`mod`, `markdown`, `inline`,
  `html`, `tokenize`) instead of one, with the same public `from_markdown`/`from_html` paths.
- Description images: `download::images` fetches from any `https://` host (no CDN allowlist,
  unlike icons), capped at 5 MiB while streaming and decoded under a 4096×4096 limit, then
  downscaled to 1600px on the long side before it reaches the screen. At most 20 images are
  fetched per description or changelog. `cache/images` has no eviction, the same gap
  `cache/icons` carries. Every redirect hop is re-checked against the scheme/host/private-host
  rules, not only the URL a caller passed, and a private host (an IP literal, `localhost`,
  `*.internal`, `*.local`) is refused.
- Version notes: `Source::changelog(project_id, version_id)` — Modrinth's `changelog` field from
  `GET /version/{id}`, CurseForge's `GET /v1/mods/{id}/files/{fileId}/changelog` (VERIFY: the
  response envelope is assumed, not confirmed live, the same gap `description` already carries).
  `Launcher::version_notes` converts either to `Block`s. A version row in the Versions tab has a
  Notes button that opens them in a modal (`NotesDialog`), closed by its Close button or Escape.
- Browser rows: a search or modpack result row now wraps its full title instead of eliding it,
  drops the page-url line, and collapses `\n`/`\r` in its description to spaces.

| Command | Result | Key line |
|---|---|---|
| `just check` | PASS (at Task 6, 2cb65aa) | `Summary [7.300s] 980 tests run: 980 passed, 0 skipped` |
| `just ui-xtest` | PASS (at Task 6, 2cb65aa) | `ok: job "Search"`, `ok: job "Project details"` |
| `just verify-api modrinth` | PASS | `PASS search sodium (5 hits)`, `PASS project sodium (AANobbMI)`, `PASS versions sodium 1.20.1 fabric (13)`. `debug verify-source modrinth` exercises no `changelog` call, so the single-version release-notes field stays verified only against `tests/fixtures/modrinth/`, not a live response. |
| `just verify-api curseforge` | SKIP | no `CURSEFORGE_API_KEY`; both the description and the changelog envelopes stay unverified |

### Plan 11 status (2026-09-08)

The garbage collector preset plan closed on 2026-09-08 on the same machine. Design doc:
`docs/superpowers/specs/2026-09-08-gc-presets-design.md`. What it added (R5, R10.3, R11.1, R12.1,
R13.1):

- An instance can pick a garbage collector: Default, Serial, Parallel, G1, ZGC, ZGC
  (generational), or Shenandoah. `instances::model::GcPreset` holds the token, and
  `InstanceJvm.gc` stores it (`instance.toml`'s `[jvm]` table gains an optional `gc` key; absent
  means Default). `config.jvm.gc` seeds the preset a new instance is created with.
- The picker offers only collectors the instance's own Java actually has. `java::gc::probe` runs
  that JVM once with `-XX:+PrintFlagsFinal -version`, and `ProbeCache` remembers the answer in
  memory and in `cache/runtimes/gc-probe.json`, keyed by the binary's path and modification
  time, with one probe in flight per key. `Launcher::gc_support(slug)` installs the runtime
  first when none is present, then probes it.
  `instances::model::supported_presets(major, flags)` turns the probe's flag names into the
  preset list a picker shows.
  A plain ZGC selection on Java 23 and later saves and launches as the generational preset,
  since both expand to the same flag there; the picker shows one ZGC row from 23 on, not two.
- Launch inserts `GcPreset::flags(major)` after `-Xms`/`-Xmx` and refuses to start when a
  hand-written `-XX:` collector flag in the extra JVM arguments names a different collector than
  the preset (`launch::Error::GcConflict`), or when a non-Default preset meets a Java major below
  8 (`launch::Error::MissingGcMajor`). `Launcher::set_instance_gc(slug, preset)` runs the same
  probe-and-check before it saves, so a rejected preset never reaches `instance.toml`.
  `Launcher::set_instance_jvm` (the heap Save button's call) keeps the saved preset untouched;
  only `set_instance_gc` changes it.
  `Default` skips the probe at launch and at save: there is no flag to check.
- CLI: `gcl config set-jvm --gc <preset>` seeds new instances; `gcl instance jvm <slug> --gc
  <preset>` changes an existing instance's preset alongside min, max, and extra args, rolling
  back the whole command on a refusal; `gcl instance gc <slug>` prints the probed Java and the
  presets it supports, flagging a saved preset the current Java has lost.
- GUI: the JVM tab's `gc_combo` shows the probed Java's label (or "Preparing Java…" while a
  runtime installs) and saves on selection, with no Save button — unlike the heap fields, which
  save only when their own Save button is pressed. An amber line reads "Unavailable: <preset>"
  when the saved preset is not one the current Java supports.

| Command | Result | Key line |
|---|---|---|
| `just check` | PASS (at a942281) | `Summary [7.878s] 1048 tests run: 1048 passed, 0 skipped` |
| `just lint-claude` | PASS | `PASS: claude files have frontmatter` |

### Verification record

Every command below ran on 2026-09-06 on Linux (Fedora/Nobara, kernel 7.2.3), against the live
services where the command talks to one. `.env` holds no CurseForge key and no Microsoft client
id on this machine.

| Command | Result | Key line |
|---|---|---|
| `just check` | PASS | `Summary [3.530s] 708 tests run: 708 passed, 0 skipped` |
| `just deny` | PASS | `advisories ok, bans ok, licenses ok, sources ok` |
| `just lint-claude` | PASS | `PASS: claude files have frontmatter` |
| `just e2e` (Fabric, MC 1.20.1) | PASS | `PASS create instance`, `PASS install loader`, `PASS content add`, `PASS dry-run launch`, `PASS check classpath files exist (60 entries)` |
| `just e2e-modpack` (Fabulously Optimized) | PASS | `PASS install modpack`, `50 content entries`, `PASS dry-run launch`, `PASS check classpath files exist (68 entries)` |
| `GCL_E2E_LOADER=quilt just e2e` (MC 1.20.1) | PASS | `PASS install loader`, `PASS content add`, `PASS check classpath files exist (66 entries)` |
| `GCL_E2E_LOADER=neoforge just e2e` (MC 1.20.2, mod Jade) | PASS | `PASS install loader`, `PASS content add`, `PASS dry-run launch`, `PASS check classpath files exist (106 entries)` |
| `GCL_E2E_LOADER=forge just e2e` (MC 1.20.1, mod JEI) | PASS | `PASS install loader`, `PASS content add`, `PASS dry-run launch`, `PASS check classpath files exist (81 entries)` |
| `just run-ui -- --smoke` | PASS | exit 0 |
| `just appimage-smoke` | PASS | exit 0 |
| `./dist/grid-craft-launcher-0.1.0-x86_64.AppImage --appimage-extract-and-run --cli --version` | PASS | `gcl 0.1.0` |
| `just verify-api mojang` | PASS | `PASS manifest (910 versions)`, `PASS version 26.2 (131 libraries)` |
| `just verify-api fabric` | PASS | `PASS list fabric 1.20.1 (253 versions)`, `PASS profile fabric-loader-0.19.5-1.20.1` |
| `just verify-api quilt` | PASS | `PASS list quilt 1.20.1 (307 versions)`, `PASS profile quilt-loader-0.20.0-beta.9-1.20.1` |
| `just verify-api forge` | PASS | `PASS list forge 1.20.1 (132 versions)`, `PASS installer 47.4.10 (10 processors)` |
| `just verify-api neoforge` | PASS | `PASS list neoforge 1.20.2 (79 versions)`, `PASS installer 20.2.93 (10 processors)` |
| `just verify-api modrinth` | PASS | `PASS search sodium (5 hits)`, `PASS versions sodium 1.20.1 fabric (13)`, `PASS hash lookup ...` |
| `just verify-api curseforge` | SKIP | `SKIP curseforge (no CURSEFORGE_API_KEY)` |
| `just verify-api msa` | SKIP | `SKIP msa (no GCL_MSA_CLIENT_ID)` |

All four loaders pass `just e2e` end to end. `scripts/e2e.sh` picks the mod per loader, because
no single mod publishes for all four: Sodium for Fabric and Quilt, JEI for Forge, Jade for
NeoForge. An earlier run of this task used Sodium for every loader and failed the Forge and
NeoForge runs at `content add`. That was test data, not launcher code: Modrinth publishes Sodium
for `fabric` and `quilt` only, at 1.20.1 and 1.20.2 alike, so `content::compatible_loaders`
correctly found no candidate. The NeoForge-loads-Forge fallback applies at 1.20.1 only, and the
NeoForge run uses 1.20.2. `scripts/e2e.sh` now takes `GCL_E2E_MOD` to override the default.

### Plan 12 status (2026-09-08)

Design doc: `docs/superpowers/specs/2026-09-08-browser-latest-design.md`. Each browser search
row now shows the newest version for the search target's Minecraft version and loader, whether
the mod is installed there, and whether the installed copy is older, with an Add/Update/Installed
button.

- `SearchHit.latest_files: Vec<LatestFileIndex>` maps CurseForge's `latestFilesIndexes`, so a
  CurseForge hit can name its newest file for a given Minecraft version and loader without a
  second request. Modrinth has no such field; `latest_files` stays empty there. **VERIFY: the
  index entry shape is taken from the public docs, unconfirmed against a live response (no
  `CURSEFORGE_API_KEY` on this machine).**
- `content::pick_latest` picks the newest version release-first, without an install target, so
  a browser row can answer "latest" the same way `content::add` would install it.
- `Launcher::latest_versions`/`latest_versions_each(source, hits, target)` resolve up to four
  hits at once, cache each project's version list in memory for the launcher's lifetime, and
  take the CurseForge index fast path when it names a match. A vanilla target
  (`VersionTarget { loader: Loader::None, .. }`) answers no latest for a mod: a vanilla instance
  runs no mods.
- `Launcher::install_state(slug, source, project_id, target, latest)` answers `NotInstalled`,
  `Installed`, or `Older`, comparing publish times from the same cached list; a pack-imported
  entry is matched by `content::entry_holding_file`. Equal publish times read as `Installed`.
- Browser rows: a state line under the title, an `↑` marker when older, and `row_install`
  reading Add / Update / a muted disabled Installed. A per-page job resolves this after search,
  guarded by `latest_generation` beside `icon_generation`, and re-runs on a target change.

Commits: `e922447` (CurseForge index parsing), `41fbf9c` (`pick_latest`), `28e04b1` (resolver
and comparator), `371e542` (browser UI), `a79eaae` (vanilla-target and incremental-answer fix).
`just lint-claude` PASS. `just check` was not re-run for this docs task; Task 5's flow test and
the fixes in `a79eaae` already exercised the feature end to end. CurseForge's `latestFilesIndexes`
shape stays VERIFY without a key.

### Plan 13 status (2026-09-08)

Design doc: `docs/superpowers/specs/2026-09-08-browser-columns-design.md`. Browser results are
now a table, and a dense list stripes its rows.

- `SearchHit.updated` carries the hit's last-changed date, from Modrinth's `date_modified` and
  CurseForge's `dateModified` (RFC 3339, empty when the source names none). `models::search_row`
  shortens it to `YYYY-MM-DD` for the row.
- Columns come from theme tokens: `col-author` 140 px, `col-downloads` 100 px right-aligned,
  `col-date` 110 px, `col-action` 170 px, and `col-gap` 16 px between them. `results_header` and
  `pack_results_header` name them; both are hidden while the list is empty. The header over the
  button column is the one place the target is named — "Version for 1.20.1 fabric" — so no row
  repeats it.
- The state line under a row's button is a short phrase that fits the column: `Latest <n>`,
  `Installed <n>`, `Installed <n> ↑` when the copy is behind, `No version`, `Checking…` while
  the job has not answered, and `Could not check` when it answered with nothing usable.
- `row_title` is not a click target: no `TouchArea`, no underline, no pointer cursor. It keeps
  an `accessible-label` and a default action for a screen reader; `ListRow`'s own area opens
  the details for a pointer.
- `ListRow` gained `alt` (odd-row tint from `Theme.surface-alt`, behind hover and selection)
  and `column-spacing`. Its `accessible-description` reads back `alt`, which is how a flow test
  reads a stripe.
- Install, Update and Add patch the affected row with `set_row_data`. Nothing calls `set_rows`
  with a fresh model after a search has painted, and a target change resets each row in place,
  so a decoded icon is never thrown away and the list never blinks.
- `flow_content` pins all of it: the header is absent before a search and present after,
  `BrowserScreen::title_touch` does not exist, the state lines read as above, the row model and
  every decoded icon survive an Add with no row falling back to "Checking…", and both the
  browser results and an instance's content list alternate `alt`.

`just ui-xtest`'s browse leg clicked `305,114` for the first hit's title. The header row moved
every result down, so that click landed on the header and opened nothing; it is `305,138` now.

Commits: `6efca93` (the date field), `fb93e70` (columns, stripes, in-place updates), and this
task's fix and tests. `just check` PASS (1089 tests). `just ui-xtest` PASS.
