#!/usr/bin/env bash
# Bump the workspace version and roll the changelog. Run this before tagging
# a release. Does not touch git: it only edits files and prints next steps.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ $# -ne 1 ]; then
    echo "usage: $0 <x.y.z>" >&2
    exit 1
fi

VERSION="$1"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "ERROR: '$VERSION' is not a semver x.y.z version" >&2
    exit 1
fi

CARGO_TOML="Cargo.toml"
CHANGELOG="CHANGELOG.md"

if [ ! -f "$CARGO_TOML" ]; then
    echo "ERROR: $CARGO_TOML not found" >&2
    exit 1
fi
if [ ! -f "$CHANGELOG" ]; then
    echo "ERROR: $CHANGELOG not found" >&2
    exit 1
fi

echo "Setting version to $VERSION in [workspace.package]..."
awk -v ver="$VERSION" '
    /^\[workspace\.package\]/ { in_section = 1 }
    /^\[/ && !/^\[workspace\.package\]/ { in_section = 0 }
    in_section && /^version = / {
        print "version = \"" ver "\""
        next
    }
    { print }
' "$CARGO_TOML" >"$CARGO_TOML.tmp"
mv "$CARGO_TOML.tmp" "$CARGO_TOML"

echo "Updating Cargo.lock..."
if cargo update --workspace --offline 2>/dev/null; then
    :
else
    echo "NOTE: 'cargo update --workspace' failed or is unsupported; refreshing metadata instead"
    cargo metadata --offline >/dev/null
fi

DATE="$(date +%Y-%m-%d)"
echo "Rolling CHANGELOG.md: Unreleased -> $VERSION - $DATE"
if ! grep -q '^## Unreleased$' "$CHANGELOG"; then
    echo "ERROR: no '## Unreleased' heading found in $CHANGELOG" >&2
    exit 1
fi
awk -v ver="$VERSION" -v date="$DATE" '
    /^## Unreleased$/ && !done {
        print "## Unreleased"
        print ""
        print "## " ver " - " date
        done = 1
        next
    }
    { print }
' "$CHANGELOG" >"$CHANGELOG.tmp"
mv "$CHANGELOG.tmp" "$CHANGELOG"

echo ""
echo "Done. Version is now $VERSION. No git commands were run."
echo "Next steps:"
echo "  git add Cargo.toml Cargo.lock CHANGELOG.md"
echo "  git commit -m \"release: v$VERSION\""
echo "  git tag v$VERSION"
echo "  git push origin v$VERSION"
