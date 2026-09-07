#!/usr/bin/env bash
# Shared by scripts/e2e.sh and scripts/e2e-modpack.sh. Source it; do not run it.

# Checks that every jar on the classpath of a dry-run launch output exists on disk.
# Usage: check_classpath <path to the launch output>. Exits non-zero on the first problem.
check_classpath() {
  local out="$1"
  # The dry run prints one argument per line, indented, so the classpath is the line after `-cp`.
  local classpath
  classpath="$(path_list_after "$out" '-cp')"
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
  # `|| [ -n "$jar" ]` so the last entry, which has no trailing newline, is checked too.
  while IFS= read -r jar || [ -n "$jar" ]; do
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
  # The module path is a second list BootstrapLauncher reads, with the same rule. A
  # version JSON writes it out by hand, so a repeat there never went through the
  # library merge. Not every version has one.
  local modulepath
  modulepath="$(path_list_after "$out" '-p')"
  if [ -z "$modulepath" ]; then
    modulepath="$(path_list_after "$out" '--module-path')"
  fi
  if [ -n "$modulepath" ]; then
    local mdups
    mdups="$(printf '%s' "$modulepath" | tr "$sep" '\n' | sed '/^$/d' | sort | uniq -d)"
    if [ -n "$mdups" ]; then
      echo "FAIL: the module path names these jars twice:"
      echo "$mdups"
      return 1
    fi
  fi
  printf 'PASS check classpath files exist, none twice (%s entries)\n' "$count"
}

# Prints the argument that follows `$2` in a dry-run launch output, or nothing.
# Usage: path_list_after <path to the launch output> <flag>
path_list_after() {
  awk -v flag="$2" '
    { line = $0; gsub(/^[[:space:]]+|[[:space:]]+$/, "", line) }
    line == flag { getline; gsub(/^[[:space:]]+/, "", $0); print; exit }
  ' "$1"
}
