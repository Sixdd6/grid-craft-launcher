# Plan 7: Usable App Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the desktop app usable end to end: every control works and a headless GUI test proves it; create → install → launch → stop works from the GUI; game settings use typed controls; widgets render dark; modpacks are searchable.

**Architecture:** see `docs/superpowers/specs/2026-09-07-usable-app-design.md`. `gcl-ui` splits into a library (`src/lib.rs`: everything except `main`) and a thin binary so integration tests can build the real window under `i-slint-backend-testing`. A `tests/support/` module builds a `Launcher` on a temp root with wiremock (reuse `crates/gcl-core/tests/common/mod.rs` patterns), a stand-in Java (`sh` script), and a `wait_until` helper. Core gains `settings::catalog`, `SettingsDoc`, `stop_instance`, and `search_packs`.

**Tech Stack:** as before, plus `i-slint-backend-testing` 1.17 (dev-dep), `tracing-appender`.

**Spec:** the design doc above; `docs/SPEC.md` R9, R11, R13; research `docs/research/2026-09-07-options-txt-catalog.md` (Task 1 writes it from the research report).

## Global Constraints

- All prior constraints (thiserror/anyhow, no unwrap outside tests, workspace deps, `just check` per task, commit per task with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`, no network/keyring in tests, no core call on the UI thread, theme tokens only, secrets never displayed).
- Every interactive Slint element that a user can click or type into has a stable element name (`snake_case := Button { ... }`); GUI tests find elements only by id.
- GUI tests never open a real window: they call `i_slint_backend_testing::init_integration_test_with_mock_time()` first, in a process where no other backend was set (one test binary per flow group; `--test-threads=1` via `[[test]] harness` settings or a `serial` guard).
- The stand-in Java for launch tests is a `#!/bin/sh` script that writes its args to a file and sleeps until a stop file appears (so Stop can be tested) or exits 0 after 1 s.
- Settings values are stored exactly as Minecraft writes them; the catalog is data, not code (a `const` table), and unknown keys always survive.
- Nothing pushed; no remote.

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `docs/research/2026-09-07-options-txt-catalog.md`, `crates/gcl-core/src/settings/catalog.rs`, `settings/doc.rs` | key catalog, layered settings document | 1 |
| `crates/gcl-core/src/launcher/mod.rs`, `launch/spawn.rs`, `gcl-cli` `stop` | stop support, process registry | 2 |
| `crates/gcl-core/src/sources/*`, `launcher` | `search_packs` | 2 |
| `crates/gcl-ui/src/lib.rs`, `main.rs`, `logging.rs`, element ids in every `.slint`, `Palette.color-scheme` | lib split, logging, ids, dark widgets | 3 |
| `crates/gcl-ui/tests/support/mod.rs`, `tests/flow_instances.rs` | harness + create/launch/stop flow | 4 |
| fixes found by Task 4 | wherever | 4 |
| `ui/components/setting-row.slint`, `ui/components/settings-editor.slint`, screens, `src/screens/settings_editor.rs` | typed settings editor (defaults + overrides) | 5 |
| `tests/flow_settings.rs`, `tests/flow_content.rs`, `tests/flow_accounts.rs`, browser pack search | remaining flows + modpack search UI | 6 |
| docs, skills, SPEC status | docs and a live run on the user's machine | 7 |

---

### Task 1: Settings catalog and layered document (core)

- [ ] Write `docs/research/2026-09-07-options-txt-catalog.md` from the controller-provided research report (the controller places it at `.superpowers/sdd/<plan>/options-research.md`; copy the tables, keep citations).
- [ ] `settings/catalog.rs`: `pub enum Control { Slider { min: f64, max: f64, step: f64, decimals: u8 }, Toggle, Choice(&'static [(&'static str, &'static str)]), Text }`, `pub struct Setting { pub key: &'static str, pub label: &'static str, pub group: Group, pub control: Control, pub default: &'static str }`, `pub enum Group { Video, Controls, Sound, Chat, Other }`, `pub const CATALOG: &[Setting]` (at least: renderDistance, simulationDistance, fov, maxFps, gamma, guiScale, fullscreen, enableVsync, graphicsMode, particles, renderClouds, entityShadows, bobView, mouseSensitivity, invertYMouse, autoJump, toggleCrouch, toggleSprint, rawMouseInput, soundCategory_master/music/record/weather/block/hostile/neutral/player/ambient/voice, showSubtitles, chatOpacity, chatScale, chatWidth, narrator, lang, mainHand, attackIndicator, biomeBlendRadius, mipmapLevels, screenEffectScale, fovEffectScale), with ranges/values per the research doc; `pub fn find(key) -> Option<&'static Setting>`; `pub fn format_value(setting, f64|bool|&str) -> String` and `pub fn parse_value(setting, &str) -> Result<Value, Error>` that round-trip Minecraft's stored forms (`true`/`false`; floats with at least one decimal; ints bare; strings bare; lists verbatim).
- [ ] `settings/doc.rs`: `pub enum Layer { Default, Preseed, Override }`, `pub struct Row { pub key: String, pub value: String, pub source: Layer, pub setting: Option<&'static Setting> }`, `pub fn merged(preseed: &BTreeMap<String,String>, overrides: Option<&BTreeMap<String,String>>, current_options: Option<&OptionsFile>) -> Vec<Row>` (catalog order first, then unknown keys from the layers and the file, alphabetical), `pub fn validate(setting, value) -> Result<(), Error>`.
- [ ] `Launcher`: `settings_rows_for_defaults()`, `settings_rows_for_instance(slug)`, `set_game_default(key, value)` / `unset_game_default(key)` (validate against the catalog when known), existing instance override setters validate too.
- [ ] Tests: catalog has no duplicate keys and every Slider has min < max; `format`/`parse` round-trips for each control kind; `merged` layering and source attribution; unknown keys preserved; validation rejects out-of-range and bad enum.
- [ ] Commit `feat(core): options.txt catalog and layered settings document`.

---

### Task 2: Stop support and pack search (core + CLI)

- [ ] `launch/spawn.rs`: `RunningGame` keeps the `Child`; `launch::stop(child: &mut Child, grace: Duration)`: unix `SIGTERM` via `nix::sys::signal::kill` (add `nix` with `signal` feature) or `child.start_kill()` after grace; windows `child.kill()`. `Launcher` keeps `running: Mutex<HashMap<String, Arc<Mutex<Option<tokio::process::Child>>>>>`; `launch_instance_async` registers the child before the wait task takes it (the wait task holds the `Arc` and takes the child out only for `wait`; use `child.wait()` on a `&mut` inside the mutex-free path: store `Child` in an `Arc<tokio::sync::Mutex<Option<Child>>>`, the wait task locks it, calls `.wait().await`, and `stop_instance` locks it and calls `start_kill()`/signal by pid). Simplest robust approach: keep the pid in the registry and signal the pid (`nix::sys::signal::kill(Pid::from_raw(pid), SIGTERM)`), then after 10 s `SIGKILL`; the wait task removes the registry entry on exit. `Launcher::stop_instance(slug) -> Result<()>` (`NotRunning` error when absent). `gcl stop <slug>`? The CLI blocks during launch; add `gcl launch --detach` later — skip the CLI for now, add `Launcher::running_slugs()`.
- [ ] `sources`: `Source::search_packs(&self, q: &SearchQuery) -> Result<SearchPage>` (Modrinth: facet `project_type:modpack`, hits kind reported via a new `SearchHit.is_pack: bool`; CurseForge: `classId = modpacks`); `Launcher::search_packs(source, q)`; `modpacks::fetch_pack` unchanged.
- [ ] Tests: stop test with a `sh` stand-in that traps TERM and exits 143 → `stop_instance` returns Ok and the wait outcome is `Exited { code: 143 }`; `search_packs` over the Modrinth search fixture with a modpack facet (record `tests/fixtures/modrinth/search_packs.json` live: `?query=fabulously&facets=[["project_type:modpack"]]&limit=3`); CurseForge with the synthetic search fixture.
- [ ] Commit `feat(core): stop a running game, modpack search`.

---

### Task 3: gcl-ui library split, logging, element ids, dark widgets

- [ ] `crates/gcl-ui/src/lib.rs` exports `app::build`, `Bridge`, `events`, `screens`, `models`, `launch_flow`, `state`, generated Slint modules (`slint::include_modules!()` in lib); `main.rs` only parses args, sets up logging, builds, runs. `gcl-ui` `Cargo.toml`: `[lib]` + `[[bin]]`.
- [ ] `src/logging.rs`: `init(root: &Root)` → `tracing_subscriber` with `tracing_appender::rolling::daily(root.logs_dir(), "gui.log")` non-blocking writer (keep the guard alive in `main`), plus stderr when `GCL_LOG` is set (env-filter). `Bridge::run`/`run_with_error` log `info!(label, ms)` on success and `error!(label, %err)` on failure. The error dialog appends "Details: <root>/logs/gui.log".
- [ ] Element ids: name every Button, LineEdit, TextEdit, ComboBox, CheckBox, Switch, Slider, SpinBox, TouchArea-row, tab, rail entry, dialog action in all `.slint` files with a `snake_case := ` name; document the naming rule in the `slint-ui` skill.
- [ ] `AppWindow`: `Palette.color-scheme: ColorScheme.dark;` (verify the property name against `cargo doc -p slint`); check the std-widgets now render dark in `just ui-preview app.slint`.
- [ ] `--smoke` unchanged. `just check`; commit `refactor(ui): library crate, file logging, element ids, dark widgets`.

---

### Task 4: GUI test harness and the create → launch → stop flow (fix what breaks)

- [ ] `crates/gcl-ui/tests/support/mod.rs`: `TestApp::new()` → temp root (`GCL_ROOT`), wiremock for Mojang (synthetic vanilla from `gcl-core/tests/common` — copy the helper) and Fabric, a stand-in Java script (writes args, waits for `<root>/stop` up to 30 s, exits 0), `Launcher::open_with_endpoints(...).with_secret_store(MemoryStore)` and `config.jvm.java_path` pointing at the script; builds `AppWindow` via `gcl_ui::app::build`; helpers `click(id)`, `type_into(id, text)`, `select_combo(id, index)` (`invoke_accessible_expand_action` + increment), `wait_until(|| cond, Duration)` implemented with a `slint::Timer` and a shared flag, driving from `slint::spawn_local`; `run(flow)` starts the event loop and quits when the flow finishes or panics (propagate the panic message).
- [ ] `tests/flow_instances.rs`: (a) empty root → instances screen shows the empty state; (b) click `create_button`, type a name, wait for versions, pick the first version, keep loader "none", click `create_confirm` → row appears; (c) create with loader fabric → loader versions load → create → progress task appears → row shows installed; (d) click `launch_button` on the row → status "Running", `InstanceState.running` true, stand-in Java received `--username`; click `stop_button` → the script exits, status "Exited", row not running; (e) rename via dialog; delete via confirm.
- [ ] Fix every failure the flow reveals in `gcl-ui` (or core when the bug is there). Expected suspects from the user's report: the create dialog never enabling Create; ComboBox label/row mismatch; errors swallowed; `launch` needing an installed instance first; Java download path on a machine with only Java 25 (`ensure_java_for` must download the Mojang runtime for 1.20.1 and show progress).
- [ ] Also run the app for real on this machine once: `GCL_ROOT=$(mktemp -d) GCL_LOG=info just run-ui` for 60 s is not clickable; instead run the CLI equivalent against the real root: `just run-cli instance create demo --minecraft 1.20.1 --loader fabric` and `just run-cli launch demo --dry-run`, and report the Java resolution path (does `ensure_java` download `java-runtime-gamma`?).
- [ ] Commit `test(ui): headless flow tests for instances; fix create and launch`.

---

### Task 5: Typed settings editor

- [ ] `ui/components/setting-row.slint`: `SettingRow { key, label, control_kind: string, value: string, number: float, min, max, step, decimals, checked: bool, choices: [string], choice_index: int, source: string /* default|preseed|override|file */, inherited: bool; callbacks changed(string key, string value), reset(string key) }` rendering `Slider` + numeric label, `Switch`, `ComboBox`, or `LineEdit` by kind; an "inherited" pill and a Reset button when overridden.
- [ ] `ui/components/settings-editor.slint`: groups as collapsible cards with rows (`[SettingRowModel]`), a search box filtering by key/label, an "Advanced" group for unknown keys with add/remove raw rows.
- [ ] Slint model `SettingRowModel` in `types.slint`; `src/models` conversion from `settings::doc::Row`; `src/screens/settings_editor.rs` shared wiring used by the Settings screen (defaults layer) and the Instance detail Settings tab (override layer): `changed` → validate in core → save → reload rows; `reset` → unset.
- [ ] Replace the raw key/value editors in both screens with the editor; keep the read-only "current options.txt" list under Advanced.
- [ ] Tests: model conversion (slider bounds, choice index, formatting), and `tests/flow_settings.rs`: open Settings → drag is not scriptable; use `invoke_accessible_increment_action` on `render_distance_slider` → row shows the new value → `config.toml` `game_defaults.renderDistance` updated; toggle `fullscreen_switch`; instance override of the same key shows source "override" and Reset returns to preseed.
- [ ] Commit `feat(ui): typed game settings editor with sliders, toggles and choices`.

---

### Task 6: Remaining flows and modpack search

- [ ] `tests/flow_content.rs`: browser search (wiremock Modrinth fixtures) → Add to instance → content list shows the row → Disable/Enable → Remove; modpack tab: search packs → Install → name prompt → new instance appears; install from file (a synthetic mrpack in the temp dir).
- [ ] `tests/flow_accounts.rs`: add offline → select → remove; Microsoft button disabled without a client id, and with one (wiremock six endpoints) sign-in completes through the device-code dialog and Cancel works.
- [ ] Browser: modpack kind shows a search result list via `Launcher::search_packs` with an Install button per row (keeps the by-id and file panels).
- [ ] Fix every failure found. Commit `test(ui): content, modpack and account flows; modpack search in the browser`.

---

### Task 7: Docs, skill, and a live run

- [ ] Update `ARCHITECTURE.md` (lib/bin split, logging, tests), `.claude/skills/slint-ui/SKILL.md` (element-id rule, testing harness usage, `wait_until`, dark palette), `.claude/skills/testing/SKILL.md` (GUI flow tests: how to run, one backend per process, stand-in Java), `.claude/skills/instance-model/SKILL.md` (catalog + layers), `docs/SPEC.md` R9/R11/R13 notes and the MVP status rows that change (settings editor, Stop, modpack search, GUI verified by tests), README (settings editor, Stop, `GCL_LOG`, `logs/gui.log`), CHANGELOG Unreleased.
- [ ] Live run on this machine against the real root: `just run-cli instance create demo --minecraft 1.20.1 --loader fabric`, `just run-cli launch demo --offline-user Player --dry-run`; then start the GUI (`just run-ui`) for 15 s and take a screenshot with `Window::take_snapshot` behind a `--screenshot <path>` flag added in Task 3 (saves a PNG after first frame and quits) to attach to the report.
- [ ] `just check && just deny && just lint-claude && just e2e`; commit `docs: usable app`.
