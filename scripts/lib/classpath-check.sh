#!/usr/bin/env bash
# Shared by scripts/e2e.sh and scripts/e2e-modpack.sh. Source it; do not run it.

# Checks that every jar on the classpath of a dry-run launch output exists on disk.
# Usage: check_classpath <path to the launch output>. Exits non-zero on the first problem.
check_classpath() {
  local out="$1"
  # The dry run prints one argument per line, indented, so the classpath is the line after `-cp`.
  local classpath
  classpath="$(awk '/^[[:space:]]*-cp$/ { getline; gsub(/^[[:space:]]+/, "", $0); print; exit }' "$out")"
  if [ -z "$classpath" ]; then
    echo "FAIL: no -cp argument in the launch output"
    return 1
  fi
  local sep=':'
  case "$classpath" in
    *\;*) sep=';' ;;
  esac
  local missing=0
  local count=0
  local jar
  while IFS= read -r jar; do
    [ -n "$jar" ] || continue
    count=$((count + 1))
    [ -f "$jar" ] || { echo "MISSING $jar"; missing=1; }
  done < <(printf '%s' "$classpath" | tr "$sep" '\n')
  if [ "$count" -eq 0 ]; then
    echo "FAIL: no classpath entries in launch output"
    return 1
  fi
  if [ "$missing" -ne 0 ]; then
    echo "FAIL: $count classpath entries, some missing"
    return 1
  fi
  # BootstrapLauncher (Forge and NeoForge) throws `Duplicate key` when a jar is named twice.
  local dups
  dups="$(printf '%s' "$classpath" | tr "$sep" '\n' | sed '/^$/d' | sort | uniq -d)"
  if [ -n "$dups" ]; then
    echo "FAIL: the classpath names these jars twice:"
    echo "$dups"
    return 1
  fi
  printf 'PASS check classpath files exist, none twice (%s entries)\n' "$count"
}
