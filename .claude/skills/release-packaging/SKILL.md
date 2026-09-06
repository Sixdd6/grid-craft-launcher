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

- Build release, then `linuxdeploy --appdir AppDir --executable target/release/grid-craft-launcher --desktop-file packaging/grid-craft-launcher.desktop --icon-file packaging/icon.png --output appimage`.
- Keep `packaging/` for the desktop file and icons. Follow the `grid-launcher` project's `build.sh` for the linuxdeploy download step.
- Slint with winit needs no bundled Qt; ensure `libxkbcommon`, `fontconfig`, and `wayland` client libs are system-provided (they are on any desktop).

## Windows and macOS

- Windows: cargo-dist zip with both binaries. MSI later.
- macOS: cargo-dist tarball for the MVP. An `.app` bundle later.

## Checklist before tagging

1. `just check` and `just deny` pass.
2. `just e2e` passes on Linux.
3. `THIRD_PARTY.md` lists every ported file.
4. Update `CHANGELOG.md` (create on first release).
