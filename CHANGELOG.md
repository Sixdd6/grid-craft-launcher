# Changelog

All notable changes to GRID Craft Launcher are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

### Added

- Instance management: create, list, and configure instances per Minecraft version.
- Mod loaders: Fabric, Quilt, Forge, and NeoForge, installed headlessly into a shared version cache.
- Content search and install from Modrinth and CurseForge, with required dependencies resolved
  and a manual-download path for files a source will not let the launcher fetch directly.
- Modpack import from a `.mrpack` file or a CurseForge modpack zip, from the catalog or a local file.
- Per-instance `options.txt` preseed and keyed overrides that touch only the settings the user changed.
- Per-instance JVM memory settings.
- Offline accounts and Microsoft account sign-in through the device-code flow, with token refresh
  and OS keyring storage.
- Launch with a real-time log stream and a crash hint when Java exits non-zero.
- `gcl`, a command-line interface covering every feature above.
- `grid-craft-launcher`, a Slint desktop app covering the same features with a dark theme.
- A typed game-settings editor: a slider, switch, choice box, or text field per `options.txt`
  key, grouped and searchable, showing which layer each value came from and offering Reset on a
  value this layer holds. A key the launcher does not know keeps a raw row and is never dropped.
  The command line validates the same way the editor does.
- A Stop button for a running game. It asks the game to exit, waits ten seconds, then kills it;
  a stop you asked for is reported as "Stopped", not as a crash.
- Modpack search in the browser, alongside install by project id and from a file.
- GUI logging to `<root>/logs/gui.log.<date>`, rotated daily, with one line per job. `GCL_LOG`
  adds the same lines on stderr. The error dialog names the file.
- `grid-craft-launcher --screenshot <path>` saves a PNG of the window and quits.
- Linux AppImage packaging and cargo-dist archives for Windows and macOS.

### Changed

- Standard form controls (combo boxes, text fields, spin boxes) now render dark with the rest of
  the window.
- Every desktop screen is driven end to end by a headless test that builds the real window over
  a real launcher and clicks through it: create, install, launch, stop, rename and delete an
  instance; the settings editor; content add, disable, enable and remove; modpack search and
  install; and the account flows.

### Fixed

- A Minecraft version that asks for a Java major version the machine does not have now downloads
  Mojang's runtime for it. It used to accept any higher local JVM, so 1.20.1 tried to run on
  Java 25.
- A game killed by a signal reports `128 + signal` instead of exit code -1.
- The browser no longer lists its own preview rows before anything is searched.
- A modpack installed from the browser now appears in the instance list at once.

### Known issues

- CurseForge and Microsoft account features are implemented against the public API docs but not
  yet verified against the live services.
- No clipboard support in the desktop app.
- No file picker: a pack archive already on disk is named by typing its path.
- Forge before 1.13 is unsupported.
