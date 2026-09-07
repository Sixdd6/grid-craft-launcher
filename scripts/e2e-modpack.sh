#!/usr/bin/env bash
# End-to-end modpack import in a temporary root, then a dry-run launch of the new instance.
#
# GCL_E2E_PACK picks the Modrinth pack slug. Nothing is installed outside the temporary root,
# which is removed on exit. The game is never started.
set -euo pipefail

# shellcheck source=scripts/lib/classpath-check.sh
source "$(dirname "$0")/lib/classpath-check.sh"

ROOT="$(mktemp -d -t gcl-e2e-pack-XXXXXX)"
trap 'rm -rf "$ROOT"' EXIT
export GCL_ROOT="$ROOT"

GCL="cargo run -q -p gcl-cli --"
PACK="${GCL_E2E_PACK:-fabulously-optimized}"

step() { printf '\n== %s\n' "$1"; }
pass() { printf 'PASS %s\n' "$1"; }

printf 'pack %s, root %s\n' "$PACK" "$ROOT"

step "install modpack"
$GCL modpack install --source modrinth --project "$PACK" --name e2epack
pass "install modpack"

step "list content"
$GCL content list e2epack --json > "$ROOT/content.json"
# An import that placed no content is a silent failure: the instance would launch
# vanilla. Fail here instead of at the classpath check, which would still pass.
python3 - "$ROOT/content.json" <<'EOF'
import json, sys
entries = json.load(open(sys.argv[1]))
if not entries:
    sys.exit("content list is empty: the pack installed nothing")
print(f"{len(entries)} content entries")
EOF
pass "list content"

step "dry-run launch"
$GCL launch e2epack --offline-user t --dry-run > "$ROOT/launch.txt"
pass "dry-run launch"

step "check classpath files exist"
check_classpath "$ROOT/launch.txt"
