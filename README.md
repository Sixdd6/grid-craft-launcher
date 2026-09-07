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
just run-ui              # opens the desktop UI
just run-ui -- --smoke   # opens it, runs one synthetic task, quits — a quick sanity check
```

Screenshots: none yet. Add them here once the UI has a stable enough look to be worth capturing.

Keyboard shortcuts:

- `1`-`5`: jump to Instances, Instance, Browser, Accounts, Settings.
- Arrow keys: move the selection in any list.
- `Enter`: open or activate the selected row.
- `Escape`: close the open dialog.

The launcher's own controls (buttons, rows, panels) follow a dark theme. Standard form controls
— combo boxes, text fields, spin boxes — come from Slint's `fluent` widget style and keep its
light palette, so they look lighter than the rest of the window. This is a known limitation, not
a bug; see the `slint-ui` skill for detail.

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
just run-cli launch fab --dry-run
just run-cli modpack install --source modrinth --project fabulously-optimized --name fo
just e2e
just e2e-modpack
```

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
