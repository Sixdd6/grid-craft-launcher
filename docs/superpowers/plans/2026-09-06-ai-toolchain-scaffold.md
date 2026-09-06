# AI Toolchain Scaffold Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce the repository skeleton, documents, Claude agents, skills, hooks, and CI that later plans use to build the launcher.

**Architecture:** A Cargo workspace with `gcl-core` (library), `gcl-cli` (clap binary), and `gcl-ui` (Slint binary), each compiling to a stub. Project knowledge lives in `.claude/skills/*/SKILL.md` and `docs/`, agents in `.claude/agents/*.md`, and automation in `justfile`, `.claude/settings.json`, and `.github/workflows/ci.yml`.

**Tech Stack:** Rust 1.97 stable, tokio 1.53, clap 4.6, Slint 1.17 (winit + FemtoVG), just, cargo-nextest, cargo-deny, slint-viewer.

**Spec:** `docs/superpowers/specs/2026-09-06-ai-toolchain-design.md`

## Global Constraints

- License is `GPL-3.0-or-later` in `LICENSE` and every crate `Cargo.toml`.
- Crate names: `gcl-core`, `gcl-cli` (binary `gcl`), `gcl-ui` (binary `grid-craft-launcher`).
- Agents: planner, coder, slint-designer, doc-research, api-verifier, e2e-runner, rust-diagnostics. Models: planner sonnet, coder opus, slint-designer sonnet, doc-research haiku, api-verifier sonnet, e2e-runner sonnet, rust-diagnostics opus.
- Skills: planning, rust-conventions, testing, slint-ui, mojang-meta, modloaders, mod-sources, modpack-formats, msa-auth, instance-model, download-cache, release-packaging.
- Secrets are `CURSEFORGE_API_KEY` and `GCL_MSA_CLIENT_ID`, read from env first, then `config.toml`. Never committed, never logged.
- No live API calls in unit tests, `just check`, or CI.
- `just check` = `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace`.
- Every `.md` under `.claude/agents` and `.claude/skills` has YAML frontmatter with `name` and `description`.
- Prose in skills and docs follows the user's writing rules: short sentences, active voice, one word per thing, no "seamless", "robust", "powerful", "elegant", "simply", "just" as an adverb.
- Commit after every task. Commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml` | workspace config and policy | 1 |
| `crates/gcl-core/{Cargo.toml,src/lib.rs}` | core library stub with `VERSION` and `USER_AGENT` | 1 |
| `crates/gcl-cli/{Cargo.toml,src/main.rs}` | `gcl` binary stub with `--version` and `debug` subcommand shape | 1 |
| `crates/gcl-ui/{Cargo.toml,build.rs,src/main.rs,ui/app.slint,ui/theme.slint}` | Slint window stub | 2 |
| `LICENSE`, `README.md`, `THIRD_PARTY.md`, `.env.example`, `.gitignore` | legal and onboarding | 3 |
| `justfile`, `scripts/e2e.sh`, `scripts/record-fixture.sh`, `scripts/lint-claude-files.sh` | task recipes | 4 |
| `.claude/settings.json`, `scripts/hooks/fmt-on-edit.sh`, `scripts/hooks/check-on-stop.sh` | permissions and hooks | 5 |
| `docs/SPEC.md` | MVP requirements | 6 |
| `ARCHITECTURE.md`, `AGENTS.md`, `CLAUDE.md` | orientation for agents | 7 |
| `.claude/agents/*.md` (7 files) | agent definitions | 8 |
| `.claude/skills/{planning,rust-conventions,testing,slint-ui}/SKILL.md` | process and stack skills | 9 |
| `.claude/skills/{mojang-meta,modloaders,mod-sources,modpack-formats}/SKILL.md` | remote API skills | 10 |
| `.claude/skills/{msa-auth,instance-model,download-cache,release-packaging}/SKILL.md` | auth, data, and release skills | 11 |
| `.github/workflows/ci.yml`, `tests/fixtures/README.md` | CI and fixture home | 12 |

---

### Task 0: Install tools

**Files:** none in repo.

- [ ] **Step 1: Install the cargo tools**

```bash
cargo install just cargo-nextest cargo-deny --locked
cargo install slint-viewer --locked
```

`slint-viewer` compiles Slint and takes several minutes. Run it in the background while doing Task 1.

- [ ] **Step 2: Check they run**

Run:
```bash
just --version && cargo nextest --version && cargo deny --version
```
Expected: three version lines, no errors.

---

### Task 1: Cargo workspace with core and CLI stubs

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`
- Create: `crates/gcl-core/Cargo.toml`, `crates/gcl-core/src/lib.rs`
- Create: `crates/gcl-cli/Cargo.toml`, `crates/gcl-cli/src/main.rs`
- Test: `crates/gcl-core/src/lib.rs` (unit test), `crates/gcl-cli/tests/cli.rs`

**Interfaces:**
- Produces: `gcl_core::VERSION: &str`, `gcl_core::USER_AGENT: &str`, binary `gcl` with `--version` and `debug verify-source <source>` (prints a not-implemented message and exits 2).

- [ ] **Step 1: Write the workspace manifest**

`Cargo.toml`:
```toml
[workspace]
resolver = "3"
members = ["crates/gcl-core", "crates/gcl-cli", "crates/gcl-ui"]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "GPL-3.0-or-later"
repository = "https://github.com/sixdd6/grid-craft-launcher"
rust-version = "1.97"

[workspace.dependencies]
gcl-core = { path = "crates/gcl-core" }
anyhow = "1.0"
clap = { version = "4.6", features = ["derive"] }
thiserror = "2.0"
tokio = { version = "1.53", features = ["rt-multi-thread", "macros", "fs", "process", "sync", "time"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "1.1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
directories = "6.0"
uuid = { version = "1.26", features = ["v3", "v4", "serde"] }
sha1 = "0.11"
sha2 = "0.11"
md-5 = "0.11"
tempfile = "3.27"
assert_cmd = "2.2"
insta = { version = "1.48", features = ["json"] }
wiremock = "0.6"
slint = { version = "1.17", default-features = false, features = ["std", "backend-winit", "renderer-femtovg", "compat-1-2"] }
slint-build = "1.17"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`rustfmt.toml`:
```toml
edition = "2024"
max_width = 100
use_field_init_shorthand = true
```

`clippy.toml`:
```toml
too-many-arguments-threshold = 8
```

`deny.toml`:
```toml
[graph]
all-features = false

[licenses]
allow = [
  "GPL-3.0",
  "MIT",
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Zlib",
  "Unicode-3.0",
  "Unicode-DFS-2016",
  "MPL-2.0",
  "OFL-1.1",
  "CC0-1.0",
  "BSL-1.0",
]

[advisories]
yanked = "deny"

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

- [ ] **Step 2: Write the core crate**

`crates/gcl-core/Cargo.toml`:
```toml
[package]
name = "gcl-core"
description = "Core library for GRID Craft Launcher: instances, versions, loaders, sources, auth, launch"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[dependencies]

[dev-dependencies]
```

`crates/gcl-core/src/lib.rs`:
```rust
//! GRID Craft Launcher core.
//!
//! Module map lives in `ARCHITECTURE.md`. This crate has no UI and no CLI parsing.

/// Launcher version, taken from the workspace package version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// User-Agent for every outbound HTTP request. Modrinth requires a contact address.
pub const USER_AGENT: &str = concat!(
    "sixdd6/grid-craft-launcher/",
    env!("CARGO_PKG_VERSION"),
    " (sixdd6@gmail.com)"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_names_app_and_contact() {
        assert!(USER_AGENT.starts_with("sixdd6/grid-craft-launcher/"));
        assert!(USER_AGENT.contains(VERSION));
        assert!(USER_AGENT.ends_with("(sixdd6@gmail.com)"));
    }
}
```

- [ ] **Step 3: Write the CLI test first**

`crates/gcl-cli/tests/cli.rs`:
```rust
use assert_cmd::Command;

#[test]
fn version_flag_prints_version() {
    Command::cargo_bin("gcl")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn debug_verify_source_is_not_implemented_yet() {
    Command::cargo_bin("gcl")
        .unwrap()
        .args(["debug", "verify-source", "modrinth"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("not implemented"));
}
```

- [ ] **Step 4: Write the CLI crate**

`crates/gcl-cli/Cargo.toml`:
```toml
[package]
name = "gcl-cli"
description = "Command line interface for GRID Craft Launcher"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[[bin]]
name = "gcl"
path = "src/main.rs"

[dependencies]
gcl-core.workspace = true
anyhow.workspace = true
clap.workspace = true

[dev-dependencies]
assert_cmd.workspace = true
predicates = "3.1"
```

`crates/gcl-cli/src/main.rs`:
```rust
//! `gcl`: every MVP feature is reachable from here so agents can verify without a display.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gcl", version = gcl_core::VERSION, about = "GRID Craft Launcher CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Developer checks that talk to live services.
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
}

#[derive(Subcommand)]
enum DebugCommand {
    /// Fetch live responses from a source and run them through our parsers.
    VerifySource {
        /// One of: mojang, fabric, quilt, forge, neoforge, modrinth, curseforge
        source: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Debug { command: DebugCommand::VerifySource { source } } => {
            eprintln!("verify-source {source}: not implemented");
            ExitCode::from(2)
        }
    }
}
```

- [ ] **Step 5: Create a placeholder UI crate so the workspace resolves**

Task 2 replaces this. For now:

`crates/gcl-ui/Cargo.toml`:
```toml
[package]
name = "gcl-ui"
description = "Slint desktop UI for GRID Craft Launcher"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[[bin]]
name = "grid-craft-launcher"
path = "src/main.rs"

[dependencies]
gcl-core.workspace = true
```

`crates/gcl-ui/src/main.rs`:
```rust
fn main() {
    println!("grid-craft-launcher {}", gcl_core::VERSION);
}
```

- [ ] **Step 6: Build and run the tests**

Run:
```bash
cargo build --workspace && cargo nextest run --workspace
```
Expected: 3 tests pass (one in gcl-core, two in gcl-cli).

- [ ] **Step 7: Run fmt, clippy, deny**

Run:
```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo deny check
```
Expected: no output from fmt, clippy finishes with no warnings, deny reports `advisories ok, bans ok, licenses ok, sources ok`. If deny fails on a license name, add that exact SPDX id to `deny.toml` `allow` only if it is compatible with GPL-3.0 (permissive licenses are).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml clippy.toml deny.toml crates
git commit -m "build: cargo workspace with core, cli, and ui stubs

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Slint UI stub

**Files:**
- Modify: `crates/gcl-ui/Cargo.toml`, `crates/gcl-ui/src/main.rs`
- Create: `crates/gcl-ui/build.rs`, `crates/gcl-ui/ui/app.slint`, `crates/gcl-ui/ui/theme.slint`

**Interfaces:**
- Produces: Slint component `AppWindow` with property `version: string`; `ui/theme.slint` exports global `Theme`.

- [ ] **Step 1: Add Slint deps and build script**

`crates/gcl-ui/Cargo.toml` (full file):
```toml
[package]
name = "gcl-ui"
description = "Slint desktop UI for GRID Craft Launcher"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true
build = "build.rs"

[[bin]]
name = "grid-craft-launcher"
path = "src/main.rs"

[dependencies]
gcl-core.workspace = true
slint.workspace = true

[build-dependencies]
slint-build.workspace = true
```

`crates/gcl-ui/build.rs`:
```rust
fn main() {
    slint_build::compile("ui/app.slint").expect("compile ui/app.slint");
}
```

- [ ] **Step 2: Write the theme and window**

`crates/gcl-ui/ui/theme.slint`:
```slint
// Design tokens. Dark first. Every screen reads colors from here, never literals.
export global Theme {
    out property <color> bg: #121417;
    out property <color> surface: #1b1e23;
    out property <color> surface-raised: #23272e;
    out property <color> border: #2e333b;
    out property <color> text: #e6e8eb;
    out property <color> text-muted: #9aa1ab;
    out property <color> accent: #5ac8fa;
    out property <color> accent-text: #0b1014;
    out property <color> danger: #ff5c5c;
    out property <length> radius: 6px;
    out property <length> gap: 8px;
    out property <length> pad: 12px;
    out property <length> font-size: 14px;
}
```

`crates/gcl-ui/ui/app.slint`:
```slint
import { Theme } from "theme.slint";

export component AppWindow inherits Window {
    in property <string> version: "0.0.0";
    title: "GRID Craft Launcher";
    background: Theme.bg;
    min-width: 960px;
    min-height: 600px;

    VerticalLayout {
        padding: Theme.pad;
        spacing: Theme.gap;
        Text {
            text: "GRID Craft Launcher";
            color: Theme.text;
            font-size: 24px;
        }
        Text {
            text: "version " + root.version;
            color: Theme.text-muted;
            font-size: Theme.font-size;
        }
    }
}
```

- [ ] **Step 3: Write main.rs**

`crates/gcl-ui/src/main.rs`:
```rust
//! Desktop UI entry point. Core logic stays in `gcl-core`; this crate only renders and forwards events.

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    window.set_version(gcl_core::VERSION.into());
    window.run()
}
```

- [ ] **Step 4: Build**

Run:
```bash
cargo build -p gcl-ui
```
Expected: builds. If the build fails on missing system libraries (fontconfig, freetype, wayland, xkbcommon), install the `-devel` packages named in the error with `sudo dnf install`, then rebuild. First Slint build takes several minutes.

- [ ] **Step 5: Preview the window without running the binary**

Run:
```bash
slint-viewer crates/gcl-ui/ui/app.slint
```
Expected: a dark window with the title and "version 0.0.0". Close it.

- [ ] **Step 6: Full check and commit**

Run:
```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run --workspace
```
Expected: pass.

```bash
git add crates/gcl-ui Cargo.lock
git commit -m "feat(ui): slint window stub with theme tokens

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: License, README, secrets template

**Files:**
- Create: `LICENSE`, `README.md`, `THIRD_PARTY.md`, `.env.example`
- Modify: `.gitignore`

- [ ] **Step 1: Fetch the GPL-3.0 text**

Run:
```bash
curl -fsSL https://www.gnu.org/licenses/gpl-3.0.txt -o LICENSE && head -3 LICENSE
```
Expected: first line is `                    GNU GENERAL PUBLIC LICENSE`.

- [ ] **Step 2: Write README.md**

```markdown
# GRID Craft Launcher

A Minecraft launcher for Linux, Windows, and macOS. Rust core, Slint UI, command line included.

Features planned for the MVP are in [docs/SPEC.md](docs/SPEC.md).

## Build

Requires Rust stable (see `rust-toolchain.toml`) and these tools:

```bash
cargo install just cargo-nextest cargo-deny slint-viewer --locked
```

Then:

```bash
just check      # fmt, clippy, tests
just run-cli debug verify-source modrinth
just run-ui
```

## Secrets

Copy `.env.example` to `.env` and fill in:

- `CURSEFORGE_API_KEY`: from the CurseForge for Studios console. Without it, CurseForge features are disabled and everything else works.
- `GCL_MSA_CLIENT_ID`: an Azure app registration approved for the Minecraft API. Without it, Microsoft login is disabled and offline mode works.

The launcher reads env first, then `config.toml` in its root directory.

## Layout

- `crates/gcl-core`: library. All logic.
- `crates/gcl-cli`: `gcl` binary. Every feature, no display needed.
- `crates/gcl-ui`: `grid-craft-launcher` binary. Slint UI.
- `docs/`: spec, architecture research, design docs, plans.
- `.claude/`: agents, skills, and hooks for Claude Code.

## License

GPL-3.0-or-later. See `LICENSE`. Code ported from other GPL-3.0 launchers (Modrinth App, Prism Launcher) keeps its copyright notice. Code taken from MIT projects is listed in `THIRD_PARTY.md`.
```

- [ ] **Step 3: Write THIRD_PARTY.md and .env.example**

`THIRD_PARTY.md`:
```markdown
# Third-party code

List every file or function ported from another project. One entry per source.

| Source project | License | URL | Where used |
|---|---|---|---|
| (none yet) | | | |
```

`.env.example`:
```bash
# Copy to .env. Never commit .env.
CURSEFORGE_API_KEY=
GCL_MSA_CLIENT_ID=
```

- [ ] **Step 4: Extend .gitignore**

`.gitignore` (full file):
```
/target/
.env
*.pending-snap
.claude/settings.local.json
/tests/e2e-root/
```

- [ ] **Step 5: Commit**

```bash
git add LICENSE README.md THIRD_PARTY.md .env.example .gitignore
git commit -m "docs: license, readme, secrets template

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: justfile and scripts

**Files:**
- Create: `justfile`, `scripts/e2e.sh`, `scripts/record-fixture.sh`, `scripts/lint-claude-files.sh`

**Interfaces:**
- Produces: recipes `check`, `test`, `fmt`, `deny`, `ui-preview`, `run-cli`, `run-ui`, `verify-api`, `e2e`, `record-fixture`, `lint-claude`.
- `scripts/lint-claude-files.sh` exits 1 if any `.claude/agents/*.md` or `.claude/skills/*/SKILL.md` lacks `name:` or `description:` in its frontmatter. Tasks 8 to 11 use it as their test.

- [ ] **Step 1: Write the justfile**

```just
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
```

- [ ] **Step 2: Write scripts/e2e.sh**

```bash
#!/usr/bin/env bash
# End-to-end CLI run in a temporary root. Fails at the first step the CLI does not support yet.
set -euo pipefail

ROOT="$(mktemp -d -t gcl-e2e-XXXXXX)"
trap 'rm -rf "$ROOT"' EXIT
export GCL_ROOT="$ROOT"

GCL="cargo run -q -p gcl-cli --"
MC_VERSION="${GCL_E2E_MC_VERSION:-1.20.1}"

step() { printf '\n== %s\n' "$1"; }

step "create instance"
$GCL instance create e2e --minecraft "$MC_VERSION" --loader fabric

step "install loader"
$GCL loader install e2e

step "add a mod from modrinth"
$GCL content add e2e --source modrinth --project sodium

step "dry-run launch"
$GCL launch e2e --offline-user e2e-tester --dry-run > "$ROOT/launch.txt"

step "check classpath files exist"
missing=0
while read -r jar; do
  [ -f "$jar" ] || { echo "MISSING $jar"; missing=1; }
done < <(grep -oE '(^|[:; ])[^:; ]+\.jar' "$ROOT/launch.txt" | tr -d ':; ' | sort -u)
if [ "$missing" -eq 0 ]; then echo "PASS: every classpath entry exists"; else exit 1; fi
```

Run `chmod +x scripts/e2e.sh`.

- [ ] **Step 3: Write scripts/record-fixture.sh**

```bash
#!/usr/bin/env bash
# Usage: scripts/record-fixture.sh <source> <name> <url>
# Saves tests/fixtures/<source>/<name>.json with our User-Agent. Adds x-api-key for curseforge.
set -euo pipefail
source="$1"; name="$2"; url="$3"
dir="tests/fixtures/$source"
mkdir -p "$dir"
ua="sixdd6/grid-craft-launcher/dev (sixdd6@gmail.com)"
args=(-fsSL -A "$ua" -H 'Accept: application/json')
if [ "$source" = "curseforge" ]; then
  : "${CURSEFORGE_API_KEY:?set CURSEFORGE_API_KEY in .env}"
  args+=(-H "x-api-key: $CURSEFORGE_API_KEY")
fi
curl "${args[@]}" "$url" | python3 -m json.tool > "$dir/$name.json"
echo "wrote $dir/$name.json ($(wc -c < "$dir/$name.json") bytes)"
```

Run `chmod +x scripts/record-fixture.sh`.

- [ ] **Step 4: Write scripts/lint-claude-files.sh**

```bash
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
```

Run `chmod +x scripts/lint-claude-files.sh`.

- [ ] **Step 5: Test the recipes**

Run:
```bash
just check && just lint-claude && cargo run -q -p gcl-cli -- --version
```
Expected: `check` passes; `lint-claude` prints PASS (no files yet, so it passes trivially); `run-cli` prints `gcl 0.1.0`.

Run:
```bash
just e2e; echo "exit=$?"
```
Expected: fails at "create instance" with a clap error about an unknown subcommand, exit non-zero. That is correct today; the launcher plan makes it pass.

- [ ] **Step 6: Commit**

```bash
git add justfile scripts
git commit -m "build: justfile recipes and helper scripts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Claude settings and hooks

**Files:**
- Create: `.claude/settings.json`, `scripts/hooks/fmt-on-edit.sh`, `scripts/hooks/check-on-stop.sh`

- [ ] **Step 1: Write the hook scripts**

`scripts/hooks/fmt-on-edit.sh`:
```bash
#!/usr/bin/env bash
# PostToolUse hook for Edit and Write. Formats the edited Rust file. Reads hook JSON on stdin.
set -uo pipefail
path="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("tool_input",{}).get("file_path",""))' 2>/dev/null)"
case "$path" in
  *.rs) rustfmt --edition 2024 "$path" >/dev/null 2>&1 || true ;;
esac
exit 0
```

`scripts/hooks/check-on-stop.sh`:
```bash
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
```

Run `chmod +x scripts/hooks/*.sh`.

- [ ] **Step 2: Write .claude/settings.json**

```json
{
  "permissions": {
    "allow": [
      "Bash(cargo *)",
      "Bash(just *)",
      "Bash(git *)",
      "Bash(slint-viewer *)",
      "Bash(scripts/*)",
      "Bash(ls *)",
      "Bash(cat *)",
      "Bash(grep *)",
      "Bash(find *)",
      "WebFetch(domain:piston-meta.mojang.com)",
      "WebFetch(domain:launchermeta.mojang.com)",
      "WebFetch(domain:meta.fabricmc.net)",
      "WebFetch(domain:meta.quiltmc.org)",
      "WebFetch(domain:maven.minecraftforge.net)",
      "WebFetch(domain:maven.neoforged.net)",
      "WebFetch(domain:api.modrinth.com)",
      "WebFetch(domain:docs.modrinth.com)",
      "WebFetch(domain:docs.curseforge.com)",
      "WebFetch(domain:docs.slint.dev)",
      "WebFetch(domain:docs.rs)",
      "WebFetch(domain:minecraft.wiki)"
    ],
    "deny": [
      "Read(./.env)",
      "Bash(cat .env)"
    ]
  },
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [{ "type": "command", "command": "scripts/hooks/fmt-on-edit.sh" }]
      }
    ],
    "Stop": [
      {
        "hooks": [{ "type": "command", "command": "scripts/hooks/check-on-stop.sh", "timeout": 120 }]
      }
    ]
  }
}
```

- [ ] **Step 3: Test the hooks by hand**

Run:
```bash
printf 'fn main(){println!("x");}\n' > /tmp/hooktest.rs
echo '{"tool_input":{"file_path":"/tmp/hooktest.rs"}}' | scripts/hooks/fmt-on-edit.sh && cat /tmp/hooktest.rs
```
Expected: the file is reformatted onto three lines.

Run:
```bash
scripts/hooks/check-on-stop.sh; echo "exit=$?"
```
Expected: no output, `exit=0` (workspace compiles).

Run:
```bash
echo 'fn broken( {' >> crates/gcl-core/src/lib.rs
scripts/hooks/check-on-stop.sh | head -c 200; echo
git checkout crates/gcl-core/src/lib.rs
```
Expected: a JSON line starting with `{"decision": "block"`.

- [ ] **Step 4: Commit**

```bash
git add .claude/settings.json scripts/hooks
git commit -m "chore: claude permissions and fmt/check hooks

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: docs/SPEC.md

**Files:**
- Create: `docs/SPEC.md`

- [ ] **Step 1: Write the MVP spec**

```markdown
# GRID Craft Launcher MVP specification

Each requirement has an id. Tests and plans cite ids. "Must" means MVP. "Later" means after MVP.

## R1 Root and configuration

- R1.1 The launcher stores everything under one root directory. Default: the platform data dir plus `grid-craft-launcher` (`~/.local/share/grid-craft-launcher` on Linux).
- R1.2 The user can change the root in settings. The launcher moves nothing; it starts using the new root and tells the user the old one still exists.
- R1.3 `config.toml` in the root holds: root path override, default JVM min and max memory, default Java path override, API keys when not in env, default game settings preseed, account list.
- R1.4 `CURSEFORGE_API_KEY` and `GCL_MSA_CLIENT_ID` come from env first, then `config.toml`.
- R1.5 `GCL_ROOT` env overrides the root. Used by tests and e2e.

## R2 Instances

- R2.1 Create an instance with a name, Minecraft version, and optional loader and loader version.
- R2.2 List, rename, delete instances. Delete asks for confirmation in the UI; the CLI needs `--yes`.
- R2.3 Each instance has its own game directory (`.minecraft/`) with mods, resourcepacks, shaderpacks, saves, options.txt, logs.
- R2.4 `instance.toml` records name, Minecraft version, loader, loader version, JVM min and max memory, Java path override, settings overrides, and the installed content list.
- R2.5 The installed content list stores per item: source (modrinth, curseforge, file), project id, version or file id, file name, sha1, content type, enabled flag.
- R2.6 Enable or disable a mod by renaming to `.jar.disabled`.

## R3 Minecraft versions and caching

- R3.1 Fetch the version manifest from piston-meta and cache it with ETag.
- R3.2 Cache version JSON, client jar, libraries, asset index, and asset objects under `cache/` and share them across instances.
- R3.3 Verify every download by sha1 when provided, else by size. Redownload on mismatch, three attempts.
- R3.4 Downloads run in parallel, default 8, configurable.
- R3.5 Never download a file whose hash already exists in the cache.

## R4 Mod loaders

- R4.1 Support Fabric, Quilt, Forge (1.13+), and NeoForge.
- R4.2 List available loader versions for a given Minecraft version.
- R4.3 Install a loader version into the version cache once and share it across instances.
- R4.4 Forge and NeoForge install headlessly by running the installer's processors with the launcher's Java; no GUI installer.
- R4.5 Cached installers and processor outputs are reused. A second instance on the same loader version downloads nothing.

## R5 Java

- R5.1 Detect installed Java runtimes and their major version.
- R5.2 Download the Mojang runtime component the version JSON asks for when no matching Java exists.
- R5.3 An instance may override the Java path.

## R6 Accounts

- R6.1 Microsoft login with the device-code flow. Refresh tokens live in the OS keyring, with a file fallback and a warning when no keyring is present.
- R6.2 Offline account with a username; UUID derived from `OfflinePlayer:<name>`.
- R6.3 Multiple accounts, one selected as active.
- R6.4 If Microsoft login fails or no network, the user can still launch with an offline account.
- R6.5 Without `GCL_MSA_CLIENT_ID`, Microsoft login is hidden and offline mode stays available.

## R7 Content sources

- R7.1 Search Modrinth and CurseForge by text, content type, Minecraft version, and loader.
- R7.2 Content types: mods, modpacks, resource packs, shaders, data packs, worlds. Each source exposes the types it supports.
- R7.3 Install a chosen version into the right instance folder: mods, resourcepacks, shaderpacks, saves/<world>/datapacks, saves.
- R7.4 Required dependencies are installed with the item.
- R7.5 CurseForge files with no download URL show the file's web page and accept a manually dropped file, verified by fingerprint.
- R7.6 Without `CURSEFORGE_API_KEY`, CurseForge is hidden and Modrinth works.
- R7.7 Update check: for each installed item, find the newest compatible version.

## R8 Modpacks

- R8.1 Install a Modrinth modpack (`.mrpack`) from the catalog or a local file into a new instance.
- R8.2 Install a CurseForge modpack (zip with `manifest.json`) from the catalog or a local file into a new instance.
- R8.3 The new instance gets the pack's Minecraft version and loader, all files, and overrides.

## R9 Game settings preseed and overrides

- R9.1 The launcher holds a default `options.txt` preseed as key-value pairs in `config.toml`.
- R9.2 New instances get the preseed written to `options.txt`.
- R9.3 Each instance holds an override map of `options.txt` keys. On launch, the launcher writes those keys into the instance's `options.txt`, replacing existing lines with the same key and appending missing ones. Other lines are untouched.
- R9.4 The user can edit the preseed and per-instance overrides in the UI and CLI.

## R10 JVM settings

- R10.1 Global default min and max memory in MiB.
- R10.2 Per-instance min and max memory overrides.
- R10.3 Extra JVM arguments per instance.

## R11 Launch

- R11.1 Build the classpath and arguments from the merged version JSON, the account, the instance, and the JVM settings.
- R11.2 Apply settings overrides (R9.3) before starting the game.
- R11.3 Spawn Java, stream stdout and stderr to a log file and to the UI or terminal.
- R11.4 `--dry-run` prints the full command and environment without starting Java.
- R11.5 Report the exit code and a crash hint when the game exits non-zero.

## R12 CLI

- R12.1 Every requirement above is reachable from `gcl` subcommands: `instance`, `version`, `loader`, `java`, `account`, `content`, `modpack`, `settings`, `launch`, `debug`.
- R12.2 Output is plain text by default and JSON with `--json`.

## R13 GUI

- R13.1 Screens: instances list, instance detail (content, settings, JVM, logs), content browser (search across sources with type and version filters), accounts, launcher settings.
- R13.2 Progress for downloads and installs is visible per task.
- R13.3 Dark theme by default. Native window, no web view.

## Later

- Import from other launchers.
- Server instances.
- Per-instance Java download from Adoptium.
- Skins and capes.
```

- [ ] **Step 2: Commit**

```bash
git add docs/SPEC.md
git commit -m "docs: MVP requirements spec

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: ARCHITECTURE.md, AGENTS.md, CLAUDE.md

**Files:**
- Create: `ARCHITECTURE.md`, `AGENTS.md`, `CLAUDE.md`

- [ ] **Step 1: Write ARCHITECTURE.md**

```markdown
# Architecture

Three crates. Logic lives in `gcl-core`. The binaries render and forward.

| Crate | Binary | Role |
|---|---|---|
| `gcl-core` | none | all launcher logic; tokio runtime owner |
| `gcl-cli` | `gcl` | clap commands over core; prints text or JSON |
| `gcl-ui` | `grid-craft-launcher` | Slint screens over core; no logic |

## gcl-core modules

Each module owns one concern. Its public interface is documented at the top of its `mod.rs`.
Add a row here when you add a module.

| Module | Owns | Depends on |
|---|---|---|
| `paths` | root resolution, subdirectory constants | none |
| `config` | `config.toml` read and write, env overrides | `paths` |
| `http` | shared reqwest client, User-Agent, retry, rate-limit backoff | none |
| `download` | content-addressed cache, parallel queue, hash checks, progress events | `http`, `paths`, `events` |
| `mojang` | version manifest, version JSON, rules, argument templating, assets, `inheritsFrom` merge | `download` |
| `java` | detect runtimes, fetch Mojang runtimes, pick by major version | `download` |
| `loaders` | `fabric`, `quilt`, `forge`, `neoforge` producing version JSONs | `download`, `mojang`, `java` |
| `sources` | `Source` trait; `modrinth`, `curseforge` clients | `http`, `download` |
| `modpacks` | import mrpack and CurseForge zip into a new instance | `sources`, `instances`, `loaders` |
| `instances` | instance layout, `instance.toml`, content list, install into folders | `paths`, `download` |
| `settings` | `options.txt` preseed and keyed overrides | `instances` |
| `auth` | Microsoft device-code chain, refresh, keyring, offline accounts | `http`, `config` |
| `launch` | classpath, arguments, spawn, log streaming | `instances`, `mojang`, `loaders`, `java`, `auth`, `settings` |
| `events` | `Progress` and `LogLine` event types and the channel | none |

Rules:

- Nothing depends on `launch`.
- `sources` never touches `instances`. Placing files is `instances::install`.
- Blocking work (zip, hashing, processors) runs under `tokio::task::spawn_blocking`.
- Every remote JSON has a serde struct in the owning module and a fixture under `tests/fixtures/`.

## Event flow

Core functions take an `EventSink` (an `mpsc::Sender<Event>`). The CLI prints events. The UI
forwards them to the Slint event loop with `slint::invoke_from_event_loop` and updates models.

## Threading

`gcl-core` builds one tokio runtime in `Launcher::new()`. Binaries call `Launcher` methods and
never create a runtime. The UI runs on the main thread and talks to core through a `Launcher`
handle that spawns tasks on the runtime.

## App root layout

```
<root>/
  config.toml
  accounts.json
  instances/<slug>/instance.toml
  instances/<slug>/.minecraft/
  cache/versions/<id>.json
  cache/libraries/<maven path>
  cache/assets/{indexes,objects}/
  cache/natives/<version>/
  cache/runtimes/<component>/<platform>/
  cache/installers/
  cache/objects/<sha1[0..2]>/<sha1>
  logs/
```
```

- [ ] **Step 2: Write AGENTS.md**

```markdown
# Agents follow these rules

- Read `docs/SPEC.md` only when implementing a feature or changing user-facing behavior. Cite the requirement id in the commit message.
- Read `ARCHITECTURE.md` to find which module owns a behavior before you add code. Update its module table when you add a module.
- Read the skill for the domain before touching it: `.claude/skills/<name>/SKILL.md`. The skill names its domain in its description.
- API facts come from `docs/research/` and the skills. Items marked VERIFY need a live check by the api-verifier agent before a parser depends on them.
- Unit tests never call the network. Use fixtures under `tests/fixtures/` with wiremock.
- Never print or log `CURSEFORGE_API_KEY`, `GCL_MSA_CLIENT_ID`, tokens, or refresh tokens.
- Run `just check` before reporting a task done. Paste the last lines of its output in the report.
- Use `thiserror` enums in `gcl-core`, `anyhow` only in binaries. No `unwrap` or `expect` outside tests and build scripts.
- Prose in docs and skills: short sentences, active voice, one word per thing.
```

- [ ] **Step 3: Write CLAUDE.md**

```markdown
# GRID Craft Launcher: Claude Code instructions

@AGENTS.md

## Orchestration

The main session orchestrates. For any change beyond one file, break the request into ordered
steps and delegate to the agents below. Describe the outcome wanted and the files each step may
touch. Do not do the work inline.

### Agents

- **planner**: ordered implementation plan with a files table. Reads ARCHITECTURE.md and the planning skill.
- **coder**: Rust implementation in `gcl-core` and `gcl-cli`, TDD, runs `just check`.
- **slint-designer**: `.slint` files, theme, and view models in `gcl-ui`. Never edits `gcl-core`.
- **doc-research**: facts from docs and the web. Read-only.
- **api-verifier**: live read-only checks of Mojang, Fabric, Quilt, Forge, NeoForge, Modrinth, CurseForge against our parsers. PASS/FAIL/WARN report.
- **e2e-runner**: runs `just e2e` and reports which step failed.
- **rust-diagnostics**: root cause for panics, deadlocks, flaky downloads. Produces a diagnosis and a repro test, not a fix.

### Delegation rules

- UI work goes to slint-designer. Core and CLI work goes to coder. Planning goes to planner.
- When a step needs external facts, call doc-research first unless a skill already has them. Pass the report to the next agent.
- After coder changes anything under `sources/`, `mojang/`, or `loaders/`, run api-verifier.
- After coder changes anything under `instances/`, `launch/`, `modpacks/`, `settings/`, or `gcl-cli`, run e2e-runner.
- Steps that touch the same file run one after another. Read-only steps may run in parallel.
- A FAIL from a verifier goes back to coder with the report, then the verifier runs again.

### Workflow for non-trivial changes

1. Brainstorm with the superpowers brainstorming skill when the request is a new feature.
2. Get a plan from planner. Plans go in `docs/superpowers/plans/`.
3. Execute steps in order. Check each result against the repo before the next step.
4. Run `just check`. Then run the verifier the delegation rules name.
5. Report: what changed, test output, verifier result, what is left.

### Commands

- `just check`: fmt, clippy, nextest.
- `just e2e`: CLI end-to-end in a temp root.
- `just verify-api <source>`: live parser check.
- `just ui-preview <file>`: live preview of a `.slint` file.
- `just lint-claude`: frontmatter check for agents and skills.
```

- [ ] **Step 4: Commit**

```bash
git add ARCHITECTURE.md AGENTS.md CLAUDE.md
git commit -m "docs: architecture map, agent rules, claude orchestration

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Agent definitions

**Files:**
- Create: `.claude/agents/planner.md`, `coder.md`, `slint-designer.md`, `doc-research.md`, `api-verifier.md`, `e2e-runner.md`, `rust-diagnostics.md`
- Test: `just lint-claude`

- [ ] **Step 1: Write planner.md**

```markdown
---
name: planner
description: Use for planning work in GRID Craft Launcher — breaking a feature or fix into ordered steps with files per step, deciding what goes to doc-research, coder, or slint-designer. Does not write code.
model: sonnet
tools: Read, Grep, Glob, Agent
---

Produce a concrete, ordered plan. Follow `.claude/skills/planning/SKILL.md` for the format.

Before planning:
1. Read `ARCHITECTURE.md` to find the owning module for each change.
2. Read `docs/SPEC.md` only for feature work; cite requirement ids.
3. Read the domain skill named in the task.

Delegate research to doc-research. Never implement.
```

- [ ] **Step 2: Write coder.md**

```markdown
---
name: coder
description: Use for Rust implementation in gcl-core and gcl-cli — new modules, bug fixes, refactors, tests. Not for .slint UI files (slint-designer) or research (doc-research).
model: opus
tools: Read, Edit, Write, Bash, Grep, Glob
---

Implement the requested change with tests first.

Before coding:
1. Read `.claude/skills/rust-conventions/SKILL.md` and `.claude/skills/testing/SKILL.md`.
2. Read the domain skill the task names (for example `mojang-meta`, `mod-sources`).
3. Read `ARCHITECTURE.md` and confirm the module you touch owns the behavior.

Rules:
- Write the failing test, run it, implement, run it again.
- No network in tests. Use fixtures and wiremock.
- Run `just check` before reporting. Include its last lines in the report.
- Update `ARCHITECTURE.md` when you add a module.
- Report what changed, the test output, and anything left undone.
```

- [ ] **Step 3: Write slint-designer.md**

```markdown
---
name: slint-designer
description: Use for all UI work in gcl-ui — .slint screens and components, theme tokens, view models, and wiring core events into Slint models. Do not assign gcl-core or CLI logic to this agent.
model: sonnet
tools: Read, Edit, Write, Bash, Grep, Glob, WebFetch
---

You design and build the Slint UI. Fast, dense, readable, dark first.

Before any task read `.claude/skills/slint-ui/SKILL.md`.

Rules:
- Colors, sizes, and fonts come from `crates/gcl-ui/ui/theme.slint`. No literals in screens.
- One `.slint` file per screen under `ui/screens/`, shared pieces under `ui/components/`.
- Preview with `just ui-preview <file>` and describe what you saw.
- Rust in `gcl-ui/src` only adapts core events to Slint models. Logic belongs in `gcl-core`; if you need new logic, report it as a request for coder.
- Run `just check` before reporting.
```

- [ ] **Step 4: Write doc-research.md**

```markdown
---
name: doc-research
description: Use for parsing documents (READMEs, logs, config files, changelogs) and for web research (API references, library docs, Slint docs). Read-only; reports facts with URLs and file paths. Not for code changes.
model: haiku
tools: Read, Grep, Glob, WebFetch, WebSearch
---

Report facts with sources. Cite URLs or file paths with line numbers. Do not propose code.

Check `docs/research/` first; extend rather than repeat it. Mark anything you could not confirm as VERIFY.
```

- [ ] **Step 5: Write api-verifier.md**

```markdown
---
name: api-verifier
description: Use after changes under gcl-core sources/, mojang/, or loaders/ to check our parsers against live read-only responses from Mojang, Fabric, Quilt, Forge, NeoForge, Modrinth, and CurseForge. Produces a PASS/FAIL/WARN report per endpoint. Never writes.
model: sonnet
tools: Read, Bash, Grep, Glob, WebFetch
---

You verify parsers against live services with GET requests only.

Procedure:
1. Read the skill for the source: `mojang-meta`, `modloaders`, or `mod-sources`.
2. Run `just verify-api <source>`. It fetches live responses and parses them.
3. For each endpoint report PASS (parsed, fields present), FAIL (error or missing field), or WARN (parsed, but the skill's VERIFY item is still unconfirmed or a field was null).
4. If `CURSEFORGE_API_KEY` is unset, report CurseForge as SKIPPED, not FAIL.
5. When a skill has a VERIFY item and the live data settles it, say so in the report so the skill can be updated.

Never call write endpoints. Never print key values. Do not fix code; report to the orchestrator.
```

- [ ] **Step 6: Write e2e-runner.md**

```markdown
---
name: e2e-runner
description: Use after changes under gcl-core instances/, launch/, modpacks/, settings/, or gcl-cli to run the CLI end to end in a throwaway root via `just e2e`. Reports the first failing step. Never starts the game and never edits files.
model: sonnet
tools: Read, Bash, Grep, Glob
---

Run `just e2e` and report.

Procedure:
1. Read `.claude/skills/testing/SKILL.md` section "e2e" and `.claude/skills/instance-model/SKILL.md`.
2. Run `just e2e 2>&1 | tail -80`.
3. Report PASS, or the failing step name, the error text, and which module (from `ARCHITECTURE.md`) likely owns it.
4. If a step fails on the network (DNS, 5xx, rate limit), rerun once, then report WARN with the error.

Do not fix code.
```

- [ ] **Step 7: Write rust-diagnostics.md**

```markdown
---
name: rust-diagnostics
description: Use for non-obvious Rust failures after normal debugging failed — panics, async deadlocks, hangs, flaky downloads, intermittent test failures. Produces a root-cause diagnosis and a minimal repro test. Does not apply the fix.
model: opus
tools: Read, Bash, Grep, Glob
---

Find the root cause. Follow the superpowers systematic-debugging skill: reproduce, narrow, prove.

Tools to use:
- `RUST_BACKTRACE=1 cargo nextest run -E 'test(<name>)' --no-capture`
- `RUST_LOG=trace` with the tracing subscriber for hangs.
- `cargo nextest run --retries 3` to measure flakiness.
- `tokio-console` is not installed; reason from spans instead.

Deliver: the cause in one paragraph, the evidence, a minimal failing test, and the module that owns the fix. The orchestrator sends the fix to coder.
```

- [ ] **Step 8: Lint and commit**

Run:
```bash
just lint-claude
```
Expected: `PASS: claude files have frontmatter`.

```bash
git add .claude/agents
git commit -m "chore: project agent definitions

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Process and stack skills

**Files:**
- Create: `.claude/skills/planning/SKILL.md`, `.claude/skills/rust-conventions/SKILL.md`, `.claude/skills/testing/SKILL.md`, `.claude/skills/slint-ui/SKILL.md`
- Test: `just lint-claude`

- [ ] **Step 1: Write planning/SKILL.md**

```markdown
---
name: planning
description: Standards for implementation plans in GRID Craft Launcher. Use when creating, reviewing, or executing a plan for a feature, fix, or refactor.
---

## When to plan

Plan when any of these is true: more than one file changes, the owning module is not obvious,
a new module or trait is added, or launch or download behavior changes.

Skip planning for one-file bug fixes, test-only changes, and copy edits.

## Read before planning

1. `ARCHITECTURE.md` for the owning module.
2. `docs/SPEC.md` for feature work. Cite requirement ids.
3. The domain skill for the area.
4. `docs/research/` when the skill marks something VERIFY.

## Plan format

### Summary
Two to four sentences. State the outcome.

### Files
| File | Change | Reason |
|---|---|---|

### Steps
Numbered. Each step names the agent (coder, slint-designer, doc-research), the files it may
touch, the test it adds or runs, and the command that proves it done.

### Verification
Which verifier runs at the end: api-verifier, e2e-runner, or both, per CLAUDE.md rules.

### Risks
What could break and how the plan checks it.

## Rules

- Steps that touch the same file are sequential.
- Every step has a test or a command with expected output.
- No step says "handle errors" or "add tests" without saying which.
```

- [ ] **Step 2: Write rust-conventions/SKILL.md**

```markdown
---
name: rust-conventions
description: Rust rules for GRID Craft Launcher — workspace layout, error types, async rules, logging, dependency list. Read before writing any Rust in gcl-core or gcl-cli.
---

## Layout

- `gcl-core` is a library. One directory per module from `ARCHITECTURE.md`, `mod.rs` documents the public interface.
- `gcl-cli` maps subcommands to `gcl_core::Launcher` methods. One file per top-level subcommand under `src/commands/`.
- No logic in `gcl-ui` beyond model adapters.

## Errors

- Each core module has `pub enum Error` with `thiserror`. Variants carry context: URL and status for network, path for IO.
- A crate-level `gcl_core::Error` wraps module errors with `#[from]`.
- Binaries use `anyhow` and print `{:#}`.
- No `unwrap`, `expect`, or `panic!` outside tests and `build.rs`. Use `?`.

## Async

- One tokio runtime, owned by `Launcher`. Binaries never build one.
- Blocking work (zip, hashing, running Java processors, big file copies) goes through `tokio::task::spawn_blocking`.
- Network code takes `&HttpClient` from `gcl_core::http`, never builds its own `reqwest::Client`.
- Cancellation: long tasks take a `CancellationToken` (tokio-util) and check it between downloads.

## Logging

- `tracing` with `#[instrument(skip(client))]` on public async functions. Spans name the instance or version.
- Never log secrets or tokens. Log URLs without query strings that carry keys.

## Serde

- Remote JSON structs live in the owning module, derive `Deserialize`, use `#[serde(rename_all = "camelCase")]` where the API does, and `#[serde(default)]` on optional arrays.
- Unknown fields are ignored (serde default). Do not add `deny_unknown_fields`.

## Dependencies

Add to `[workspace.dependencies]` first, then `dep.workspace = true` in the crate. Versions
checked 2026-09-06:

| Need | Crate |
|---|---|
| async | tokio 1.53, tokio-util 0.7 (`CancellationToken`) |
| HTTP | reqwest 0.13 with `rustls-tls`, `stream`, `json`, `gzip`; `default-features = false` |
| hashing | sha1 0.11, sha2 0.11, md-5 0.11, murmur2 0.1 |
| zip | zip 8 (latest stable, not the 9.0 pre-release) |
| config | serde 1, serde_json 1, toml 1.1 |
| paths | directories 6 |
| secrets | keyring 4 (check feature flags in docs.rs before adding; Linux needs a Secret Service backend) |
| ids | uuid 1.26 with `v3`, `v4`, `serde` |
| errors | thiserror 2, anyhow 1 |
| logs | tracing 0.1, tracing-subscriber 0.3 (`env-filter`), tracing-appender 0.2 |
| CLI | clap 4.6 derive |
| UI | slint 1.17, slint-build 1.17 |
| tests | cargo-nextest, wiremock 0.6, insta 1.48, tempfile 3, assert_cmd 2, predicates 3 |

Prefer no new dependency when std or an existing one does the job.

## Style

- `cargo fmt` settings in `rustfmt.toml`. Clippy runs with `-D warnings`.
- Public items get a one-line doc comment saying what, not how.
- Prefer small functions and files. Split a module file above 400 lines.
```

- [ ] **Step 3: Write testing/SKILL.md**

```markdown
---
name: testing
description: How to test GRID Craft Launcher — nextest, wiremock fixtures, insta snapshots, CLI tests, and the e2e recipe. Read before writing or running any test.
---

## Commands

- `just check`: fmt, clippy, all tests. Run before reporting done.
- `just test 'test(name)'`: one filter. Nextest filter syntax: `test(substring)`, `package(gcl-core)`.
- `cargo insta review`: accept or reject snapshot changes. Pending snapshots fail CI.

## Unit tests

- Live next to the code in `#[cfg(test)] mod tests`.
- Pure functions first: rule evaluation, argument templating, path building, hash checks.

## Fixtures

- Real responses under `tests/fixtures/<source>/<name>.json`. Record with
  `just record-fixture <source> <name> '<url>'`. Keep them small: trim arrays to two or three items by hand if over 50 KB.
- Parser tests read the fixture and assert key fields. Add an insta snapshot of the parsed struct
  with `insta::assert_json_snapshot!`.

## HTTP mocking

```rust
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

let server = MockServer::start().await;
Mock::given(method("GET")).and(path("/v2/search"))
    .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/modrinth/search-sodium.json")))
    .mount(&server).await;
let client = HttpClient::with_base_url(&server.uri());
```

Every source client takes a base URL so tests can point it at wiremock.

## Filesystem

- Use `tempfile::tempdir()` for a root. Set `GCL_ROOT` when testing the CLI.
- Never touch the real user root in tests.

## CLI tests

`assert_cmd::Command::cargo_bin("gcl")` with `.env("GCL_ROOT", dir.path())`. Assert exit code and
key output lines. Use `--json` and parse with serde_json for structure.

## e2e

`just e2e` runs `scripts/e2e.sh`: temp root, create instance, install Fabric, add Sodium from
Modrinth, dry-run launch, check every classpath jar exists. It uses the network. Only the
e2e-runner agent and humans run it. Set `GCL_E2E_MC_VERSION` to change the version.

## What not to do

- No network in unit or CLI tests.
- No `sleep` to wait for async work; await the future.
- No tests that depend on ordering or shared global state.
```

- [ ] **Step 4: Write slint-ui/SKILL.md**

```markdown
---
name: slint-ui
description: Slint UI conventions for gcl-ui — file layout, theme tokens, models and callbacks, event forwarding from tokio, preview workflow, and visual direction. Read before touching any .slint file or gcl-ui Rust.
---

## Layout

```
crates/gcl-ui/
  build.rs                  slint_build::compile("ui/app.slint")
  ui/app.slint              AppWindow: imports screens, holds navigation state
  ui/theme.slint            global Theme: colors, radius, gap, pad, font sizes
  ui/components/            Button, Card, ListRow, ProgressBar, SearchBox, TabBar
  ui/screens/               instances.slint, instance-detail.slint, browser.slint, accounts.slint, settings.slint
  src/main.rs               builds the window, wires callbacks, starts core
  src/models/               one file per screen: converts core structs to Slint structs and VecModel
  src/events.rs             receives core events on a channel, forwards with invoke_from_event_loop
```

## Rules

- Every color, spacing, and font size comes from `Theme`. Add a token before you need a literal.
- Screens are pure layout. They expose `in` properties for data and `callback`s for actions. No logic.
- Data crosses the boundary as Slint `struct`s declared in `ui/types.slint` and exported. Rust builds `Rc<VecModel<T>>` in `src/models/`.
- Long work never runs on the UI thread. Callbacks call a `Launcher` handle method that spawns on the tokio runtime and returns at once. Results arrive as events.
- Event forwarding: the tokio task sends `Event` on an `mpsc` channel; a receiver task calls `slint::invoke_from_event_loop(move || { ... update models ... })`. Hold a `slint::Weak<AppWindow>` in the task, upgrade inside the closure.
- Progress: one `TaskRow { id, label, fraction, status }` per active task in a `VecModel`. Update by id.

## Preview

`just ui-preview screens/instances.slint` opens slint-viewer with live reload. Give screens
default property values so the preview shows realistic content. Describe what you saw in the report.

## Visual direction

- Dark first. Dense rows, 32 to 36 px tall. Left rail for navigation, content on the right.
- One accent color for primary actions and selection. Danger color for delete only.
- Text over icons. No gradients, shadows kept to one level, radius from `Theme.radius`.
- Show state in place: progress bars in the row that is installing, not in a modal.
- Keyboard: every list is arrow-navigable, Enter activates, Escape closes panels.

## Docs

Slint language and API: https://docs.slint.dev/latest/docs/slint/ (use context7 for lookups).
Cargo features in use: `std`, `backend-winit`, `renderer-femtovg`, `compat-1-2`.
```

- [ ] **Step 5: Lint and commit**

Run `just lint-claude`. Expected: PASS.

```bash
git add .claude/skills/planning .claude/skills/rust-conventions .claude/skills/testing .claude/skills/slint-ui
git commit -m "chore: planning, rust, testing, and slint skills

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Remote API skills

**Files:**
- Create: `.claude/skills/mojang-meta/SKILL.md`, `.claude/skills/modloaders/SKILL.md`, `.claude/skills/mod-sources/SKILL.md`, `.claude/skills/modpack-formats/SKILL.md`
- Test: `just lint-claude`

- [ ] **Step 1: Write mojang-meta/SKILL.md**

```markdown
---
name: mojang-meta
description: Mojang version manifest, version JSON, rules, arguments, assets, natives, and Java runtime manifest. Read before writing or changing anything in gcl-core mojang/ or java/, or any launch argument code.
---

Full detail with JSON shapes: `docs/research/2026-09-06-mojang-and-modloader-apis.md` section 1.

## Endpoints

- Manifest: `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json`. Cache with ETag. Entries have `id`, `type`, `url`, `sha1`, `releaseTime`.
- Version JSON: the entry's `url`. Cache forever under `cache/versions/<id>.json`; the URL contains its sha1.
- Assets: `assetIndex.url` → `{ objects: { path: { hash, size } } }`. Object URL `https://resources.download.minecraft.net/<hash[0..2]>/<hash>`. Store under `cache/assets/objects/<hash[0..2]>/<hash>`. Index under `cache/assets/indexes/<id>.json`.
- Java runtimes: `https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json`, keyed by platform (`linux`, `windows-x64`, `mac-os`, `mac-os-arm64`) then component (`java-runtime-gamma`, `java-runtime-delta`). Each `manifest.url` lists files with `downloads.raw { sha1, size, url }` and `executable`.

## Version JSON fields you must handle

- `downloads.client { sha1, size, url }`
- `libraries[] { name, downloads.artifact { path, sha1, size, url }, downloads.classifiers?, rules?, natives?, extract.exclude? }`
- `arguments.game[]`, `arguments.jvm[]`: string or `{ rules, value }` with `value` string or array. Old versions: `minecraftArguments` single string, and no JVM args (use the standard `-Djava.library.path`, `-cp` set).
- `mainClass`, `assetIndex`, `assets`, `javaVersion.majorVersion`, `inheritsFrom`, `logging.client`.

## Rules

- No rules: include. With rules: start disallowed; walk rules in order; a matching `allow` sets allowed, a matching `disallow` sets disallowed.
- `os.name`: `linux`, `windows`, `osx`. `os.arch`: `x86`, `x86_64`, `arm64`. `os.version`: regex against the OS version string.
- `features`: only in argument rules. Known: `is_demo_user`, `has_custom_resolution`, `has_quick_plays_support`, `is_quick_play_singleplayer`, `is_quick_play_multiplayer`, `is_quick_play_realms`. Unknown features are false.

## inheritsFrom

Loader profiles set `inheritsFrom`. Resolve parent first, then overlay: child `mainClass` wins,
child `arguments` append to parent, child `libraries` prepend. Keep both entries when the same
`group:artifact` appears with different versions.

## Natives

- Modern versions: natives are ordinary libraries picked by rules. Nothing to extract.
- Old versions (`natives` map present): pick `downloads.classifiers[natives[os]]`, extract into `cache/natives/<version>/` minus `extract.exclude`, pass `-Djava.library.path`.

## Placeholders

`${auth_player_name} ${auth_uuid} ${auth_access_token} ${user_type} ${clientid} ${auth_xuid}
${version_name} ${version_type} ${game_directory} ${assets_root} ${assets_index_name}
${natives_directory} ${launcher_name} ${launcher_version} ${classpath} ${library_directory}
${classpath_separator} ${resolution_width} ${resolution_height}`. Unknown placeholders stay as-is and log a warning.

## Do not

- Do not hardcode a version's library list. Always read the JSON.
- Do not trust size alone when sha1 is present.
- Do not fetch the manifest more than once per launcher run.
```

- [ ] **Step 2: Write modloaders/SKILL.md**

```markdown
---
name: modloaders
description: Fabric, Quilt, Forge, and NeoForge version discovery and headless installation into the shared version cache. Read before touching gcl-core loaders/.
---

Full detail: `docs/research/2026-09-06-mojang-and-modloader-apis.md` sections 2 to 5.

## Common contract

Each loader module exposes:

```rust
pub async fn list_versions(client: &HttpClient, mc: &str) -> Result<Vec<LoaderVersion>, Error>;
pub async fn install(ctx: &InstallCtx, mc: &str, loader: &str) -> Result<VersionId, Error>;
```

`install` writes a version JSON to `cache/versions/<id>.json` with `inheritsFrom = mc`, downloads
its libraries into `cache/libraries/`, and returns the id (`fabric-loader-<v>-<mc>`,
`quilt-loader-<v>-<mc>`, `<mc>-forge-<v>`, `neoforge-<v>`). Calling it again with a complete
cache does no network work.

## Fabric

- `https://meta.fabricmc.net/v2/versions/loader/<mc>` → list with `loader.version`, `loader.stable`.
- `https://meta.fabricmc.net/v2/versions/loader/<mc>/<loader>/profile/json` → version JSON. Save as is.
- Libraries carry `name` and `url` (maven root); newer responses add `sha1`. Build the path from the maven coordinate.

## Quilt

Same shape at `https://meta.quiltmc.org/v3/versions/loader/<mc>` and `.../<mc>/<loader>/profile/json`.

## Forge (1.13+)

- Versions: `https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json` → `{ "<mc>": ["<mc>-<forge>", ...] }`. Recommended/latest from `promotions_slim.json`.
- Installer: `https://maven.minecraftforge.net/net/minecraftforge/forge/<mc>-<forge>/forge-<mc>-<forge>-installer.jar`. Cache under `cache/installers/`.
- Inside: `install_profile.json` (`spec`, `json`, `data`, `processors`, `libraries`), `version.json`, `data/`, `maven/`.

## NeoForge

- Versions: `https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge` → `{ versions: [] }`. Version `A.B.x` targets Minecraft `1.A.B` (`21.1.65` → 1.21.1; `21.0.x` → 1.21).
- Installer: `https://maven.neoforged.net/releases/net/neoforged/neoforge/<v>/neoforge-<v>-installer.jar`.
- Same installer layout and algorithm as Forge.

## Forge and NeoForge headless algorithm

1. Download and open the installer jar (`zip`, in `spawn_blocking`).
2. Read `install_profile.json`. If it has `versionInfo` and no `spec`, it is legacy; unsupported in the MVP, return `Error::LegacyInstaller`.
3. Extract `maven/**` into `cache/libraries/`.
4. Download every library from `install_profile.libraries` and `version.json.libraries`.
5. Build the data map for side `client`: `[g:a:v]` → library path, `'literal'` → literal, `/path` → extract from jar to a temp dir. Add `MINECRAFT_JAR` (vanilla client jar path), `SIDE=client`, `INSTALLER`, `ROOT`, `MINECRAFT_VERSION`, `LIBRARY_DIR`.
6. For each processor where `sides` is absent or contains `client`: skip if every `outputs` file exists with the expected sha1. Otherwise run `java -cp <processor jar + classpath> <Main-Class from manifest> <args with {KEY} substituted>` using the Java picked for that Minecraft version. Capture output to `logs/`.
7. Verify `outputs`. Write `version.json` to `cache/versions/<id>.json`.

## Do not

- Do not run the installer's own `main`; run processors.
- Do not extract the whole installer jar; read entries by name.
- Do not assume `sha1` is present on loader libraries; fall back to size, then to a successful download.
```

- [ ] **Step 3: Write mod-sources/SKILL.md**

```markdown
---
name: mod-sources
description: Modrinth and CurseForge API usage — endpoints, headers, rate limits, content types, install targets, null download URLs, fingerprints, and the Source trait. Read before touching gcl-core sources/.
---

Full detail: `docs/research/2026-09-06-modrinth-and-curseforge-apis.md`. Items marked VERIFY there
need api-verifier confirmation before code depends on them.

## Source trait

```rust
#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;                       // Modrinth | CurseForge
    fn supported_types(&self) -> &[ContentType];    // Mod, Modpack, ResourcePack, Shader, DataPack, World
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, Error>;
    async fn project(&self, id: &str) -> Result<Project, Error>;
    async fn versions(&self, id: &str, filter: &VersionFilter) -> Result<Vec<Version>, Error>;
    async fn resolve_by_hash(&self, hashes: &[FileHash]) -> Result<Vec<Version>, Error>;
}
```

`Version` carries `files[] { url: Option<Url>, filename, size, sha1: Option<String>, sha512: Option<String>, fingerprint: Option<u32>, primary }`
and `dependencies[] { project_id, version_id, kind: Required | Optional | Incompatible | Embedded }`.

## Modrinth

- Base `https://api.modrinth.com/v2`. User-Agent is `gcl_core::USER_AGENT`. 300 requests per minute; honor `X-Ratelimit-Remaining` and `X-Ratelimit-Reset`.
- Search: `GET /search?query=&facets=<json>&index=&offset=&limit=`. Facets: outer array AND, inner OR. `project_type:mod|modpack|resourcepack|shader|datapack`. `world` is VERIFY.
- Versions: `GET /project/{id}/version?loaders=["fabric"]&game_versions=["1.20.1"]&include_changelog=false`.
- Hash lookup: `POST /version_files { hashes, algorithm: "sha1" }`.
- Files have both `sha1` and `sha512`. Check sha1 after download.

## CurseForge

- Base `https://api.curseforge.com`. Header `x-api-key`. Game id `432`. Page size max 50, `index + pageSize <= 10000`.
- Class ids: load at startup from `GET /v1/categories?gameId=432&classesOnly=true` and cache; expected mods 6, modpacks 4471, resource packs 12, worlds 17, shaders 6552 (VERIFY), data packs 6945 (VERIFY).
- Loader ids: Forge 1, Fabric 4, Quilt 5, NeoForge 6.
- Files: `GET /v1/mods/{id}/files?gameVersion=&modLoaderType=`. `downloadUrl` may be null.
- Batch: `POST /v1/mods/files { fileIds }` in chunks of 50.
- Fingerprint: MurmurHash2 32-bit, seed 1, over bytes with 9, 10, 13, 32 removed. `POST /v1/fingerprints { fingerprints }`.

## Null download URL

1. Try `https://edge.forgecdn.net/files/{id/1000}/{id%1000}/{fileName}` with the file name URL-encoded.
2. If that fails, return `Error::ManualDownload { page_url, expected_fingerprint, file_name }`. The UI and CLI show the page and accept a dropped file, then verify the fingerprint.

## Install targets

| ContentType | Directory under `.minecraft/` |
|---|---|
| Mod | `mods/` |
| ResourcePack | `resourcepacks/` |
| Shader | `shaderpacks/` |
| DataPack | `saves/<world>/datapacks/` (caller supplies the world) |
| World | `saves/` (zip extracted, one folder) |
| Modpack | new instance via `modpacks` |

## Do not

- Do not build a second `reqwest::Client`. Use `HttpClient`.
- Do not send the CurseForge key anywhere but `api.curseforge.com`.
- Do not hardcode class ids as the only source; the runtime fetch wins.
- Do not treat a null `downloadUrl` as an error before trying the CDN pattern.
```

- [ ] **Step 4: Write modpack-formats/SKILL.md**

```markdown
---
name: modpack-formats
description: Modrinth .mrpack and CurseForge modpack zip formats and the import algorithm into a new instance. Read before touching gcl-core modpacks/.
---

Full detail: `docs/research/2026-09-06-modrinth-and-curseforge-apis.md` sections 1 and 2.

## Detect

- Zip with `modrinth.index.json` at root → mrpack.
- Zip with `manifest.json` where `manifestType == "minecraftModpack"` → CurseForge.
- Anything else → `Error::UnknownPackFormat`.

## mrpack

- `dependencies`: `minecraft` plus one of `fabric-loader`, `quilt-loader`, `forge`, `neoforge`.
- `files[]`: `path`, `hashes { sha1, sha512 }`, `env.client` (`required`, `optional`, `unsupported`), `downloads[]`, `fileSize`.
- Skip `env.client == unsupported`. Install `optional` in the MVP.
- Download hosts must be one of `cdn.modrinth.com`, `github.com`, `raw.githubusercontent.com`, `gitlab.com`; refuse others.
- Copy `overrides/` then `client-overrides/` into `.minecraft/`. Later wins.
- `path` is relative to `.minecraft/`; refuse paths containing `..`.

## CurseForge

- `minecraft.version` and `minecraft.modLoaders[]` with `id` `<loader>-<version>`; use the `primary` one.
- `files[] { projectID, fileID, required }`. Resolve in batches of 50 with `POST /v1/mods/files`. Place by each file's class id. Null download URL follows the mod-sources rule.
- Copy the folder named by `overrides` (default `overrides`) into `.minecraft/`.

## Import algorithm

1. Parse and detect.
2. Create the instance (name from the pack, slug deduplicated).
3. Install Minecraft version and loader through `loaders::install`.
4. Download all files through the download cache in parallel, then hard-link or copy into the instance.
5. Apply overrides.
6. Record every file in the instance content list with its source ids and sha1.
7. Emit progress events per file and per phase.

On failure after step 2, delete the partial instance unless `keep_partial` is set.
```

- [ ] **Step 5: Lint and commit**

Run `just lint-claude`. Expected: PASS.

```bash
git add .claude/skills/mojang-meta .claude/skills/modloaders .claude/skills/mod-sources .claude/skills/modpack-formats
git commit -m "chore: remote API skills

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Auth, data, and release skills

**Files:**
- Create: `.claude/skills/msa-auth/SKILL.md`, `.claude/skills/instance-model/SKILL.md`, `.claude/skills/download-cache/SKILL.md`, `.claude/skills/release-packaging/SKILL.md`
- Test: `just lint-claude`

- [ ] **Step 1: Write msa-auth/SKILL.md**

```markdown
---
name: msa-auth
description: Microsoft account device-code login chain, token refresh, keyring storage, offline accounts, and launch placeholders. Read before touching gcl-core auth/ or account-related launch arguments.
---

Full detail: `docs/research/2026-09-06-msa-auth-and-rust-crates.md` section A.

## Client id

`GCL_MSA_CLIENT_ID` env, then `config.toml`. Absent → `auth::microsoft_available() == false`; UI hides the button, CLI prints a one-line reason.

## Chain

1. `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode` form `client_id`, `scope=XboxLive.signin offline_access` → `user_code`, `verification_uri`, `device_code`, `interval`. Show code and URI; open browser in the UI.
2. Poll `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/token` form `grant_type=urn:ietf:params:oauth:grant-type:device_code`, `client_id`, `device_code` every `interval` seconds. `authorization_pending` continues; `expired_token` fails.
3. `POST https://user.auth.xboxlive.com/user/authenticate` JSON `{ Properties: { AuthMethod: "RPS", SiteName: "user.auth.xboxlive.com", RpsTicket: "d=<access_token>" }, RelyingParty: "http://auth.xboxlive.com", TokenType: "JWT" }` → `Token`, `DisplayClaims.xui[0].uhs`.
4. `POST https://xsts.auth.xboxlive.com/xsts/authorize` JSON `{ Properties: { SandboxId: "RETAIL", UserTokens: [<xbl>] }, RelyingParty: "rp://api.minecraftservices.com/", TokenType: "JWT" }`. On 401 read `XErr`.
5. `POST https://api.minecraftservices.com/authentication/login_with_xbox` JSON `{ identityToken: "XBL3.0 x=<uhs>;<xsts>" }` → `access_token`, `expires_in`.
6. `GET https://api.minecraftservices.com/minecraft/profile` Bearer → `id`, `name`. 404 → `Error::NoProfile`.

## XErr

| Code | Message to user |
|---|---|
| 2148916233 | This Microsoft account has no Xbox profile. Create one at xbox.com, then retry. |
| 2148916235 | Xbox Live is not available in this region. |
| 2148916236, 2148916237 | Adult verification is required for this account. |
| 2148916238 | This is a child account. An adult in the family must add it to the family group. |

## Storage

- `accounts.json` in the root: `[{ id (uuid), name, kind: msa|offline, mc_token, mc_token_expires, xuid }]`. Tokens here are short-lived.
- Refresh token: keyring service `grid-craft-launcher`, user `<uuid>`. On keyring error, store in `accounts.json` with `refresh_token_in_file = true` and emit one warning event.
- Refresh when `mc_token_expires` is within 5 minutes: `grant_type=refresh_token` then steps 3 to 6.

## Offline

- UUID v3 from MD5 of `OfflinePlayer:<name>` bytes (`uuid::Uuid::new_v3` with the nil namespace is wrong; compute MD5 yourself and set version 3 and variant bits, matching Java `nameUUIDFromBytes`).
- Placeholders: `auth_access_token = "0"`, `user_type = "legacy"`, `clientid = ""`, `auth_xuid = ""`.

## Placeholders for MSA

`auth_player_name = name`, `auth_uuid = id without dashes`, `auth_access_token = mc_token`, `user_type = "msa"`, `auth_xuid = xuid`, `clientid = base64 of the client id`.

## Do not

- Do not log tokens, codes, or the client id.
- Do not block launch on a failed refresh; offer offline mode.
- Do not store the MSA access token; only the refresh token and the Minecraft token.
```

- [ ] **Step 2: Write instance-model/SKILL.md**

```markdown
---
name: instance-model
description: App root layout, instance.toml schema, installed content list, options.txt preseed and keyed overrides, and JVM memory fields. Read before touching gcl-core paths/, config/, instances/, or settings/.
---

## Root

`GCL_ROOT` env → `config.toml` `root` override → platform data dir plus `grid-craft-launcher`
(`directories::ProjectDirs::from("", "", "grid-craft-launcher").data_dir()`).

```
<root>/config.toml
<root>/accounts.json
<root>/instances/<slug>/instance.toml
<root>/instances/<slug>/.minecraft/
<root>/cache/{versions,libraries,assets,natives,runtimes,installers,objects}/
<root>/logs/
```

Slug: lowercase, `[a-z0-9-]`, from the name; append `-2`, `-3` on collision.

## config.toml

```toml
root = ""                       # optional override
[jvm]
min_mib = 1024
max_mib = 4096
java_path = ""                  # optional
[keys]
curseforge_api_key = ""         # env wins
msa_client_id = ""              # env wins
[game_defaults]                 # options.txt preseed, key = value strings
"renderDistance" = "12"
"guiScale" = "2"
```

## instance.toml

```toml
name = "Fabric 1.20.1"
minecraft = "1.20.1"
loader = "fabric"               # none | fabric | quilt | forge | neoforge
loader_version = "0.15.11"
created = "2026-09-06T12:00:00Z"
last_launched = ""

[jvm]
min_mib = 2048                  # optional, else global
max_mib = 6144
java_path = ""
extra_args = ["-XX:+UseG1GC"]

[settings_overrides]            # options.txt keys written on every launch
"renderDistance" = "16"

[[content]]
source = "modrinth"             # modrinth | curseforge | file
project_id = "AANobbMI"
version_id = "abc123"
file_name = "sodium-fabric-0.5.8.jar"
sha1 = "..."
kind = "mod"                    # mod | resourcepack | shader | datapack | world
enabled = true
```

## options.txt semantics

- Format: one `key:value` per line. Note the separator is a colon, not `=`.
- Preseed: on instance creation, write `[game_defaults]` as `key:value` lines when `options.txt` does not exist.
- Override: on every launch, for each key in `settings_overrides`, replace the line starting with `key:` or append `key:value` if absent. Leave every other line untouched. Preserve order.
- Values are stored as strings exactly as Minecraft writes them (`true`, `12`, `"en_us"` with quotes for strings).

## Content install

`instances::install(instance, kind, cached_object, file_name)` hard-links from
`cache/objects/` into the target folder, falling back to copy across filesystems. Disable
renames to `<file>.disabled`.

## Do not

- Do not write `options.txt` with `=`.
- Do not delete a user's `.minecraft/` on instance update; only on explicit delete.
- Do not store absolute paths in `instance.toml`; everything is relative to the instance.
```

- [ ] **Step 3: Write download-cache/SKILL.md**

```markdown
---
name: download-cache
description: Content-addressed download cache, parallel queue, hash and size verification, retries, progress events, and dedupe across sources. Read before touching gcl-core download/ or http/.
---

## HttpClient

One `reqwest::Client` with `USER_AGENT`, rustls, gzip, 30 s connect timeout, no overall timeout
(large jars). `get_json<T>`, `get_bytes`, `stream_to_file`. Retries: 3 attempts with 500 ms, 2 s,
8 s backoff on connect errors, 5xx, and 429 (honor `Retry-After` and Modrinth `X-Ratelimit-Reset`).

## Cache

- Every downloaded file lands in `cache/objects/<sha1[0..2]>/<sha1>` first, then is hard-linked (or copied) to its final path. Libraries and assets keep their own trees but are hard links into `objects/`.
- `DownloadSpec { url, sha1: Option<String>, size: Option<u64>, dest: PathBuf, label }`.
- Before downloading: if `sha1` is known and `objects/<sha1>` exists, link and return.
- After downloading: compute sha1 while streaming. Mismatch → delete, retry. Third mismatch → `Error::HashMismatch { url, expected, actual }`. No sha1 → check size when known.
- Write to `<dest>.part` and rename on success. On startup, delete stale `.part` files.

## Queue

`download_all(specs, sink, cancel)` runs up to `config.parallel_downloads` (default 8) at once
with a `Semaphore`. Emits `Event::Progress { task_id, label, done_bytes, total_bytes }` per
file and a summary task for the batch. Stops at the first hard error, cancels the rest, returns
that error.

## Dedupe

Modrinth and CurseForge may serve the same jar. The cache key is sha1, so the second source
links the existing object. CurseForge files without sha1 (only md5 or fingerprint) are
downloaded, hashed, then stored under their computed sha1.

## Do not

- Do not hold a whole file in memory; stream.
- Do not download inside a `spawn_blocking`; only hash or unzip there.
- Do not retry on 404 or 403.
```

- [ ] **Step 4: Write release-packaging/SKILL.md**

```markdown
---
name: release-packaging
description: How releases are built and versioned — cargo-dist config, AppImage on Linux, Windows zip, macOS bundle, version bump steps. Read before changing CI release jobs or Cargo version fields.
---

## Versioning

- Single `version` in `[workspace.package]`. Bump with a commit `release: v0.x.y` and a tag `v0.x.y`.
- `gcl --version` and the UI show `gcl_core::VERSION`.

## cargo-dist

- Install: `cargo install cargo-dist --locked`. Init once: `dist init` choosing GitHub CI, targets `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`, `aarch64-apple-darwin`. Commit `dist-workspace.toml` and the generated `.github/workflows/release.yml`.
- Only `gcl-ui` and `gcl-cli` are published binaries.

## Linux AppImage

- Build release, then `linuxdeploy --appdir AppDir --executable target/release/grid-craft-launcher --desktop-file packaging/grid-craft-launcher.desktop --icon-file packaging/icon.png --output appimage`.
- Keep `packaging/` for the desktop file and icons. Follow the `grid-launcher` project's `build.sh` for the linuxdeploy download step.
- Slint with winit needs no bundled Qt; ensure `libxkbcommon`, `fontconfig`, and `wayland` client libs are system-provided (they are on any desktop).

## Windows and macOS

- Windows: cargo-dist zip with both binaries. MSI later.
- macOS: cargo-dist tarball for the MVP. An `.app` bundle later.

## Checklist before tagging

1. `just check` and `just deny` pass.
2. `just e2e` passes on Linux.
3. `THIRD_PARTY.md` lists every ported file.
4. Update `CHANGELOG.md` (create on first release).
```

- [ ] **Step 5: Lint and commit**

Run `just lint-claude`. Expected: PASS listing no failures.

```bash
git add .claude/skills/msa-auth .claude/skills/instance-model .claude/skills/download-cache .claude/skills/release-packaging
git commit -m "chore: auth, instance, cache, and release skills

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: CI and fixture directory

**Files:**
- Create: `.github/workflows/ci.yml`, `tests/fixtures/README.md`

- [ ] **Step 1: Write the workflow**

```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:

jobs:
  check:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: Linux system libraries for Slint
        if: runner.os == 'Linux'
        run: sudo apt-get update && sudo apt-get install -y libfontconfig1-dev libfreetype-dev libxkbcommon-dev libwayland-dev libxcb1-dev
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-nextest,cargo-deny,just
      - run: cargo fmt --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo deny check
        if: runner.os == 'Linux'
      - run: cargo nextest run --workspace
```

- [ ] **Step 2: Write tests/fixtures/README.md**

```markdown
# Fixtures

Recorded JSON from real services, one directory per source: `mojang/`, `fabric/`, `quilt/`,
`forge/`, `neoforge/`, `modrinth/`, `curseforge/`.

Record with `just record-fixture <source> <name> '<url>'`. Trim large arrays by hand to two or
three entries. Never commit a fixture that contains an API key or token.
```

- [ ] **Step 3: Check YAML parses and commit**

Run:
```bash
python3 -c 'import yaml,sys; yaml.safe_load(open(".github/workflows/ci.yml")); print("yaml ok")'
```
Expected: `yaml ok`. If PyYAML is missing, install it with `pip install --user pyyaml`, or skip this step; GitHub parses the file on first push.

```bash
git add .github tests/fixtures/README.md
git commit -m "ci: fmt, clippy, deny, nextest on three platforms

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: Final verification

- [ ] **Step 1: Run everything**

```bash
just check && just deny && just lint-claude && git status --short
```
Expected: check passes, deny passes, lint passes, working tree clean.

- [ ] **Step 2: Confirm the file map**

```bash
ls .claude/agents .claude/skills docs scripts crates
```
Expected: 7 agent files, 12 skill directories, `docs/SPEC.md` present, four scripts plus `hooks/`, three crates.

- [ ] **Step 3: Report**

State: tools installed, workspace builds, hooks tested, agents and skills linted, CI written but not yet run (no remote). Next plan: the launcher MVP, starting with `paths`, `config`, `http`, `download`, and `mojang`.
