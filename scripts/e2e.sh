#!/usr/bin/env bash
# End-to-end CLI run in a temporary root. Fails at the first step the CLI does not support yet.
#
# GCL_E2E_LOADER picks the loader (fabric, quilt, forge, neoforge); GCL_E2E_MC_VERSION the
# Minecraft version. Nothing is installed outside the temporary root, which is removed on exit.
set -euo pipefail

ROOT="$(mktemp -d -t gcl-e2e-XXXXXX)"
trap 'rm -rf "$ROOT"' EXIT
export GCL_ROOT="$ROOT"

GCL="cargo run -q -p gcl-cli --"
LOADER="${GCL_E2E_LOADER:-fabric}"
# NeoForge's `neoforge` artifact starts at Minecraft 1.20.2, so its default is a version it
# actually published for. GCL_E2E_MC_VERSION overrides this.
case "$LOADER" in
  neoforge) DEFAULT_MC=1.20.2 ;;
  *) DEFAULT_MC=1.20.1 ;;
esac
MC_VERSION="${GCL_E2E_MC_VERSION:-$DEFAULT_MC}"

step() { printf '\n== %s\n' "$1"; }
pass() { printf 'PASS %s\n' "$1"; }

printf 'loader %s, minecraft %s, root %s\n' "$LOADER" "$MC_VERSION" "$ROOT"

step "create instance"
$GCL instance create e2e --minecraft "$MC_VERSION" --loader "$LOADER"
pass "create instance"

step "install loader"
$GCL loader install e2e
pass "install loader"

step "add a mod from modrinth"
if $GCL content --help >/dev/null 2>&1; then
  $GCL content add e2e --source modrinth --project sodium
  pass "content add"
else
  echo "SKIP content add (plan 3)"
fi

step "dry-run launch"
$GCL launch e2e --offline-user e2e-tester --dry-run > "$ROOT/launch.txt"
pass "dry-run launch"

step "check classpath files exist"
# The dry run prints one argument per line, indented, so the classpath is the line after `-cp`.
classpath="$(awk '/^[[:space:]]*-cp$/ { getline; gsub(/^[[:space:]]+/, "", $0); print; exit }' "$ROOT/launch.txt")"
if [ -z "$classpath" ]; then
  echo "FAIL: no -cp argument in the launch output"
  exit 1
fi
sep=':'
case "$classpath" in
  *\;*) sep=';' ;;
esac
missing=0
count=0
while IFS= read -r jar; do
  [ -n "$jar" ] || continue
  count=$((count + 1))
  [ -f "$jar" ] || { echo "MISSING $jar"; missing=1; }
done < <(printf '%s' "$classpath" | tr "$sep" '\n')
if [ "$count" -eq 0 ]; then
  echo "FAIL: no classpath entries in launch output"
  exit 1
fi
if [ "$missing" -ne 0 ]; then
  echo "FAIL: $count classpath entries, some missing"
  exit 1
fi
pass "check classpath files exist ($count entries)"
