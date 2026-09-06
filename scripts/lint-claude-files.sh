#!/usr/bin/env bash
# Every agent and skill file must start with YAML frontmatter holding name and description.
set -uo pipefail
status=0
for f in .claude/agents/*.md .claude/skills/*/SKILL.md; do
  [ -e "$f" ] || continue
  if [ "$(head -1 "$f")" != "---" ]; then echo "FAIL $f: no frontmatter"; status=1; continue; fi
  fm="$(awk 'NR>1 && /^---$/ {exit} NR>1 {print}' "$f")"
  echo "$fm" | grep -q '^name:' || { echo "FAIL $f: missing name"; status=1; }
  echo "$fm" | grep -q '^description:' || { echo "FAIL $f: missing description"; status=1; }
done
[ "$status" -eq 0 ] && echo "PASS: claude files have frontmatter"
exit "$status"
