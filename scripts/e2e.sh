#!/usr/bin/env bash
# End-to-end CLI run in a temporary root. Fails at the first step the CLI does not support yet.
#
# GCL_E2E_LOADER picks the loader (fabric, quilt, forge, neoforge); GCL_E2E_MC_VERSION the
# Minecraft version; GCL_E2E_MOD the Modrinth project to add, defaulted per loader since not
# every mod publishes for every loader. Nothing is installed outside the temporary root, which
# is removed on exit.
set -euo pipefail

# shellcheck source=scripts/lib/classpath-check.sh
source "$(dirname "$0")/lib/classpath-check.sh"

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

# Sodium is Fabric/Quilt-only, so Forge and NeoForge need a mod published for them instead.
# JEI has no NeoForge 1.20.2 build, so NeoForge uses Jade there. GCL_E2E_MOD overrides this.
case "$LOADER" in
  fabric|quilt) DEFAULT_MOD=sodium ;;
  forge) DEFAULT_MOD=jei ;;
  neoforge) DEFAULT_MOD=jade ;;
  *) echo "ERROR: unknown GCL_E2E_LOADER '$LOADER'" >&2; exit 1 ;;
esac
MOD="${GCL_E2E_MOD:-$DEFAULT_MOD}"

step() { printf '\n== %s\n' "$1"; }
pass() { printf 'PASS %s\n' "$1"; }

printf 'loader %s, minecraft %s, mod %s, root %s\n' "$LOADER" "$MC_VERSION" "$MOD" "$ROOT"

step "create instance"
$GCL instance create e2e --minecraft "$MC_VERSION" --loader "$LOADER"
pass "create instance"

step "install loader"
$GCL loader install e2e
pass "install loader"

step "add a mod from modrinth"
ADD_OUTPUT="$($GCL content add e2e --source modrinth --project "$MOD" | tee /dev/stderr)"
# The content list stores the Modrinth project id, not the slug we passed in, so match either:
# the slug (case-insensitively, since file names capitalize mod names differently) or the id
# `content add` printed in parentheses.
PROJECT_ID="$(printf '%s' "$ADD_OUTPUT" | grep -oE '\([A-Za-z0-9]+\)$' | tr -d '()')" || true
$GCL content list e2e --json > "$ROOT/content.json"
if ! grep -qi "$MOD" "$ROOT/content.json" && { [ -z "$PROJECT_ID" ] || ! grep -q "$PROJECT_ID" "$ROOT/content.json"; }; then
  echo "FAIL: $MOD is missing from the content list"
  cat "$ROOT/content.json"
  exit 1
fi
pass "content add"

# A dry run still resolves the JVM, and `ensure_java_component` accepts only the exact major
# Minecraft asks for. With nothing configured it downloads Mojang's `java-runtime-gamma`, some
# 96 MB, on every run. A configured `jvm.java_path` is used as given, so pointing the config at
# the machine's own `java` skips that download. The CLI has no setter for this key, so the
# config file is edited here; the loader and content steps above are untouched.
step "point the config at the local java"
JAVA_BIN="$(command -v java || true)"
CONFIG="$ROOT/config.toml"
if [ -z "$JAVA_BIN" ]; then
  echo "NOTE: no java on PATH, so the dry run will download Mojang's runtime"
elif grep -q '^\[jvm\]' "$CONFIG" 2>/dev/null; then
  sed -i "s|^\[jvm\]|[jvm]\njava_path = \"$JAVA_BIN\"|" "$CONFIG"
  pass "java_path = $JAVA_BIN"
else
  printf '\n[jvm]\njava_path = "%s"\n' "$JAVA_BIN" >> "$CONFIG"
  pass "java_path = $JAVA_BIN"
fi

step "dry-run launch"
$GCL launch e2e --offline-user e2e-tester --dry-run > "$ROOT/launch.txt"
pass "dry-run launch"

step "check classpath files exist"
check_classpath "$ROOT/launch.txt"
