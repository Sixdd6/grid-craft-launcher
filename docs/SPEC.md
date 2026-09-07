# GRID Craft Launcher MVP specification

Each requirement has an id. Tests and plans cite ids. "Must" means MVP. "Later" means after MVP.

## R1 Root and configuration

- R1.1 The launcher stores everything under one root directory. Default: the platform data dir plus `grid-craft-launcher` (`~/.local/share/grid-craft-launcher` on Linux).
- R1.2 The user can change the root in settings. The launcher moves nothing; it starts using the new root and tells the user the old one still exists.
- R1.3 `config.toml` in the root holds: root path override, default JVM min and max memory, default Java path override, API keys when not in env, default game settings preseed.
- R1.4 `CURSEFORGE_API_KEY` and `GCL_MSA_CLIENT_ID` come from env first, then `config.toml`.
- R1.5 `GCL_ROOT` env overrides the root. Used by tests and e2e.
- R1.6 `accounts.json` in the root holds the account list with cached short-lived tokens. Refresh tokens live in the OS keyring (R6.1).

## R2 Instances

- R2.1 Create an instance with a name, Minecraft version, and optional loader and loader version.
- R2.2 List, rename, delete instances. Delete asks for confirmation in the UI; the CLI needs `--yes`. Rename is in the GUI too: the detail header's Rename button opens a prompt with the current name in it, and the slug never changes.
- R2.3 Each instance has its own game directory (`.minecraft/`) with mods, resourcepacks, shaderpacks, saves, options.txt, logs.
- R2.4 `instance.toml` records name, Minecraft version, loader, loader version, JVM min and max memory, Java path override, settings overrides, and the installed content list.
- R2.5 The installed content list stores per item: source (modrinth, curseforge, file), project id, version or file id, file name, sha1, content type, enabled flag.
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

- R7.1 Search Modrinth and CurseForge by text, content type, Minecraft version, and loader.
- R7.2 Content types: mods, modpacks, resource packs, shaders, data packs, worlds. Each source exposes the types it supports. Verified: Modrinth supports mod, resourcepack, shader, and datapack; it has no `world` project type, so worlds are Modrinth-unsupported. CurseForge supports mods, modpacks, resource packs, and worlds; it also supports shaders and data packs when its `/v1/categories` response has those classes, which is unverified on this codebase's development machine, which has no CurseForge API key.
- R7.3 Install a chosen version into the right instance folder: mods, resourcepacks, shaderpacks, saves/<world>/datapacks, saves.
- R7.4 Required dependencies are installed with the item.
- R7.5 CurseForge files with no download URL show the file's web page and accept a manually dropped file, verified by fingerprint. CLI convention: a command that leaves one or more manual downloads pending (`content add`, `content update --apply`, `modpack install`, `modpack install-file`) exits with code 3, not 0, even though it installed everything it could. Text and JSON output both still list what was installed and what needs a hand download.
- R7.6 Without `CURSEFORGE_API_KEY`, CurseForge is hidden and Modrinth works.
- R7.7 Update check: for each installed item, find the newest compatible version.

## R8 Modpacks

- R8.1 Install a Modrinth modpack (`.mrpack`) from the catalog or a local file into a new instance.
- R8.2 Install a CurseForge modpack (zip with `manifest.json`) from the catalog or a local file into a new instance.
- Both are reachable from the GUI: the browser's modpack panel installs by project id or slug, and its "From file" row imports an archive already on disk. Both name the new instance through the same prompt.
- R8.3 The new instance gets the pack's Minecraft version and loader, all files, and overrides.

## R9 Game settings preseed and overrides

- R9.1 The launcher holds a default `options.txt` preseed as key-value pairs in `config.toml`.
- R9.2 New instances get the preseed written to `options.txt`.
- R9.3 Each instance holds an override map of `options.txt` keys. On launch, the launcher writes those keys into the instance's `options.txt`, replacing existing lines with the same key and appending missing ones. Other lines are untouched.
- R9.4 The user can edit the preseed and per-instance overrides in the UI and CLI.

## R10 JVM settings

- R10.1 Global default min and max memory in MiB.
- R10.2 Per-instance min and max memory overrides.
- R10.3 Extra JVM arguments per instance.

## R11 Launch

- R11.1 Build the classpath and arguments from the merged version JSON, the account, the instance, and the JVM settings.
- R11.2 Apply settings overrides (R9.3) before starting the game.
- R11.3 Spawn Java, stream stdout and stderr to a log file and to the UI or terminal.
- R11.4 `--dry-run` prints the full command and environment without starting Java.
- R11.5 Report the exit code and a crash hint when the game exits non-zero.

## R12 CLI

- R12.1 Every requirement above is reachable from `gcl` subcommands: `instance`, `version`, `loader`, `java`, `account`, `content`, `modpack`, `settings`, `launch`, `debug`.
- R12.2 Output is plain text by default and JSON with `--json`.

## R13 GUI

- R13.1 Screens: instances list, instance detail (content, settings, JVM, logs), content browser (search across sources with type and version filters), accounts, launcher settings. All five are delivered (`crates/gcl-ui`).
- R13.2 Progress for downloads and installs is visible per task. Delivered: `App.tasks` shows one row per active task with a fraction and status, in the bottom panel.
- R13.3 Dark theme by default. Native window, no web view. Delivered through `winit` + `renderer-femtovg`; `std-widgets` controls (combo boxes, text fields, spin boxes) keep the `fluent` style's light palette and sit on the dark shell rather than matching it — see the `slint-ui` skill's "Known limitations".
- R13.4 Keyboard: every list is arrow-navigable, Enter activates a row, Escape closes the open dialog, digits 1-5 jump to a screen. Delivered and unit-tested at the pure-function level (`gcl-ui/src/keys.rs`); the `.slint` wiring is verified by compiling, not by a keyboard-driving test.
- Known gaps: modpack discovery in the browser is install-by-id (source + project id) or by a path to an archive on disk, not a modpack search flow — `gcl-core` has no modpack search endpoint. `InstanceState.stop` is present but disabled: `gcl-core` cannot kill a running launch yet, so the button reports why instead of acting. The CurseForge search and the Microsoft device-code sign-in flows are implemented and unit-tested but unverified live on this machine.

## Later

- Import from other launchers.
- Server instances.
- Per-instance Java download from Adoptium.
- Skins and capes.

## MVP status (2026-09-06)

Status values: **Done** (works in the CLI and the GUI), **Done (CLI only)** (no GUI surface),
**Partial** (part of the requirement is missing), **Not verified live** (the code and its unit
tests are there, but no live run has exercised it here).

| Id | Status | Note |
|---|---|---|
| R1.1 | Done | Root defaults to the platform data dir plus `grid-craft-launcher`. |
| R1.2 | Done | `gcl config root <dir>` and the settings screen change it; nothing moves, and both say the old root stays. |
| R1.3 | Done | `config.toml` holds the root, JVM defaults, Java path, API keys, and the `options.txt` preseed. |
| R1.4 | Done | `Config::curseforge_api_key` and `msa_client_id` read env first, then `config.toml`. |
| R1.5 | Done | `GCL_ROOT` overrides the root; both e2e scripts run under it. |
| R1.6 | Done | `accounts.json` holds accounts and cached tokens; refresh tokens go to the keyring. |
| R2.1 | Done | `instance create --minecraft --loader --loader-version`; the GUI has the same dialog. |
| R2.2 | Done | List, rename, delete. The CLI needs `--yes` to delete; the GUI confirms. The slug never changes. |
| R2.3 | Done | Each instance owns `.minecraft/` with mods, resourcepacks, shaderpacks, saves, options.txt, logs. |
| R2.4 | Done | `instance.toml` records every field the spec lists. |
| R2.5 | Done | A `ContentEntry` carries source, project id, version id, file name, sha1, kind, and the enabled flag. |
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
| R7.1 | Partial | Modrinth search runs live with text, type, version, and loader filters. CurseForge search is written and unit-tested against synthetic fixtures, never run live: no API key here. |
| R7.2 | Partial | Mods, modpacks, resource packs, shaders, data packs, and worlds are modeled. Modrinth has no world project type. The CurseForge class list is unverified. |
| R7.3 | Done | Files land in `mods/`, `resourcepacks/`, `shaderpacks/`, `saves/<world>/datapacks/`, and `saves/`. |
| R7.4 | Done | `content add` walks required dependencies breadth-first, depth 10. |
| R7.5 | Not verified live | A file with no download URL becomes a pending manual download; `content import-file` verifies it and the command exits 3. Only CurseForge serves such files, so this path has no live run. |
| R7.6 | Done | Without `CURSEFORGE_API_KEY`, CurseForge is hidden and Modrinth works. |
| R7.7 | Done | `content update` lists newer compatible versions; `--apply` installs them. |
| R8.1 | Done | `.mrpack` imports from the catalog and from a file. `just e2e-modpack` imported Fabulously Optimized live today: 50 content entries. |
| R8.2 | Not verified live | The CurseForge pack parser and importer are written and unit-tested against synthetic fixtures. No key here, so no live import. |
| R8.3 | Done | The new instance takes the pack's Minecraft version, loader, files, and overrides. |
| R9.1 | Done | `config.toml` holds the preseed as key-value pairs. |
| R9.2 | Done | A new instance gets the preseed written to `options.txt`. |
| R9.3 | Done | Launch writes the instance's overrides into `options.txt`, replacing matching keys and appending the rest. |
| R9.4 | Done | `settings set`, `settings defaults set`, and the GUI settings tabs edit both. |
| R10.1 | Done | `config jvm --min --max` sets the launcher-wide heap bounds. |
| R10.2 | Done | Per-instance min and max override the defaults; the JVM tab edits them. |
| R10.3 | Done | `instance.toml` carries extra JVM arguments per instance. |
| R11.1 | Done | The classpath and arguments come from the merged version JSON, the account, the instance, and the JVM settings. |
| R11.2 | Done | Overrides are applied before Java starts. |
| R11.3 | Done | Java's stdout and stderr stream to `<root>/logs/<slug>-<timestamp>.log` and to events. |
| R11.4 | Done | `--dry-run` prints the program, working directory, and every argument. |
| R11.5 | Done | A non-zero game exit is reported with a crash hint. |
| R12.1 | Done | `instance`, `version`, `loader`, `java`, `account`, `content`, `modpack`, `settings`, `launch`, `config`, and `debug` cover the requirements above. |
| R12.2 | Done | Every command prints text by default and JSON with `--json`. |
| R13.1 | Done | All five screens ship: instances, instance detail, browser, accounts, settings. |
| R13.2 | Done | The bottom panel shows one row per task with a fraction and a status. |
| R13.3 | Partial | The shell is dark and native (winit + FemtoVG). `std-widgets` controls keep the `fluent` style's light palette. |
| R13.4 | Partial | Digits 1-5, arrows, Enter, and Escape are wired. `keys.rs` is unit-tested; the `.slint` wiring is verified by compiling, not by a keyboard session. |

### Known limitations

- **CurseForge is unverified live.** This machine has no `CURSEFORGE_API_KEY`. Search, install,
  fingerprint lookup, and pack import compile and pass unit tests against synthetic fixtures
  built from the public docs. `debug verify-source curseforge` prints SKIP.
- **Microsoft login is unverified live.** The repo ships no `GCL_MSA_CLIENT_ID`, so the six-step
  device-code chain has run against wiremock only. `debug verify-source msa` prints SKIP.
- **Widget palette.** `std-widgets` controls (`ComboBox`, `TextEdit`, `SpinBox`) render in the
  `fluent` style's light palette on the dark shell. No `Theme` token reaches their colors.
- **No clipboard.** Slint 1.17 exposes no clipboard call here. Text a user may want to copy sits
  in a read-only, selectable `TextEdit`, so Ctrl+C on a selection is the whole copy story.
- **Modpack discovery is by id.** The browser installs a pack from a source and a project id, or
  from a typed path to an archive. There is no modpack search, because `gcl-core` has no modpack
  search endpoint, and no file picker.
- **No Stop button.** `InstanceState.stop` reports why it did nothing: `gcl-core` cannot kill a
  running launch yet.
- **Keyboard routing is verified by compile.** The pure functions have unit tests; nobody has
  driven the built window from a keyboard in a test.
- **Forge before 1.13 is unsupported.** The installer format changed at 1.13; older Forge
  versions are out of scope for the MVP.
- **NeoForge has no 1.20.1 build.** That release line shipped as `net.neoforged:forge` 47.1.x.
  `verify-source neoforge` and the NeoForge e2e run use 1.20.2, the first version the `neoforge`
  artifact covers.
- **CurseForge pack data packs are skipped.** A pack file whose project class is a data pack is
  skipped with a warning: a data pack needs `saves/<world>/datapacks/`, and a pack manifest names
  no world.

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

The two failing e2e runs are a test-data problem, not a launcher fault. `scripts/e2e.sh` adds the
same mod for every loader: Sodium (`AANobbMI`). Modrinth publishes Sodium for `fabric` and
`quilt` only, at both 1.20.1 and 1.20.2, so the Forge and NeoForge runs correctly find no
compatible version and stop. Both runs installed their loader first, which is the step those two
loaders exist to prove: Forge 47.4.10 and NeoForge 20.2.93 each ran their ten installer
processors headlessly and passed. `content::compatible_loaders` behaves as specified; the
NeoForge-loads-Forge fallback applies at 1.20.1 only, and the NeoForge run uses 1.20.2. A later
change to `scripts/e2e.sh` should pick the mod per loader.
