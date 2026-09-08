# Changelog

All notable changes to GRID Craft Launcher are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

- Browser: a search row now shows the newest version for the target instance's Minecraft
  version and loader, whether it is already installed, and whether the installed copy is
  older, marked with an `↑`. The row's button reads Add, Update, or a disabled Installed.
  Update installs the newest version over the old one. A vanilla target shows no latest for a
  mod, since a vanilla instance runs no mods.
- JVM settings: an instance can pick a garbage collector preset — Default, Serial, Parallel, G1,
  ZGC, ZGC (generational), or Shenandoah. The picker offers only collectors the instance's own
  Java supports, found by probing that JVM once and caching the answer. Launch applies the
  matching flags and refuses to start rather than hand Java a collector it does not have, or a
  hand-written `-XX:` flag that fights the preset. `gcl config set-jvm --gc`, `gcl instance jvm
  --gc`, and `gcl instance gc` on the CLI; a `gc_combo` on the GUI's JVM tab that saves on
  selection.
- Browser and instance detail: a mod, resource pack, shader, or data pack now has a details
  screen. It shows the project's description — the source's markdown or HTML turned into
  headings, paragraphs, bullets, code blocks, tables, rules, quotes, and images — and a Versions
  tab that lists the versions the target instance can run, marks the one installed, and installs
  any other one over it. A search row's title opens it; so does the source button on an
  installed content row, which returns to the instance it came from.
- Project details: a description or a version's release notes now render tables, horizontal
  rules, quotes, nested bullet lists, and images from any `https://` host, not only headings,
  paragraphs, bullets, and code. An image is capped at 5 MiB while it downloads and 4096×4096
  while it decodes, then shown at up to 1600px on its long side; at most 20 images load per
  description or changelog. A version row in the Versions tab has a Notes button that opens its
  release notes in a modal, closed by Close or Escape.
- Browser: a search or modpack result row now wraps its full title instead of eliding it, drops
  the page-url line, and folds an embedded line break in its description to a space.
- Instance detail: an installed content row shows the project's title on top and the file name
  under it, sorted by title. `instance.toml` gained an optional `title` per entry. An entry
  written before the key existed has none; `content update` fills in up to 25 of them
  per run and saves the file once.
- Browser: a search row shows the project's icon. Icons are downloaded once into `cache/icons`
  and reused; the download is capped at 2 MiB and allowed only from the sources' own CDNs over
  HTTPS.
- Launch: a loader that repeats a vanilla library no longer puts the jar on the classpath twice. NeoForge 1.21 and Forge refused to start with `Duplicate key`.
- Content: a dependency pinned to another version of an installed mod is reported as a conflict instead of placing a second jar. Modpack files resolve to their Modrinth project when the hash is known.
- Loaders: a Forge processor output whose entries match is accepted when the host zlib changes the compressed bytes. Fedora and Arch ship zlib-ng.

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
- The game log and the Logs tab show plain text. Minecraft writes log4j XML events; a new
  streaming parser turns each one into a normal `[HH:MM:SS] [thread/LEVEL]: message` line, with
  every throwable line kept verbatim.
- Log timestamps use local time, not UTC.
- `just ui-xtest`: a real-input smoke test. It drives the desktop app with actual X pointer and
  keyboard events under Xvfb, instead of the accessible actions the flow tests use.

### Changed

- Standard form controls (combo boxes, text fields, spin boxes) now render dark with the rest of
  the window.
- Every desktop screen is driven end to end by a headless test that builds the real window over
  a real launcher and clicks through it: create, install, launch, stop, rename and delete an
  instance; the settings editor; content add, disable, enable and remove; modpack search and
  install; and the account flows.
- The flow test harness clicks and types through real pointer and keyboard events, with hit
  testing, instead of firing callbacks through accessible actions.
- The `options.txt` catalog was checked against a real 26.2 Fabric file: `fov` shows and edits
  in degrees rather than the stored -1.0 to 1.0 float, `renderClouds` and `mainHand` keep the
  quotes the file writes around their tokens, and several other keys and ranges were corrected.
  New 26.2 keys were added, and `gcl settings set` now accepts a bare choice token (`fast`, not
  only `"fast"`) and stores it quoted.

### Fixed

- A Minecraft version that asks for a Java major version the machine does not have now downloads
  Mojang's runtime for it. It used to accept any higher local JVM, so 1.20.1 tried to run on
  Java 25.
- A game killed by a signal reports `128 + signal` instead of exit code -1.
- The browser no longer lists its own preview rows before anything is searched.
- A modpack installed from the browser now appears in the instance list at once.
- The crash hint now names the real cause of a crash. It used to report the first exception
  marker found from the start of the log tail, which could be a benign boot-time error; it now
  searches each marker from the end, in priority order.
- A dialog closed with Escape no longer eats the next click. Every dialog now exists only while
  open, instead of staying mounted and invisible.
- Number-key shortcuts stopped working after a screen change, because the focused field was
  destroyed with its screen. Keyboard focus now returns to the navigation rail after every
  navigation.
- The Stop button is now hidden, not just disabled, while no game is running. It used to render
  red and clickable-looking while inactive.
- Every screen starts with the data the launcher has, not sample rows: a never-launched
  instance's Logs tab, the browser's search box and results, and the accounts list all start
  empty until a real read fills them in.
- The instance list re-reads from disk every time it is opened, so an instance created or
  changed elsewhere (the CLI, a modpack install) shows up without a manual Refresh.

### Known issues

- CurseForge and Microsoft account features are implemented against the public API docs but not
  yet verified against the live services.
- No clipboard support in the desktop app.
- No file picker: a pack archive already on disk is named by typing its path.
- Forge before 1.13 is unsupported.
