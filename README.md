# GRID Craft Launcher

A Minecraft launcher for Linux, Windows, and macOS. Rust core, Slint UI, command line included.

Features planned for the MVP are in [docs/SPEC.md](docs/SPEC.md).

## Build

Requires Rust stable (see `rust-toolchain.toml`) and these tools:

```bash
cargo install just cargo-nextest cargo-deny slint-viewer --locked
```

Also requires `python3` (used by the Claude Code hook scripts).

Then:

```bash
just check      # fmt, clippy, tests
just run-cli debug verify-source modrinth
just run-ui
```

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

## Layout

- `crates/gcl-core`: library. All logic.
- `crates/gcl-cli`: `gcl` binary. Every feature, no display needed.
- `crates/gcl-ui`: `grid-craft-launcher` binary. Slint UI.
- `docs/`: spec, architecture research, design docs, plans.
- `.claude/`: agents, skills, and hooks for Claude Code.

## License

GPL-3.0-or-later. See `LICENSE`. Code ported from other GPL-3.0 launchers (Modrinth App, Prism Launcher) keeps its copyright notice. Code taken from MIT projects is listed in `THIRD_PARTY.md`.
