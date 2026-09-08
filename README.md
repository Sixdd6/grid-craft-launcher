# GRID Craft Launcher

A Minecraft launcher for Linux, Windows, and macOS. Rust core, Slint UI, command line included.

Features planned for the MVP are in [docs/SPEC.md](docs/SPEC.md).

What each MVP requirement delivers: [MVP status table](docs/SPEC.md#mvp-status-2026-09-06).

## Install

**Linux (AppImage):**

1. Download `grid-craft-launcher-<version>-x86_64.AppImage` from the
   [Releases page](https://github.com/sixdd6/grid-craft-launcher/releases).
2. `chmod +x grid-craft-launcher-<version>-x86_64.AppImage`
3. Run it. It opens the desktop UI.

Run `./grid-craft-launcher-<version>-x86_64.AppImage --cli <args>` to use the command line
tool instead.

**Windows and macOS:** download the archive built by cargo-dist from the same Releases page.

**From source:** see Build below.

## Build

Requires Rust stable (see `rust-toolchain.toml`) and these tools:

```bash
cargo install just cargo-nextest cargo-deny slint-viewer cargo-dist --locked
```

Also requires `python3` (used by the Claude Code hook scripts).

Then:

```bash
just check      # fmt, clippy, tests
just run-cli debug verify-source modrinth
just run-ui
```

## Run the app

```bash
just run-ui                              # opens the desktop UI
just run-ui -- --smoke                   # opens it, runs one synthetic task, quits
just run-ui -- --screenshot shot.png     # opens it, saves a PNG of the window, quits
GCL_LOG=debug just run-ui                # the same, with logging on stderr
```

The app writes a log of every job it runs — the label, how long it took, and the error chain
when one fails — to `<root>/logs/gui.log.<date>`, rotated daily. The error dialog ends with that
path. `GCL_LOG` adds the same lines on stderr and takes a filter directive: `GCL_LOG=debug`, or
`GCL_LOG=gcl_core::download=trace` for one module.

`--screenshot` needs a compositor that actually draws the window. A Wayland session gives an
unmapped or occluded window no frame callback, so the run waits for a frame that never comes;
`xvfb-run -a grid-craft-launcher --screenshot shot.png` always works.

Screenshots: none yet. Add them here once the UI has a stable enough look to be worth capturing.

Keyboard shortcuts:

- `1`-`5`: jump to Instances, Instance, Browser, Accounts, Settings.
- Arrow keys: move the selection in any list.
- `Enter`: open or activate the selected row.
- `Escape`: close the open dialog.

The whole window is dark, standard form controls included — combo boxes, text fields and spin
boxes follow the shell through Slint's `Palette.color-scheme`.

Game settings are a typed editor rather than a key-value table: a slider, switch, choice box, or
text field per `options.txt` key, grouped and searchable. Each row says which layer its value
came from — an instance override, the file the game wrote, the launcher preseed, or the game's
own default — and a row this screen's layer holds offers Reset. A key the launcher does not know
keeps a raw row and is never dropped.

A search result's title opens a details page for that mod, pack, or shader: the project's
description, rendered with headings, paragraphs, bullets, code, tables, rules, quotes, and
images, and a Versions tab listing the versions your instance can run. The tab marks the version
you have installed and installs any other one over it. Each version also has a Notes button that
opens its release notes in the same rendered style. An installed item's source button opens the
same page, and Back returns to where you came from. Installed items show the project's title
with the file name under it, sorted by title.

The instance detail screen has a Stop button. It asks the game to exit, waits ten seconds, then
kills it; a stop you asked for is reported as "Stopped", not as a crash.

### Real-input smoke

`just ui-xtest` drives the desktop app with real X pointer and keyboard events under Xvfb,
instead of the accessible actions the flow tests use. It needs Xvfb, `xdpyinfo`, ImageMagick's
`import`, the `python3-xlib` package, and network access. It checks that a dialog closed with
Escape does not eat the next click, that a number-key shortcut still works after a screen
change, and that an instance created from the CLI shows up in the GUI's instance list without a
Refresh. It also searches Modrinth in the browser and opens a result's details page from its
title.

## Try it

```bash
just run-cli version list
just run-cli instance create demo --minecraft 1.20.1
just run-cli instance list
just run-cli java list
just run-cli account add-offline you
just run-cli instance create fab --minecraft 1.20.1 --loader fabric
just run-cli content search sodium
just run-cli content add fab --source modrinth --project sodium
just run-cli account add-msa
just run-cli instance gc fab
just run-cli instance jvm fab --min 2048 --max 6144 --gc g1
just run-cli config set-jvm --gc g1
just run-cli launch fab --dry-run
just run-cli modpack install --source modrinth --project fabulously-optimized --name fo
just e2e
just e2e-modpack
```

Garbage collector presets: `instance gc <slug>` runs the instance's Java once and prints the
collectors that build carries. `instance jvm <slug> --gc <preset>` saves one of them and
refuses one the Java lacks, naming what it does have. `config set-jvm --gc <preset>` sets the
preset a newly created instance starts with; it is not read again at launch.

CurseForge: without a `CURSEFORGE_API_KEY` in `.env`, `--source curseforge` is disabled and
everything above still works against Modrinth. Put a key in `.env` to enable it.

## Secrets

Copy `.env.example` to `.env` and fill in:

- `CURSEFORGE_API_KEY`: from the CurseForge for Studios console. Without it, CurseForge features are disabled and everything else works.
- `GCL_MSA_CLIENT_ID`: an Azure app registration approved for the Minecraft API. Without it, Microsoft login is disabled and offline mode works.

The launcher reads env first, then `config.toml` in its root directory.

## Microsoft login

Microsoft login needs your own Azure app registration; the repo ships no client id.

1. In the Azure Portal, go to Microsoft Entra ID → App registrations → New registration.
2. Supported account types: personal Microsoft accounts.
3. Platform: Mobile and desktop applications. Redirect URI:
   `https://login.microsoftonline.com/common/oauth2/nativeclient`. No client secret needed.
4. Turn on "Allow public client flows".
5. Submit the Minecraft launcher approval form, linked from
   https://help.minecraft.net/hc/en-us/articles/16254801392141. Until Microsoft approves it,
   sign-in fails at the last step with an "invalid app registration" error.
6. Put the app's client id in `.env` as `GCL_MSA_CLIENT_ID`.

Then sign in:

```bash
just run-cli account add-msa
```

Without `GCL_MSA_CLIENT_ID`, Microsoft login is disabled and offline mode works
(`just run-cli account add-offline <name>`).

## Layout

- `crates/gcl-core`: library. All logic.
- `crates/gcl-cli`: `gcl` binary. Every feature, no display needed.
- `crates/gcl-ui`: `grid-craft-launcher` binary. Slint UI.
- `docs/`: spec, architecture research, design docs, plans.
- `.claude/`: agents, skills, and hooks for Claude Code.

## Release

Checklist before tagging a release:

```bash
just check && just deny && just lint-claude
just e2e
just e2e-modpack
just run-ui -- --smoke
just appimage && just appimage-smoke
just bump-version x.y.z
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "release: vx.y.z"
git tag vx.y.z
git push origin vx.y.z
```

The pushed tag runs the release workflow, which builds the Windows and macOS archives through
cargo-dist. cargo-dist does not build the AppImage; attach the file from `just appimage` to the
GitHub release by hand.

## License

GPL-3.0-or-later. See `LICENSE`. Code ported from other GPL-3.0 launchers (Modrinth App, Prism Launcher) keeps its copyright notice. Code taken from MIT projects is listed in `THIRD_PARTY.md`.
