# GRID Craft Launcher: AI toolchain design

Date: 2026-09-06
Status: approved in chat, pending written review

## 1. Purpose

Set up the repository so that Claude Code agents can build the launcher with few
hallucinated API details, verifiable steps, and a fixed division of labor. This spec covers the
toolchain only: repo layout, license, agents, skills, hooks, docs, and CI. The launcher's own
features are listed in `docs/SPEC.md`, which this toolchain produces as its first artifact.

Decisions already made:

| Decision | Choice |
|---|---|
| Stack | Rust, Slint UI (winit backend, FemtoVG renderer), tokio async core |
| License | GPL-3.0-or-later |
| Accounts | Microsoft device-code login and offline mode, both in the MVP |
| Surfaces | CLI and desktop GUI, both in the MVP, over one core library |
| Toolchain style | Mirror grid-launcher: project agents, skills, CLAUDE.md, AGENTS.md, ARCHITECTURE.md, SPEC.md |

## 2. Repository layout

```
grid-craft-launcher/
  Cargo.toml                   workspace; shared deps in [workspace.dependencies]
  rust-toolchain.toml          pin stable channel
  rustfmt.toml, clippy.toml, deny.toml
  justfile                     one recipe per agent task (see section 6)
  LICENSE                      GPL-3.0-or-later full text
  README.md                    what it is, how to build, where the secrets go
  CLAUDE.md                    orchestration rules for Claude Code (imports AGENTS.md)
  AGENTS.md                    rules every agent follows, tool-agnostic
  ARCHITECTURE.md              module ownership map; agents update it when they add a module
  .env.example                 CURSEFORGE_API_KEY=, GCL_MSA_CLIENT_ID=
  .gitignore                   target/, .env, *.pending-snap, .claude/settings.local.json
  .github/workflows/ci.yml     fmt, clippy, deny, nextest on Linux, Windows, macOS
  crates/
    gcl-core/                  library crate, no UI, no CLI parsing
    gcl-cli/                   binary `gcl`; clap; every MVP feature reachable here
    gcl-ui/                    binary `grid-craft-launcher`; Slint; ui/*.slint
  docs/
    SPEC.md                    MVP product requirements, testable statements
    research/                  API research reports (three exist today)
    superpowers/specs/         design docs
    superpowers/plans/         implementation plans
  tests/fixtures/              recorded JSON and small zips for mocked tests
  .claude/
    settings.json              permissions and hooks
    agents/                    seven agent definitions
    skills/                    twelve skills
```

### gcl-core module map

Each module has one owner concern and a public interface documented at the top of its
`mod.rs`. ARCHITECTURE.md repeats this table and stays current.

| Module | Owns |
|---|---|
| `paths` | app root resolution (config, default XDG data dir, user override), subdirectory constants |
| `config` | launcher settings file (`config.toml`): root path, JVM defaults, API keys, account list |
| `http` | one shared reqwest client, User-Agent, retry policy, rate-limit backoff |
| `download` | content-addressed cache keyed by sha1, parallel download queue, hash and size checks, progress events |
| `mojang` | version manifest, version JSON, asset index, rule evaluation, argument templating, `inheritsFrom` merge |
| `java` | detect installed Java, download Mojang runtimes, pick a runtime by `javaVersion.majorVersion` |
| `loaders` | `fabric`, `quilt`, `forge`, `neoforge` submodules; each yields a version JSON into the shared version cache |
| `sources` | `modrinth` and `curseforge` clients behind one `Source` trait: search, project, versions, download, fingerprint |
| `modpacks` | import `.mrpack` and CurseForge zip into a new instance |
| `instances` | instance directory layout, `instance.toml`, create, list, delete, installed content list |
| `settings` | preseed `options.txt` from launcher defaults; keyed overrides per instance |
| `auth` | Microsoft device-code chain, refresh, keyring storage, offline accounts |
| `launch` | build classpath and arguments, spawn Java, stream logs, exit status |
| `events` | progress and log event types sent to CLI and UI over a channel |

Cross-module rule: `launch` depends on `instances`, `mojang`, `loaders`, `java`, `auth`,
`settings`. Nothing depends on `launch`. `sources` never touches `instances`; `modpacks` and a
thin `install` function in `instances` do the placing.

### App root layout (user-configurable root)

```
<root>/
  config.toml
  accounts.json               cached tokens and profiles; refresh tokens live in the keyring
  instances/<slug>/
    instance.toml             name, mc version, loader + version, jvm min/max, java path override,
                              settings overrides, installed content list with source ids and hashes
    .minecraft/               game dir: mods/, resourcepacks/, shaderpacks/, saves/, options.txt, logs/
  cache/
    versions/<id>.json        vanilla and loader version JSONs
    libraries/<maven path>    shared by every instance
    assets/indexes/, assets/objects/
    natives/<version>/
    runtimes/<component>/<platform>/
    installers/forge-*.jar, neoforge-*.jar
    objects/<sha1[0..2]>/<sha1>   content-addressed downloads (mods, packs) before hard-link or copy into instances
  logs/
```

## 3. License

`LICENSE` holds GPL-3.0-or-later. Every crate's `Cargo.toml` sets
`license = "GPL-3.0-or-later"`. `deny.toml` allows GPL-3.0, MIT, Apache-2.0, BSD-2/3, ISC,
Zlib, Unicode, MPL-2.0 and rejects the rest so a permissive-only crate cannot pull in an
incompatible dependency. The README says how porting from Modrinth App or Prism is allowed
and how MIT code needs attribution in `THIRD_PARTY.md`.

## 4. Agents

All live in `.claude/agents/`. Frontmatter fields: `name`, `description`, `model`, `tools`.
The global `doc-research` agent is reused unchanged; the project copies of `planner` and
`coder` replace the global ones because the descriptions must point at this repo's skills.

| Agent | Model | Tools | Job | Must read first |
|---|---|---|---|---|
| planner | sonnet | Read, Grep, Glob, Agent | ordered plan with files per step, delegates research to doc-research | ARCHITECTURE.md, planning skill |
| coder | opus | Read, Edit, Write, Bash, Grep, Glob | implement in Rust, TDD, run `just check` before reporting | rust-conventions, testing, domain skill named in the task |
| slint-designer | sonnet | Read, Edit, Write, Bash, Grep, Glob, WebFetch | `.slint` files, theme, view models in `gcl-ui`; never edits `gcl-core` | slint-ui skill |
| doc-research | haiku | Read, Grep, Glob, WebFetch, WebSearch | facts from docs and web, read-only | global |
| api-verifier | sonnet | Read, Bash, Grep, Glob, WebFetch | live read-only GET calls to Mojang, Fabric, Quilt, Forge, NeoForge, Modrinth, and CurseForge when a key exists; compares real JSON to our parsers via `just verify-api <source>`; PASS/FAIL/WARN report | mojang-meta, modloaders, mod-sources |
| e2e-runner | sonnet | Read, Bash, Grep, Glob | runs `just e2e` in a temp root: create instance, install loader, add a mod, resolve launch command, check every classpath file exists; never starts the game | testing, instance-model |
| rust-diagnostics | opus | Read, Bash, Grep, Glob | root cause for panics, deadlocks, flaky downloads; produces a diagnosis and a minimal repro test; does not fix | rust-conventions, testing |

Delegation rules go in CLAUDE.md:

- UI work to slint-designer, core and CLI work to coder, planning to planner.
- External facts through doc-research before planner or coder, unless a skill already has them.
- After coder finishes anything under `sources/`, `mojang/`, or `loaders/`, run api-verifier.
- After coder finishes anything under `instances/`, `launch/`, `modpacks/`, or the CLI, run e2e-runner.
- Two steps that touch the same file run sequentially.
- Any FAIL goes back to coder, then the verifier reruns.

## 5. Skills

All live in `.claude/skills/<name>/SKILL.md`. Each has a frontmatter `description` written so
the trigger is obvious, a "read this when" line, the facts, and a "do not" list. API skills cite
`docs/research/` and say which items are marked VERIFY.

| Skill | Content |
|---|---|
| planning | when to plan, documents to read, plan format with files table, step order, verification per step. Ported from grid-launcher. |
| rust-conventions | workspace layout, error types (`thiserror` in core, one error enum per module), async rules (no blocking in async, `spawn_blocking` for zip and hashing), tracing spans, no `unwrap` outside tests, clippy pedantic subset, crate list from research |
| testing | nextest, wiremock fixture pattern (`tests/fixtures/<source>/<name>.json`), insta for parsed structs, `assert_cmd` for CLI, temp roots via `tempfile`, how to record a new fixture with api-verifier, what `just check` runs |
| slint-ui | crate layout for `gcl-ui`, one `.slint` per screen under `ui/screens/`, shared components under `ui/components/`, theme tokens in `ui/theme.slint`, Rust side: `slint::Model` adapters, `invoke_from_event_loop` for progress events from tokio, preview with `slint-viewer`, design direction (dense, fast, dark-first, no web-style chrome) |
| mojang-meta | version manifest, version JSON fields, rules, argument templating, assets, natives, Java runtime manifest, caching rules |
| modloaders | Fabric and Quilt profile fetch, Forge and NeoForge headless install with processors, version cache layout, MVP limit of Forge 1.13+ |
| mod-sources | Modrinth and CurseForge endpoints, headers, rate limits, project types and class IDs, install target per type, null download URL handling, fingerprint algorithm, `Source` trait shape |
| modpack-formats | mrpack and CurseForge manifest, override order, env filtering, batch file resolution |
| msa-auth | device-code chain, endpoints and bodies, XErr table, refresh, keyring with file fallback, offline UUID, launch placeholders |
| instance-model | app root layout, `instance.toml` schema, content list entries, settings preseed and keyed override semantics for `options.txt` (key `=` value lines; override map wins; unknown keys appended), JVM memory fields |
| download-cache | content-addressed store, hard-link then copy fallback into instances, parallelism (8 by default), retry with backoff, verify sha1 or size, progress events, dedupe across sources |
| release-packaging | cargo-dist config, AppImage via linuxdeploy, Windows zip and MSI later, macOS app bundle later, version bump steps |

## 6. Tools, hooks, CI

### justfile

| Recipe | Runs |
|---|---|
| `check` | `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace` |
| `test <filter>` | `cargo nextest run -E 'test(<filter>)'` |
| `fmt` | `cargo fmt` |
| `deny` | `cargo deny check` |
| `ui-preview <file>` | `slint-viewer crates/gcl-ui/ui/<file>` |
| `run-cli *args` | `cargo run -p gcl-cli -- {{args}}` |
| `run-ui` | `cargo run -p gcl-ui` |
| `verify-api <source>` | `cargo run -p gcl-cli -- debug verify-source <source>` (parses live responses, prints PASS/FAIL per endpoint) |
| `e2e` | script under `scripts/e2e.sh`: temp root, `gcl instance create`, `gcl loader install fabric`, `gcl mod add modrinth sodium`, `gcl launch --dry-run`, check files |
| `record-fixture <source> <name> <url>` | fetches and stores JSON under `tests/fixtures/` |

### .claude/settings.json

- Permissions allow: `Bash(cargo *)`, `Bash(just *)`, `Bash(git *)`, `Bash(slint-viewer *)`,
  `WebFetch` for the API hosts listed in the research docs.
- Hook PostToolUse on Edit and Write: if the path ends in `.rs`, run `cargo fmt -- <file>`.
- Hook Stop: run `cargo check --workspace --quiet`; on failure return a blocking message so the
  session fixes the build before it ends.

### CI

`.github/workflows/ci.yml`: matrix on ubuntu, windows, macos; steps `cargo fmt --check`,
`cargo clippy -D warnings`, `cargo deny check`, `cargo nextest run`. No live API calls in CI.

### Secrets

`.env.example` documents `CURSEFORGE_API_KEY` and `GCL_MSA_CLIENT_ID`. `.env` is ignored.
The app reads env first, then `config.toml`. Agents never print key values.

## 7. Documents

- `CLAUDE.md`: imports AGENTS.md, lists agents, delegation rules, workflow for non-trivial
  changes (plan, execute in order, `just check`, verifier, report), and a pointer to the
  superpowers skills for brainstorming and plans.
- `AGENTS.md`: read SPEC.md only for feature work, ARCHITECTURE.md to find an owner module,
  the matching skill before touching a domain, no live API calls in unit tests, no secrets in
  logs, update ARCHITECTURE.md when adding a module.
- `docs/SPEC.md`: the MVP list from the user rewritten as numbered, testable requirements,
  grouped: root and config, instances, versions and caching, loaders, Java, accounts, sources
  and content types, modpacks, settings preseed and overrides, JVM memory, launch, CLI, GUI.
- `ARCHITECTURE.md`: crate and module tables from section 2, event flow (core → channel → CLI
  printer or UI model), threading rule (tokio runtime owned by the core; UI calls into it via
  a handle).

## 8. Error handling

- Core returns typed errors. Binaries render them. Network errors carry the URL and status.
- Missing CurseForge key disables CurseForge features with one clear message; nothing else fails.
- Missing MSA client ID disables Microsoft login; offline mode still works.
- Download verification failure deletes the file and retries up to three times, then errors.
- Forge processor failure keeps the temp dir and points to it in the error.

## 9. Testing strategy

- Unit tests per module, fixtures for every remote JSON shape, insta snapshots for parsed
  structs and generated launch commands.
- CLI integration tests with `assert_cmd` against a temp root and a wiremock server.
- Live verification only through api-verifier and e2e-runner, never in `just check` or CI.
- The GUI is tested by hand and by slint-viewer preview; core logic stays out of the UI crate
  so nothing important is untestable.

## 10. Out of scope for this toolchain task

Writing any launcher code. The next step is an implementation plan that scaffolds the
workspace, docs, agents, skills, hooks, and CI, then a second plan for the launcher MVP.
