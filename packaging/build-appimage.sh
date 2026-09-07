#!/usr/bin/env bash
# Build a single-file x86_64 AppImage that holds the UI and the CLI binary.
# Output: dist/grid-craft-launcher-<version>-x86_64.AppImage
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APPDIR="$ROOT/AppDir"
DIST="$ROOT/dist"
TOOLS="$ROOT/packaging/tools"
APPIMAGETOOL="$TOOLS/appimagetool-x86_64.AppImage"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
UPDATE_INFO="gh-releases-zsync|sixdd6|grid-craft-launcher|latest|grid-craft-launcher-*-x86_64.AppImage.zsync"

# The single source of truth for the version is [workspace.package] in Cargo.toml.
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
if [ -z "$VERSION" ]; then
    echo "ERROR: could not read the version from Cargo.toml" >&2
    exit 1
fi
echo "Version: $VERSION"

echo "Building release binaries..."
cargo build --release -p gcl-ui -p gcl-cli

echo "Assembling AppDir..."
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/icons/hicolor/256x256/apps"

cp target/release/grid-craft-launcher "$APPDIR/usr/bin/grid-craft-launcher"
cp target/release/gcl "$APPDIR/usr/bin/gcl"

cp packaging/AppRun "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun"

# appimagetool reads the desktop file at the AppDir root.
cp packaging/grid-craft-launcher.desktop "$APPDIR/grid-craft-launcher.desktop"
printf 'X-AppImage-Version=%s\n' "$VERSION" >>"$APPDIR/grid-craft-launcher.desktop"

# The icon must sit at the AppDir root and in the hicolor theme directory.
cp packaging/icon.png "$APPDIR/grid-craft-launcher.png"
cp packaging/icon.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/grid-craft-launcher.png"

# The "continuous" release of appimagetool publishes no checksum or signature, so this
# download is trusted on first use. The file lands in packaging/tools/ and is reused on
# every later build, so the trust decision is made once per machine, not once per build.
if [ ! -x "$APPIMAGETOOL" ]; then
    echo "Downloading appimagetool..."
    mkdir -p "$TOOLS"
    curl -fsSL -o "$APPIMAGETOOL" "$APPIMAGETOOL_URL"
    chmod +x "$APPIMAGETOOL"
fi

# The update information lets AppImage updaters (Gear Lever, AppImageUpdate)
# find new GitHub releases. The companion .zsync file needs zsyncmake from the
# "zsync" package; without it the information is still embedded, but only a CI
# build produces the .zsync that delta updates need.
if ! command -v zsyncmake >/dev/null 2>&1; then
    echo "NOTE: zsyncmake not found — no .zsync file (fine for a local build)"
fi

OUTPUT_NAME="grid-craft-launcher-${VERSION}-x86_64.AppImage"
mkdir -p "$DIST"
echo "Building AppImage..."
"$APPIMAGETOOL" --appimage-extract-and-run -u "$UPDATE_INFO" "$APPDIR" "$DIST/$OUTPUT_NAME"

# appimagetool may write the .zsync into the current directory. Keep both files
# together in dist/.
if [ -f "$OUTPUT_NAME.zsync" ]; then
    mv "$OUTPUT_NAME.zsync" "$DIST/"
fi

echo ""
echo "AppImage: $DIST/$OUTPUT_NAME"
