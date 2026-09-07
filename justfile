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
# Needs `Xvfb`, ImageMagick's `import`, and python-xlib. The GUI runs over a throwaway root
# on display :97, `scripts/ui-xtest.py` sends the clicks and keys through XTest, and the
# check reads the GUI log for the jobs the run must have raised. Screenshots and the log stay
# in the temp root, whose path is printed at the end.
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
    trap 'for p in $app $xvfb; do kill "$p" 2>/dev/null || true; done; echo "shots and log: $root"' EXIT
    Xvfb :97 -screen 0 1200x760x24 >"$root/xvfb.log" 2>&1 &
    xvfb=$!
    sleep 2
    env -u WAYLAND_DISPLAY DISPLAY=:97 GCL_ROOT="$root" GCL_LOG=info \
        target/debug/grid-craft-launcher >"$root/gui.log" 2>&1 &
    app=$!
    sleep 6
    # Create a Fabric instance, open it, open Rename, close it with Escape, then navigate
    # with one click: the click after an Escape is the one a mounted-but-closed dialog ate.
    GCL_XTEST_DISPLAY=:97 python3 scripts/ui-xtest.py focus \
        click:1070,29 sleep:3 click:600,310 type:smoke \
        click:600,468 sleep:1 click:600,512 sleep:3 \
        click:813,541 sleep:8 shot:"$root/created.png" \
        click:250,75 sleep:2 click:933,37 sleep:2 key:Escape sleep:1 \
        click:68,181 sleep:2 shot:"$root/accounts.png"
    fail=0
    for want in "Create instance" "Install loader"; do
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
    if [ "$fail" = 0 ]; then echo PASS; else echo FAIL; exit 1; fi
