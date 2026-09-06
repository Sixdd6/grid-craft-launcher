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
- R2.2 List, rename, delete instances. Delete asks for confirmation in the UI; the CLI needs `--yes`.
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

- R6.1 Microsoft login with the device-code flow. Refresh tokens live in the OS keyring, with a file fallback and a warning when no keyring is present.
- R6.2 Offline account with a username; UUID derived from `OfflinePlayer:<name>`.
- R6.3 Multiple accounts, one selected as active.
- R6.4 If Microsoft login fails or no network, the user can still launch with an offline account.
- R6.5 Without `GCL_MSA_CLIENT_ID`, Microsoft login is hidden and offline mode stays available.

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

- R13.1 Screens: instances list, instance detail (content, settings, JVM, logs), content browser (search across sources with type and version filters), accounts, launcher settings.
- R13.2 Progress for downloads and installs is visible per task.
- R13.3 Dark theme by default. Native window, no web view.

## Later

- Import from other launchers.
- Server instances.
- Per-instance Java download from Adoptium.
- Skins and capes.
