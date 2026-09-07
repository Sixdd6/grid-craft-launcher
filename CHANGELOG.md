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
- Linux AppImage packaging and cargo-dist archives for Windows and macOS.

### Known issues

- CurseForge and Microsoft account features are implemented against the public API docs but not
  yet verified against the live services.
- Standard form controls (combo boxes, text fields, spin boxes) use Slint's `fluent` style and its
  light palette, so they look lighter than the rest of the dark-themed window.
- No clipboard support in the desktop app.
- Modpack discovery works by project id, not by search, for the source catalogs.
- No stop button for a running instance; close the game window or process to end a launch.
- Forge before 1.13 is unsupported.
