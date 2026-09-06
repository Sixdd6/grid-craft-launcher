#!/usr/bin/env bash
# PostToolUse hook for Edit and Write. Formats the edited Rust file. Reads hook JSON on stdin.
set -uo pipefail
path="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("tool_input",{}).get("file_path",""))' 2>/dev/null)"
case "$path" in
  *.rs) rustfmt --edition 2024 "$path" >/dev/null 2>&1 || true ;;
esac
exit 0
