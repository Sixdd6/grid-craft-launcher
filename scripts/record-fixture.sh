#!/usr/bin/env bash
# Usage: scripts/record-fixture.sh <source> <name> <url>
# Saves tests/fixtures/<source>/<name>.json with our User-Agent. Adds x-api-key for curseforge.
set -euo pipefail
source="$1"; name="$2"; url="$3"
dir="tests/fixtures/$source"
mkdir -p "$dir"
ua="sixdd6/grid-craft-launcher/dev (sixdd6@gmail.com)"
args=(-fsSL -A "$ua" -H 'Accept: application/json')
if [ "$source" = "curseforge" ]; then
  : "${CURSEFORGE_API_KEY:?set CURSEFORGE_API_KEY in .env}"
  args+=(-H "x-api-key: $CURSEFORGE_API_KEY")
fi
tmp="$(mktemp)"
pretty="$(mktemp)"
trap 'rm -f "$tmp" "$pretty"' EXIT
curl "${args[@]}" "$url" -o "$tmp"
python3 -m json.tool < "$tmp" > "$pretty"
mv "$pretty" "$dir/$name.json"
rm -f "$tmp"
echo "wrote $dir/$name.json ($(wc -c < "$dir/$name.json") bytes)"
