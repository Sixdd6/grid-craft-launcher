#!/usr/bin/env bash
# End-to-end CLI run in a temporary root. Fails at the first step the CLI does not support yet.
set -euo pipefail

ROOT="$(mktemp -d -t gcl-e2e-XXXXXX)"
trap 'rm -rf "$ROOT"' EXIT
export GCL_ROOT="$ROOT"

GCL="cargo run -q -p gcl-cli --"
MC_VERSION="${GCL_E2E_MC_VERSION:-1.20.1}"

step() { printf '\n== %s\n' "$1"; }

step "create instance"
$GCL instance create e2e --minecraft "$MC_VERSION" --loader fabric

step "install loader"
$GCL loader install e2e

step "add a mod from modrinth"
$GCL content add e2e --source modrinth --project sodium

step "dry-run launch"
$GCL launch e2e --offline-user e2e-tester --dry-run > "$ROOT/launch.txt"

step "check classpath files exist"
missing=0
while read -r jar; do
  [ -f "$jar" ] || { echo "MISSING $jar"; missing=1; }
done < <(grep -oE '(^|[:; ])[^:; ]+\.jar' "$ROOT/launch.txt" | tr -d ':; ' | sort -u)
if [ "$missing" -eq 0 ]; then echo "PASS: every classpath entry exists"; else exit 1; fi
