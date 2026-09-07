# MVP Plan 5: Slint GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native desktop app (`grid-craft-launcher`) over `gcl-core` that covers SPEC R13: instances list, instance detail (content, settings, JVM, logs), content browser across sources, accounts (offline and Microsoft), launcher settings, per-task progress, dark theme. Every action the CLI can do is reachable from the GUI.

**Architecture:** `gcl-ui` holds no logic. `src/bridge.rs` runs `Launcher` calls on worker threads and posts results to the UI thread with `Weak::upgrade_in_event_loop`. `src/events.rs` drains the core event receiver on one thread and batches updates into a `TaskRow` model and a log model. `src/models/` converts core structs to Slint structs. Screens are `.slint` files with `in` properties and callbacks only; `ui/app.slint` owns navigation; `ui/theme.slint` owns every color and size. Core gets one preparatory change so a `Launcher` can be shared across threads.

**Tech Stack:** Slint 1.17 (winit + FemtoVG, `fluent` style, resources embedded), `slint-build`, `slint-viewer` for preview; core as before.

**Spec:** `docs/SPEC.md` R13.1-R13.3 plus every user-facing requirement that the CLI already meets (R1-R12); skills `slint-ui`, `rust-conventions`, `testing`; research `docs/research/2026-09-06-slint-1.17-desktop-patterns.md`.

## Global Constraints

- All prior global constraints (thiserror/anyhow, no unwrap outside tests, workspace deps, `just check` per task, commit per task with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`, no network or keyring in tests, secrets never displayed).
- No logic in `gcl-ui`: no version picking, no path building, no HTTP. If a screen needs a fact, `gcl-core` exposes it.
- Every color, spacing, radius, and font size comes from `ui/theme.slint`. No literals in screens or components.
- One `.slint` file per screen under `ui/screens/`, shared pieces under `ui/components/`, all Slint structs in `ui/types.slint`, one global `App` in `ui/app.slint` for navigation state and cross-screen callbacks.
- Rust side: `Launcher` is shared as `Arc<Launcher>`; every core call runs on a worker thread through `bridge::run` and never on the UI thread; results come back through `upgrade_in_event_loop`. `ModelRc` and `Image` are built on the UI thread only.
- Progress: core `Event`s feed a `TaskRow { id, label, fraction, status }` model updated by id; logs feed a bounded ring (last 2000 lines) per screen that shows them.
- Tokens and keys never reach a Slint property: accounts show name, kind, expiry; settings show `<set>`/`<unset>` for keys with a "change" field that writes and clears.
- The app starts with `Launcher::new(None)`; a failure shows a dialog with the error and a Quit button.
- Tests: `src/models/` conversions and `src/events.rs` batching are unit-tested (no Slint instance needed); `bridge` is tested with a fake job; the app has a `--smoke` flag that opens the window, loads the instances screen, and quits after the first frame (`slint::Timer::single_shot`), used by hand (`just run-ui -- --smoke`) since CI has no display. Every screen has default property values so `just ui-preview screens/<name>.slint` shows realistic content.
- Keyboard: lists are arrow-navigable through `FocusScope`, Enter activates, Escape closes dialogs.

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `crates/gcl-core/src/launcher/mod.rs`, `config` | `Launcher: Send + Sync`, `config()` behind `RwLock`, `msa_login_with_cancel`, `Launcher::launch_status` | 1 |
| `crates/gcl-ui/build.rs`, `Cargo.toml`, `ui/theme.slint`, `ui/types.slint`, `ui/components/*.slint`, `ui/app.slint` | theme, types, shared components, shell with left rail | 2 |
| `crates/gcl-ui/src/{main,app,bridge,events}.rs`, `src/models/mod.rs` | startup, worker bridge, event forwarder, model conversions | 2 |
| `ui/screens/instances.slint`, `src/models/instances.rs`, `src/screens/instances.rs` | instances list, create/delete dialogs | 3 |
| `ui/screens/instance.slint`, `src/models/instance.rs`, `src/screens/instance.rs` | detail with content/settings/jvm/logs tabs, launch | 4 |
| `ui/screens/browser.slint`, `src/models/browser.rs`, `src/screens/browser.rs` | search, filters, add, modpack install, pending manual | 5 |
| `ui/screens/accounts.slint`, `ui/screens/settings.slint`, `src/screens/{accounts,settings}.rs` | accounts incl. Microsoft device-code dialog; launcher settings | 6 |
| `ui/components/progress-panel.slint`, `ui/components/toast.slint`, `src/screens/progress.rs` | task list, warnings, error dialog | 7 |
| docs, skill, README, packaging desktop file | docs | 8 |

---

### Task 1: Core preparation for a shared Launcher

**Files:** `crates/gcl-core/src/launcher/mod.rs`, `crates/gcl-core/tests/launcher_flow.rs`

- [ ] `Launcher` must be `Send + Sync`: add `#[cfg(test)] fn assert_send_sync<T: Send + Sync>() {}` test calling it with `Launcher`. Fix whatever blocks it (`OnceLock`, `Arc<dyn ProcessRunner>`, `Box<dyn SecretStore>` are fine; check `Vec<BoxSource>`, `tokio::runtime::Runtime`).
- [ ] Config behind `RwLock<Config>`: `config(&self) -> ConfigRead<'_>` (a guard deref-ing to `Config`), `update_config(&self, f: impl FnOnce(&mut Config)) -> Result<(), Error>` that mutates and saves atomically; keep `config_mut` only if no caller needs `&mut self` semantics (remove it and update the CLI: `config set-*` use `update_config`). `sources()` cache must rebuild when the CurseForge key changes: `update_config` invalidates the source cache (`OnceLock` → `RwLock<Option<Vec<BoxSource>>>`) — document.
- [ ] `msa_login_with_cancel(&self, on_code, cancel: CancellationToken)`: like `msa_login` but with a caller-provided child token (`self.cancel_token().child_token()` by default in `msa_login`).
- [ ] `Launcher::instance_summary(&self, slug) -> Result<InstanceSummary>`: `{ instance: Instance, installed_version_id: Option<String> (cache/versions/<id>.json exists), pending_manual: Vec<ManualDownload>, java: Option<PathBuf> (configured) }` so the detail screen needs one call.
- [ ] `Launcher::launch_instance_async(&self, slug, account, offline_user) -> Result<RunningLaunch>` where `RunningLaunch { pid: u32, log_path: PathBuf, wait: JoinHandle<Result<LaunchOutcome>> }`: spawn without blocking so the GUI can show "running" and keep working (`launch::spawn` + a runtime task doing `wait`); `launch_instance` (blocking) stays for the CLI.
- [ ] Tests: send/sync assertion; `update_config` persists and rebuilds sources (key set → two sources); `launch_instance_async` dry-run is not applicable — test with `sh -c "exit 3"` by pointing `jvm.java_path` at a script? Simpler: keep `launch_instance_async` untested beyond compile plus a unit test of the `RunningLaunch` join with a fake child (spawn `sh -c 'exit 3'` through `launch::spawn` directly, as plan-2 tests do).
- [ ] `just check`; commit `feat(core): thread-safe launcher for the gui`.

---

### Task 2: UI shell, theme, types, bridge, events

**Files:** `crates/gcl-ui/{build.rs,Cargo.toml}`, `ui/{theme,types,app}.slint`, `ui/components/{rail,button,card,list-row,search-box,tab-bar,dialog,progress-bar,empty-state}.slint`, `src/{main,app,bridge,events}.rs`, `src/models/mod.rs`

**Interfaces:**
```slint
// types.slint (export all)
export struct InstanceRow { slug: string, name: string, minecraft: string, loader: string, loader_version: string, last_launched: string, installed: bool, running: bool }
export struct ContentRow { project_id: string, source: string, name: string, version: string, kind: string, enabled: bool, update_available: bool, file_name: string }
export struct SearchRow { source: string, project_id: string, slug: string, title: string, description: string, author: string, kind: string, downloads: string, page_url: string }
export struct AccountRow { id: string, name: string, kind: string, expires: string, active: bool }
export struct TaskRow { id: int, label: string, fraction: float, status: string /* running|done|failed */, detail: string }
export struct LogLine { level: string, text: string }
export struct SettingRow { key: string, value: string }
export struct LoaderVersionRow { version: string, stable: bool, recommended: bool }
export struct VersionRow { id: string, kind: string, release_time: string }
export struct PendingRow { project_id: string, file_name: string, page_url: string, kind: string }
export enum Screen { instances, instance, browser, accounts, settings }
// app.slint
export global App { in-out property <Screen> screen: instances; in-out property <string> current_slug; in-out property <string> status_text; in-out property <bool> busy; in-out property <[TaskRow]> tasks; in-out property <[LogLine]> app_log; in-out property <string> error_title; in-out property <string> error_text; in-out property <bool> error_open;
  callback navigate(Screen); callback open_instance(string); callback dismiss_error(); }
export component AppWindow inherits Window { /* rail + screen switch + progress panel + toast host + error dialog */ }
```
```rust
// bridge.rs
pub struct Bridge { launcher: Arc<Launcher>, weak: slint::Weak<AppWindow> }
impl Bridge {
    pub fn run<T: Send + 'static>(&self, label: &'static str, job: impl FnOnce(&Launcher) -> Result<T, gcl_core::Error> + Send + 'static, done: impl FnOnce(&AppWindow, T) + Send + 'static);
    // spawns a std::thread; on Ok → upgrade_in_event_loop(done); on Err → upgrade_in_event_loop(show_error(label, err))
}
// events.rs
pub fn start_forwarder(rx: UnboundedReceiver<Event>, weak: Weak<AppWindow>) -> JoinHandle<()>;
// batches events every 50 ms; applies to App.tasks (VecModel<TaskRow>, update by id; done rows removed after 5 s via Timer on UI thread) and App.app_log (ring of 2000); Warning → toast
pub fn apply(tasks: &mut Vec<TaskRow>, log: &mut VecDeque<LogLine>, events: &[Event]) -> Vec<String /* warnings */>;  // pure, unit-tested
// models/mod.rs
pub fn instance_row(i: &Instance, installed: bool, running: bool) -> InstanceRow; pub fn content_row(e: &ContentEntry, update: bool) -> ContentRow; pub fn search_row(h: &SearchHit) -> SearchRow; pub fn account_row(a: &Account) -> AccountRow; pub fn setting_rows(map: &BTreeMap<String,String>) -> Vec<SettingRow>; pub fn format_bytes(u64) -> String; pub fn format_downloads(u64) -> String
```
`main.rs`: parse `--smoke`; `Launcher::new(None)` → on error show an error window and exit 1; `AppWindow::new`; `Bridge`; `start_forwarder`; wire `App.navigate` etc.; `--smoke` → `Timer::single_shot(500 ms, quit_event_loop)`; `run()`.
`build.rs`: `compile_with_config("ui/app.slint", CompilerConfiguration::new().with_style("fluent").embed_resources(EmbedResourcesKind::EmbedFiles))`.

- [ ] Theme tokens extended (surface levels, text sizes: body 14, small 12, title 20, h1 24; row height 34px; rail width 200px; accent, danger, success, warning; focus ring).
- [ ] Components with default previews; the shell renders with placeholder screens (each screen file created as an empty `export component XScreen inherits Rectangle {}` for now).
- [ ] Unit tests: `events::apply` (start → progress → finish; failed; warning collection; ring bound); `models` formatters (`format_bytes(1536) == "1.5 KiB"`, downloads `12345 → "12.3K"`); `bridge` with a fake job (needs a Slint instance? test `Bridge::run` by extracting the thread + channel logic into `bridge::spawn_job` that returns a `std::sync::mpsc::Receiver<Result<T>>`; test that).
- [ ] `just check`; `just run-ui -- --smoke` exits 0 (report the output; if no display is available, say so and rely on `cargo build`); `just ui-preview app.slint` renders (describe). Commit `feat(ui): shell, theme, bridge, event forwarder`.

---

### Task 3: Instances screen

- [ ] `ui/screens/instances.slint`: `InstancesScreen { in property <[InstanceRow]> rows; in property <string> filter; callback open(string); callback create(); callback delete(string); callback launch(string); callback refresh(); }` — list with name, MC version, loader, last launched, a Launch button per row (disabled when running), a Create button, an empty state.
- [ ] `ui/components/create-instance-dialog.slint`: name, MC version ComboBox (`[VersionRow]`, releases only unless "show snapshots"), loader ComboBox (none/fabric/quilt/forge/neoforge), loader version ComboBox (`[LoaderVersionRow]` loaded when MC+loader chosen; "recommended" preselected), Create/Cancel; `callback load_loader_versions(string mc, string loader)`.
- [ ] `ui/components/confirm-dialog.slint` (title, text, Confirm/Cancel).
- [ ] `src/screens/instances.rs`: load rows (`launcher.instances().list()` + installed check via `instance_summary`), create (`instances().create` then `install_loader` in the background with progress), delete (confirm → `delete`), launch (→ Task 4's launch path; here just navigate to the detail screen and trigger launch).
- [ ] Preview with 3 default rows; `--smoke` still passes. Commit `feat(ui): instances screen`.

---

### Task 4: Instance detail screen

- [ ] `ui/screens/instance.slint`: header (name, MC, loader, Launch/Stop button, status text), `TabBar` with Content / Settings / JVM / Logs.
  - Content tab: `[ContentRow]` list with Enable/Disable, Remove, "Check updates", "Apply updates", "Add content" (navigates to browser with `current_slug`), pending manual downloads section (`[PendingRow]` with Open page + "I downloaded it" → file picker: for the MVP a `LineEdit` path field + Import button).
  - Settings tab: `[SettingRow]` overrides editor (key, value, Set, Unset) plus "Current options.txt" read-only list.
  - JVM tab: min/max MiB `SpinBox`, extra args `LineEdit`, Java path `LineEdit`, Save.
  - Logs tab: `[LogLine]` for the running/last game, auto-scroll toggle, "Open log folder" (prints the path; no shell open in MVP).
- [ ] `src/screens/instance.rs`: `instance_summary`; content ops via `Launcher` (`list_content`, `set_content_enabled`, `remove_content`, `check_updates`, `apply_updates`, `pending_manual`, `import_manual_file`); settings via `settings::read` + `update` through a new `Launcher::set_instance_override(slug, key, value)` / `unset_instance_override` / `set_instance_jvm(slug, InstanceJvm)` (add to core in this task; thin wrappers that validate and save); launch via `launch_instance_async` with the active account (or an offline-user prompt dialog when none), status updates from `RunningLaunch` join, game log lines from `Event::Log` while running.
- [ ] Commit `feat(ui): instance detail with content, settings, jvm, logs`.

---

### Task 5: Content browser

- [ ] `ui/screens/browser.slint`: search box, source ComboBox (modrinth, curseforge when enabled), type ComboBox (mod, resourcepack, shader, datapack, world, modpack), MC version and loader filters (prefilled from `current_slug`), results `[SearchRow]` (title, author, downloads, description), per-row "Add to <instance>" (or "Install modpack" for the modpack type → name dialog), pager (Prev/Next, page size 20), a "target instance" ComboBox when no `current_slug`.
- [ ] `src/screens/browser.rs`: `launcher.search(source, query)`; add via `add_content` with progress; modpack via `import_modpack`; manual downloads surface as a toast + the detail screen's pending list; datapack asks for a world (ComboBox from `saves/`: add `Launcher::list_worlds(slug)` in core).
- [ ] Commit `feat(ui): content browser and modpack install`.

---

### Task 6: Accounts and settings screens

- [ ] `ui/screens/accounts.slint`: `[AccountRow]` with Select/Remove/Refresh(msa), "Add offline" (name dialog), "Sign in with Microsoft" (disabled with a hint when unavailable); `ui/components/device-code-dialog.slint`: shows the code and URL, "Copy code" (Slint has no clipboard API in 1.17 core? UNCERTAIN — if absent, show the code large and selectable via `TextInput read-only`), Cancel (cancels the child token), status line from login events.
- [ ] `ui/screens/settings.slint`: root path (read-only + "Change" → text field + Apply; note that data is not moved), parallel downloads, JVM defaults, keys (`<set>`/`<unset>` + change field), game defaults editor (`[SettingRow]`), "Verify sources" button that runs the debug checks and shows PASS/FAIL lines.
- [ ] `src/screens/{accounts,settings}.rs`: `accounts()` store ops, `msa_login_with_cancel` on a worker with the device-code callback posting to the dialog; `update_config` for settings.
- [ ] Commit `feat(ui): accounts and settings screens`.

---

### Task 7: Progress panel, toasts, errors, keyboard

- [ ] `ui/components/progress-panel.slint`: collapsible bottom panel listing `[TaskRow]` with a bar each; header shows "N tasks" and the busiest label; `ui/components/toast.slint`: stack of warnings that fade after 6 s; error dialog (title, text, Copy/Close).
- [ ] Keyboard: `FocusScope` on the rail (1-5 switch screens), lists (Up/Down/Enter), dialogs (Escape).
- [ ] Wire `events.rs` outputs to these; `--smoke` exercises a synthetic task event.
- [ ] Commit `feat(ui): progress panel, toasts, errors, keyboard navigation`.

---

### Task 8: Docs and packaging stub

- [ ] Update `ARCHITECTURE.md` (gcl-ui structure, bridge/forwarder threading, the `Launcher` changes), `.claude/skills/slint-ui/SKILL.md` (real file layout, patterns as built, preview commands per screen, threading rules with the `Bridge` API), `.claude/skills/testing/SKILL.md` (`--smoke`, what is unit-tested in gcl-ui), `docs/SPEC.md` R13 notes, README ("Run the app": `just run-ui`; screenshots later), add `packaging/grid-craft-launcher.desktop` and a placeholder `packaging/icon.png` (generate a simple 256x256 PNG with python3 or `magick` if present; else a documented placeholder).
- [ ] `just check && just deny && just lint-claude`; `just run-ui -- --smoke`. Commit `docs: gui`.
