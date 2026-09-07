---
name: release-packaging
description: How releases are built and versioned — cargo-dist config, AppImage on Linux, Windows zip, macOS bundle, version bump steps. Read before changing CI release jobs or Cargo version fields.
---

## Versioning

- Single `version` in `[workspace.package]`. `gcl --version` and the UI show `gcl_core::VERSION`.
- Bump with `just bump-version x.y.z` (`scripts/bump-version.sh`). It validates the version is
  `x.y.z`, edits `[workspace.package] version` in `Cargo.toml`, runs `cargo update --workspace
  --offline` so `Cargo.lock` matches, and renames `CHANGELOG.md`'s `## Unreleased` heading to
  `## x.y.z - <date>` with a fresh `## Unreleased` above it. It runs no git command; it prints the
  commit, tag, and push commands to run next.
- After the script: `git add -A`, `git commit -m "release: vx.y.z"`, `git tag vx.y.z`,
  `git push origin vx.y.z`.

## cargo-dist

- Install: `cargo install cargo-dist --locked` (the command is `dist`). Init once with `dist init --yes`,
  then edit `dist-workspace.toml`. Re-run `dist generate` after every config change and commit both
  `dist-workspace.toml` and `.github/workflows/release.yml`.
- Config lives in `dist-workspace.toml` under `[dist]`: `ci = "github"`, `installers = []`,
  `pr-run-mode = "plan"`, and the four targets `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`.
- The workspace sets `publish = false`, which hides binaries from dist. `gcl-ui` and `gcl-cli`
  therefore carry `[package.metadata.dist] dist = true`; `gcl-core` carries `dist = false`.
- Slint's Linux system libraries come from the `[dist.dependencies.apt]` table. Keep that package
  list equal to the apt list in `.github/workflows/ci.yml`. dist puts them in the release
  workflow's `matrix.packages_install`, on the Linux runner only.
- dist makes one archive per package, so a release carries `gcl-ui-<target>` and `gcl-cli-<target>`
  archives with one binary each. It cannot put binaries from two packages in one archive. The
  Linux AppImage is the artifact that ships both.
- `just dist-plan` (`dist plan`) shows what a release would build. `dist build --artifacts local
  --target <triple>` builds one target into `target/distrib/`.

## Linux AppImage

- `packaging/` holds everything the AppImage build consumes: `grid-craft-launcher.desktop` (Name,
  `Exec=grid-craft-launcher`, `Icon=grid-craft-launcher`, `Categories=Game;`), `icon.png` (256x256,
  the rasterized icon copied into the AppImage), and `icon.svg` (the hand-written
  source the PNG was rendered from — regenerate the PNG from it with
  `magick -background none packaging/icon.svg -resize 256x256 packaging/icon.png` rather than
  editing the PNG directly).
- Build with `just appimage` (`packaging/build-appimage.sh`). The script builds both release
  binaries, assembles `AppDir/`, downloads `appimagetool-x86_64.AppImage` into `packaging/tools/`
  when it is absent, and writes `dist/grid-craft-launcher-<version>-x86_64.AppImage`.
- `packaging/AppRun` starts the UI. `--cli` starts `gcl` with the remaining arguments, so one
  AppImage carries both binaries.
- The AppImage embeds update information
  (`gh-releases-zsync|sixdd6|grid-craft-launcher|latest|grid-craft-launcher-*-x86_64.AppImage.zsync`).
  The companion `.zsync` file needs `zsyncmake` (package `zsync`); the script prints a note and
  continues when it is missing.
- Smoke test the result with `just appimage-smoke`.
- `AppDir/`, `dist/`, and `packaging/tools/` are build output. They stay out of git.
- Slint with winit needs no bundled Qt; ensure `libxkbcommon`, `fontconfig`, and `wayland` client libs are system-provided (they are on any desktop).

## Windows and macOS

- Windows: cargo-dist zip with both binaries. MSI later.
- macOS: cargo-dist tarball for the MVP. An `.app` bundle later.

## Checklist before tagging

1. `just check && just deny && just lint-claude` pass.
2. `just e2e` passes.
3. `just e2e-modpack` passes.
4. `just run-ui -- --smoke` opens the UI, runs one synthetic task, and quits cleanly.
5. `just appimage && just appimage-smoke` build and start the AppImage.
6. `just bump-version x.y.z` sets the version and rolls `CHANGELOG.md`.
7. Commit, `git tag vx.y.z`, push the tag. The release workflow builds the Windows and macOS
   archives. cargo-dist does not build the AppImage; attach it to the GitHub release by hand.
8. `THIRD_PARTY.md` lists every ported file, or states that there are none.
