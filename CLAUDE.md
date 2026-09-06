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
- `just deny`: license and advisory check.
- `just fmt`: format the workspace.
- `just test '<filter>'`: run tests matching a nextest filter.
- `just run-cli <args>`: run the CLI binary.
- `just run-ui`: run the UI binary.
- `just e2e`: CLI end-to-end in a temp root.
- `just verify-api <source>`: live parser check.
- `just ui-preview <file>`: live preview of a `.slint` file.
- `just lint-claude`: frontmatter check for agents and skills.
- `just record-fixture <source> <name> '<url>'`: save a live JSON response as a test fixture.
