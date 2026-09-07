---
name: release-packaging
description: How releases are built and versioned — cargo-dist config, AppImage on Linux, Windows zip, macOS bundle, version bump steps. Read before changing CI release jobs or Cargo version fields.
---

## Versioning

- Single `version` in `[workspace.package]`. Bump with a commit `release: v0.x.y` and a tag `v0.x.y`.
- `gcl --version` and the UI show `gcl_core::VERSION`.

## cargo-dist

- Install: `cargo install cargo-dist --locked`. Init once: `dist init` choosing GitHub CI, targets `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`. Commit `dist-workspace.toml` and the generated `.github/workflows/release.yml`.
- Only `gcl-ui` and `gcl-cli` are published binaries.

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

1. `just check` and `just deny` pass.
2. `just e2e` passes on Linux.
3. `THIRD_PARTY.md` lists every ported file.
4. Update `CHANGELOG.md` (create on first release).
