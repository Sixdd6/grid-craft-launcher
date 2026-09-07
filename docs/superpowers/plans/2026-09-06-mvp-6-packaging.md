# MVP Plan 6: Packaging and Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a runnable Linux AppImage locally, a cargo-dist release configuration and GitHub release workflow for Linux, Windows, and macOS archives, a version bump script, a changelog, and a documented release checklist. Close the MVP with a status table in the spec.

**Architecture:** `packaging/build-appimage.sh` assembles an AppDir (both binaries, AppRun, desktop file, icon) from a release build and runs a downloaded `appimagetool` with `--appimage-extract-and-run` (no FUSE needed), embedding GitHub zsync update info. cargo-dist owns cross-platform archives through `dist-workspace.toml` and a generated `release.yml`; it is configured and planned locally but never pushed (no remote exists). `scripts/bump-version.sh` edits the workspace version and the changelog header.

**Tech Stack:** cargo-dist 0.32, appimagetool (continuous), bash, GitHub Actions YAML.

**Spec:** `docs/SPEC.md` (all MVP requirements, final status), `.claude/skills/release-packaging/SKILL.md`, `packaging/` (desktop file and icons from plan 5), grid-launcher's `build.sh` as the AppImage reference.

## Global Constraints

- Prior global constraints (no secrets, `just check` per task, commit per task with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`).
- Nothing is pushed or tagged; no remote exists. The release workflow is generated and committed but only validated by `dist plan` and YAML parsing.
- The AppImage bundles `grid-craft-launcher` and `gcl`; AppRun executes `grid-craft-launcher` and forwards arguments; `gcl` is reachable via `APPIMAGE --cli` handled in AppRun (`if [ "$1" = "--cli" ]; then shift; exec "$HERE/usr/bin/gcl" "$@"; fi`).
- The AppImage must start on this machine: `dist/grid-craft-launcher-<ver>-x86_64.AppImage --appimage-extract-and-run --smoke` exits 0.
- Version lives only in `[workspace.package].version`; `gcl --version` and the desktop file (`X-AppImage-Version`) show it.
- Prose rules for docs unchanged.

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `packaging/build-appimage.sh`, `packaging/AppRun`, `justfile` | local AppImage build | 1 |
| `dist-workspace.toml`, `.github/workflows/release.yml`, `justfile` | cargo-dist config and workflow | 2 |
| `scripts/bump-version.sh`, `CHANGELOG.md`, `README.md`, `THIRD_PARTY.md`, skill | versioning, docs, checklist | 3 |
| `docs/SPEC.md`, `ARCHITECTURE.md`, README | MVP status table and final verification | 4 |

---

### Task 1: AppImage build

- [ ] `packaging/AppRun` (bash: `HERE`, `--cli` switch, `exec "$HERE/usr/bin/grid-craft-launcher" "$@"`), executable.
- [ ] `packaging/build-appimage.sh`: `set -euo pipefail`; `cargo build --release -p gcl-ui -p gcl-cli`; read the version from `Cargo.toml` (`grep -m1 '^version' Cargo.toml`); assemble `AppDir/usr/bin/{grid-craft-launcher,gcl}`, `AppDir/AppRun`, `AppDir/grid-craft-launcher.desktop` (copy `packaging/grid-craft-launcher.desktop`, add `X-AppImage-Version=<ver>`), `AppDir/grid-craft-launcher.png` (256 px) and `AppDir/usr/share/icons/hicolor/256x256/apps/grid-craft-launcher.png`; download `appimagetool-x86_64.AppImage` into `packaging/tools/` when missing (curl, GitHub continuous release), `chmod +x`; run `./packaging/tools/appimagetool --appimage-extract-and-run -u "gh-releases-zsync|sixdd6|grid-craft-launcher|latest|grid-craft-launcher-*-x86_64.AppImage.zsync" AppDir "dist/grid-craft-launcher-${VERSION}-x86_64.AppImage"`; move any `.zsync` into `dist/`; print the path. `.gitignore` gains `/AppDir/`, `/dist/`, `/packaging/tools/`.
- [ ] `justfile`: `appimage: packaging/build-appimage.sh` and `appimage-smoke: dist/grid-craft-launcher-*-x86_64.AppImage --appimage-extract-and-run --smoke`.
- [ ] Verify: `just appimage` builds (report size), `just appimage-smoke` exits 0, `./dist/*.AppImage --appimage-extract-and-run --cli --version` prints the version. Commit `build: appimage packaging`.

---

### Task 2: cargo-dist

- [ ] `cargo install cargo-dist --locked` (0.32). Run `dist init --yes` (or the equivalent non-interactive flags: `--ci github`, `--hosting github`, installers none) with targets `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`; if the CLI needs an interactive answer, write `dist-workspace.toml` by hand per the cargo-dist 0.32 docs and run `dist generate` to produce `.github/workflows/release.yml`. Only `gcl-ui` and `gcl-cli` are dist members (`gcl-core` excluded via `dist = false` in its `Cargo.toml` `[package.metadata.dist]`).
- [ ] Linux builds need Slint's system libraries; add the apt packages from `ci.yml` to the release workflow's Linux job via the `[dist.github-custom-runners]`/`dependencies` config that cargo-dist supports (or a `dist.dependencies.apt` table); document if unsupported.
- [ ] Verify: `dist plan` succeeds and lists the four archives; `dist build --artifacts local --target x86_64-unknown-linux-gnu` produces `target/distrib/*.tar.xz` containing both binaries; `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/release.yml"))'` parses (if PyYAML missing, skip and say so). `just dist-plan` recipe. Commit `build: cargo-dist configuration and release workflow`.

---

### Task 3: Versioning, changelog, docs

- [ ] `scripts/bump-version.sh <x.y.z>`: validates semver; edits `[workspace.package] version`; runs `cargo update -w --offline` (or `cargo metadata` refresh) so `Cargo.lock` matches; moves the `## Unreleased` section of `CHANGELOG.md` under `## <x.y.z> - <date>` and adds a fresh `## Unreleased`; prints the next commands (`git commit`, `git tag v<x.y.z>`); no git actions itself. Test by running it in a temp copy of the repo (`git worktree add` or `cp -r` to a temp dir) and diffing.
- [ ] `CHANGELOG.md`: `## Unreleased` with a summary of plans 1-5 (instances, loaders, sources, modpacks, settings, accounts incl. Microsoft, launch, CLI, GUI, AppImage).
- [ ] `README.md`: "Install" (AppImage download from Releases, `chmod +x`, run; Windows/macOS archives from cargo-dist; build from source), "Release" (checklist: `just check && just deny`, `just e2e`, `just e2e-modpack`, `just run-ui -- --smoke`, `just appimage && just appimage-smoke`, `scripts/bump-version.sh`, commit, tag, push tag → workflow). `THIRD_PARTY.md`: confirm no ported code (or list it). `.claude/skills/release-packaging/SKILL.md`: match the built scripts and recipes.
- [ ] Commit `docs: versioning, changelog, install and release docs`.

---

### Task 4: MVP verification and status

- [ ] Run and record: `just check`, `just deny`, `just lint-claude`, `just e2e`, `just e2e-modpack`, `GCL_E2E_LOADER=neoforge just e2e`, `just run-ui -- --smoke`, `just appimage-smoke`, `just verify-api mojang`, `just verify-api fabric`, `just verify-api modrinth` (curseforge and msa print SKIP).
- [ ] `docs/SPEC.md`: add an "MVP status (2026-09-06)" table: every R-id → Done / Done (CLI only) / Partial (what is missing) / Not verified live (CurseForge, Microsoft) with one line each; list known limitations (std-widgets palette, no clipboard, modpack discovery by id, no stop button, keyboard routing compile-verified).
- [ ] `ARCHITECTURE.md`: packaging section (AppImage layout, cargo-dist members, release flow).
- [ ] Commit `docs: mvp status and packaging architecture`.
