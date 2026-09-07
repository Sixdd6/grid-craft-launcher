#!/usr/bin/env bash
# Lists every named element in the Slint sources, grouped by the component that declares it.
#
# The Slint testing backend addresses an element as `<Component>::<name>`, where the name is
# the `name := Element { }` label in the `.slint` file. This prints that table, which is what
# `docs/research/2026-09-07-ui-element-ids.md` holds and what a GUI flow test looks names up in.
#
# Usage: scripts/list-slint-ids.sh [ui-dir]   (default crates/gcl-ui/ui)
set -euo pipefail

dir="${1:-crates/gcl-ui/ui}"

find "$dir" -name '*.slint' | sort | while read -r file; do
    component=""
    while IFS= read -r line; do
        case "$line" in
            *component\ *inherits*)
                component="$(printf '%s\n' "$line" \
                    | sed -E 's/^[[:space:]]*(export[[:space:]]+)?component[[:space:]]+([A-Za-z0-9_-]+).*/\2/')"
                ;;
        esac
        name="$(printf '%s\n' "$line" \
            | sed -nE 's/^[[:space:]]*(if[[:space:]].*:[[:space:]]*|for[[:space:]].*:[[:space:]]*)?([a-z][a-z0-9_]*)[[:space:]]*:=[[:space:]]*([A-Za-z0-9_-]+).*/\2 \3/p')"
        if [ -n "$name" ]; then
            kind="${name##* }"
            # A Timer is not an element: it never enters the element tree, so
            # `find_by_element_id` can never answer with one. Keep it out of the table.
            if [ "$kind" != "Timer" ]; then
                printf '%s\t%s::%s\n' "$file" "${component:-?}" "$name"
            fi
        fi
    done < "$file"
done
