#!/usr/bin/env bash
# Stop hook. Blocks the session from ending while the workspace does not compile.
set -uo pipefail
if ! out="$(cargo check --workspace --quiet 2>&1)"; then
  python3 - "$out" <<'EOF'
import json, sys
print(json.dumps({"decision": "block", "reason": "cargo check failed. Fix the build before finishing:\n" + sys.argv[1][-3000:]}))
EOF
fi
exit 0
