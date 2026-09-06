#!/usr/bin/env bash
# Stop hook. Blocks the session from ending while the workspace does not compile.
set -uo pipefail

input="$(cat)"
stop_hook_active="$(python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except ValueError:
    data = {}
print("true" if data.get("stop_hook_active") else "false")
' <<<"$input")"

if [ "$stop_hook_active" = "true" ]; then
  exit 0
fi

if ! out="$(cargo check -p gcl-core -p gcl-cli --quiet 2>&1)"; then
  python3 - "$out" <<'EOF'
import json, sys
print(json.dumps({"decision": "block", "reason": "cargo check failed. Fix the build before finishing:\n" + sys.argv[1][-3000:]}))
EOF
fi
exit 0
