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

run-ui:
    cargo run -p gcl-ui

# Live read-only check of a source's endpoints against our parsers
verify-api source:
    cargo run -p gcl-cli -- debug verify-source {{source}}

# End-to-end CLI run in a throwaway root. Does not start the game.
e2e:
    scripts/e2e.sh

# Save a live JSON response as a test fixture: `just record-fixture modrinth search-sodium 'https://...'`
record-fixture source name url:
    scripts/record-fixture.sh {{source}} {{name}} '{{url}}'

# Check agent and skill files have frontmatter
lint-claude:
    scripts/lint-claude-files.sh
