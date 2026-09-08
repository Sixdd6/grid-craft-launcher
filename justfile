set dotenv-load := true
set shell := ["bash", "-euo", "pipefail", "-c"]

# fmt, clippy, tests. Agents run this before reporting done.
check:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace

# Run tests matching a nextest filter expression, e.g. `just test 'test(mojang)'`
test filter:
    cargo nextest run --workspace -E '{{filter}}'

fmt:
    cargo fmt

deny:
    cargo deny check

# Preview a .slint file live, e.g. `just ui-preview screens/instances.slint`
ui-preview file:
    slint-viewer crates/gcl-ui/ui/{{file}}

run-cli *args:
    cargo run -p gcl-cli -- {{args}}

run-ui *args:
    cargo run -p gcl-ui -- {{args}}

# Live read-only check of a source's endpoints against our parsers
verify-api source:
    cargo run -p gcl-cli -- debug verify-source {{source}}

# End-to-end CLI run in a throwaway root. Does not start the game.
e2e:
    scripts/e2e.sh

# End-to-end modpack import in a throwaway root. Does not start the game.
e2e-modpack:
    scripts/e2e-modpack.sh

# Save a live JSON response as a test fixture: `just record-fixture modrinth search-sodium 'https://...'`
record-fixture source name url:
    scripts/record-fixture.sh {{source}} {{name}} '{{url}}'

# Check agent and skill files have frontmatter
lint-claude:
    scripts/lint-claude-files.sh

# Build the Linux AppImage into dist/
appimage:
    packaging/build-appimage.sh

# Run the built AppImage's UI smoke test
appimage-smoke:
    f=$(ls -t dist/grid-craft-launcher-*-x86_64.AppImage | head -1); "$f" --appimage-extract-and-run --smoke

# Show what cargo-dist would build for a release
dist-plan:
    dist plan

# Set the workspace version and roll the changelog, e.g. `just bump-version 0.2.0`
bump-version version:
    scripts/bump-version.sh {{version}}

# Drive the real window with real X input on Xvfb and report PASS or FAIL.
#
# It drives two legs: create an instance through the dialog, then search Modrinth in the
# browser and open a hit's project details.
#
# Needs the network, `Xvfb`, `xdpyinfo` (x11-utils), ImageMagick's `import`, and python3-xlib.
# Xvfb picks its own free display through `-displayfd`, so no fixed number and no stale lock
# can break the run. The GUI runs over a throwaway root, `scripts/ui-xtest.py` sends the
# clicks and keys through XTest, and the check reads the GUI log for the jobs the run must
# have raised. Every wait is a poll with a bounded timeout, not a fixed sleep. The temp root
# is removed on PASS and kept on failure, with its path printed.
#
# This is the only test that goes through hit-testing, pointer grabs, and X input focus. The
# flow tests in `crates/gcl-ui/tests` drive the same window through the Slint testing backend,
# which has neither an X server nor a window manager.
ui-xtest:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p gcl-ui
    root=$(mktemp -d gcl-xtest-XXXXXX --tmpdir)
    app=""
    xvfb=""
    ok=0
    cleanup() {
        for p in $app $xvfb; do kill "$p" 2>/dev/null || true; done
        # Wait as well as kill: an Xvfb that is still running holds its lock file, and the
        # next run would find a display that answers and then dies under it.
        for p in $app $xvfb; do wait "$p" 2>/dev/null || true; done
        if [ "$ok" = 1 ]; then rm -rf "$root"; else echo "shots and log kept in: $root"; fi
    }
    trap cleanup EXIT
    # Poll `cmd` every half second until it succeeds, or fail after `limit` seconds.
    wait_for() {
        what="$1"; limit="$2"; shift 2
        deadline=$((SECONDS + limit))
        until eval "$*"; do
            if [ "$SECONDS" -ge "$deadline" ]; then
                echo "timeout: waited ${limit}s for $what"
                return 1
            fi
            sleep 0.5
        done
    }
    # `-displayfd 3` makes Xvfb choose a free display and write its number to fd 3.
    Xvfb -displayfd 3 -screen 0 1200x760x24 3>"$root/display" >"$root/xvfb.log" 2>&1 &
    xvfb=$!
    wait_for "Xvfb to name its display" 30 '[ -s "$root/display" ]'
    disp=":$(tr -d '[:space:]' <"$root/display")"
    echo "display: $disp"
    wait_for "the X server on $disp" 30 "xdpyinfo -display '$disp' >/dev/null 2>&1"
    env -u WAYLAND_DISPLAY DISPLAY="$disp" GCL_ROOT="$root" GCL_LOG=info \
        target/debug/grid-craft-launcher >"$root/gui.log" 2>&1 &
    app=$!
    wait_for "the app to start" 60 'grep -q "gui start" "$root/gui.log"'
    # Create a Fabric instance. `focus` waits for the window itself, so no sleep here.
    timeout 180 env GCL_XTEST_DISPLAY="$disp" python3 scripts/ui-xtest.py focus \
        click:1070,29 sleep:3 click:600,310 type:smoke \
        click:600,468 sleep:1 click:600,512 sleep:3 \
        click:813,541
    wait_for "the instance to be created and the loader installed" 180 \
        '[ -d "$root/instances/smoke" ] && grep -q "Install loader" "$root/gui.log"'
    # Open it, open Rename, close it with Escape, then navigate with one click: the click
    # after an Escape is the one a mounted-but-closed dialog ate.
    timeout 180 env GCL_XTEST_DISPLAY="$disp" python3 scripts/ui-xtest.py focus \
        shot:"$root/created.png" \
        click:250,75 sleep:2 click:933,37 sleep:2 key:Escape sleep:1 \
        click:68,181 sleep:2 shot:"$root/accounts.png"
    # The GUI log writes a label in quotes and colours the line, so the job name alone is
    # ambiguous: "Search" is also a prefix of "Search icons". The quotes are the whole
    # pattern, and a variable carries them past `wait_for`'s eval.
    want_search='"Search"'
    want_details='"Project details"'
    # Open the browser from the rail, search Modrinth, and open the first hit's details.
    # A search row's title is its own click target, so real hit-testing is the only check
    # that it can be hit at all. Needs the network: the search and the description are
    # live Modrinth calls.
    timeout 180 env GCL_XTEST_DISPLAY="$disp" python3 scripts/ui-xtest.py focus \
        click:68,145 sleep:2 click:600,29 type:sodium key:Return
    wait_for "the search to answer" 120 'grep -q "$want_search" "$root/gui.log"'
    timeout 180 env GCL_XTEST_DISPLAY="$disp" python3 scripts/ui-xtest.py focus \
        shot:"$root/results.png" click:305,114
    wait_for "the project details to load" 120 \
        'grep -q "$want_details" "$root/gui.log"'
    timeout 180 env GCL_XTEST_DISPLAY="$disp" python3 scripts/ui-xtest.py focus \
        shot:"$root/details.png" click:360,155 sleep:2 shot:"$root/versions.png"
    fail=0
    for want in "Create instance" "Install loader" "$want_search" "$want_details"; do
        if grep -q "$want" "$root/gui.log"; then
            echo "ok: job $want"
        else
            echo "missing: job $want"
            fail=1
        fi
    done
    if [ -d "$root/instances/smoke" ]; then
        echo "ok: instances/smoke on disk"
    else
        echo "missing: instances/smoke"
        fail=1
    fi
    # The accounts screen reloads the store when it is shown, so a second read is the proof
    # that the one click after the Escape navigated. The first read is the one at start-up.
    if [ "$(grep -c 'Load accounts' "$root/gui.log")" -ge 2 ]; then
        echo "ok: the click after Escape reached the Accounts rail entry"
    else
        echo "missing: the click after Escape was lost"
        fail=1
    fi
    if [ "$fail" = 0 ]; then ok=1; echo PASS; else echo FAIL; exit 1; fi
