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
let client = HttpClient::new();
let source = ModrinthSource::with_base_url(client.clone(), server.uri());
```

`HttpClient` is host-agnostic. Every source client (`ModrinthSource`, `CurseForgeSource`) and the
`mojang` module take a base URL so tests can point them at wiremock.

`Launcher::mojang()` reads `GCL_MOJANG_BASE_URL` (`launcher::MOJANG_BASE_URL_ENV`) and uses it
instead of `piston-meta`. That env var is test-only: set it in CLI tests to point `gcl` at a
wiremock server. Never set it in production or document it as a user setting.

`Launcher::loader_endpoints()` reads five more test-only overrides the same way, one per loader
host: `GCL_FABRIC_BASE_URL`, `GCL_QUILT_BASE_URL`, `GCL_FORGE_META_BASE_URL` (the metadata host,
not the maven host: see the `modloaders` skill), `GCL_FORGE_MAVEN_BASE_URL`, and
`GCL_NEOFORGE_BASE_URL`. Two more cover the content sources: `GCL_MODRINTH_BASE_URL` and
`GCL_CURSEFORGE_BASE_URL` (`launcher::MODRINTH_BASE_URL_ENV` / `CURSEFORGE_BASE_URL_ENV`), read
by `Endpoints::from_env()` and used to build `Modrinth::with_base_url` /
`CurseForge::with_base_url` in `Launcher::sources()`. `Endpoints::from_env()` collects all eight
plus the Mojang one; set whichever ones a test's mock server needs to answer for.
`Launcher::open_with_endpoints(root, endpoints)` is the lower-level seam: it takes a whole
`Endpoints` value and reads no environment at all, for a test that wants to build several
launchers with different hosts side by side.

**Every `GCL_*_BASE_URL` override is read in a debug build only.** `Endpoints::from_env()` is
`#[cfg(debug_assertions)]`; a release build has a second version that returns
`Endpoints::default()` and reads no environment, so a shipped launcher cannot be pointed at
another host. Tests, `cargo run`, `just e2e`, and `just e2e-modpack` all run debug builds, so
nothing that uses the overrides changes. A release binary under test needs
`Launcher::open_with_endpoints` instead.

### MSA wiremock pattern

The six-step Microsoft login chain (see the `msa-auth` skill) runs against one `MockServer`,
one mock path per step:

```rust
let server = MockServer::start().await;
let ep = MsaEndpoints {
    device_code: format!("{}/devicecode", server.uri()),
    token: format!("{}/token", server.uri()),
    xbl: format!("{}/xbl", server.uri()),
    xsts: format!("{}/xsts", server.uri()),
    mc_login: format!("{}/mclogin", server.uri()),
    profile: format!("{}/profile", server.uri()),
};
let msa = Msa::new(HttpClient::new()?.with_backoff(vec![Duration::ZERO; 3]), ep, client_id);
```

`crates/gcl-core/src/auth/msa_tests.rs` and `session_tests.rs` build `MsaEndpoints` and `Msa`
this way; `Endpoints.msa` is the same shape one layer up, for a test that goes through
`Launcher::open_with_endpoints` instead of building `Msa` directly. A poll sequence
(`authorization_pending` then success, or `slow_down` then success) is served with a `Respond`
impl that answers a different body each call — see `msa_tests.rs`'s `Sequence` type.

Use `MemoryStore` (`auth::secrets::MemoryStore::new()`) as the secret store in any test that
signs in or refreshes: no real keyring, nothing left behind after the run. A `Launcher`-level
test wires it in with `Launcher::open_with_endpoints(root, endpoints).with_secret_store(Box::new(MemoryStore::new()))`,
called before anything touches `Launcher::secrets()` — the store is a `OnceCell`, so the first
call wins. A test that exercises the real `open_default` fallback path instead (choosing
between the keyring and `FileStore`) sets `GCL_NO_KEYRING=1` first, so it forces the file store
and never touches a developer's real keyring; unset it afterward or scope it to the process
running under `GCL_ROOT`. `secrets_tests.rs` covers `FileStore`'s 0600 permission and
`KeyringStore::probe` directly.

### Recording-sleep pattern

`session::login_device_code` takes a `sleep: &SleepFn` so a test can drive the poll loop with
no real waiting. Build one that appends every requested `Duration` to a shared `Vec` and
resolves immediately:

```rust
fn recording_sleep() -> (Arc<Mutex<Vec<Duration>>>, impl Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync) {
    let waits = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&waits);
    let sleep = move |d: Duration| {
        recorder.lock().expect("lock").push(d);
        Box::pin(std::future::ready(())) as BoxFuture<'static, ()>
    };
    (waits, sleep)
}
```

Assert on `waits.lock().unwrap()` afterward: how many polls ran, and whether a `slow_down`
answer added the extra five seconds to the interval. See `session_tests.rs` for the full
pattern, including a test that runs the code past its `expires_in_secs` and asserts
`Error::DeviceCodeExpired`.

### FakeSource

`content` and `modpacks` unit tests do not need wiremock for a content source: a `FakeSource`
implementing `crate::sources::Source` in the test module stands in for Modrinth or CurseForge.
See `crates/gcl-core/src/content/tests.rs` for the pattern:

```rust
struct FakeSource {
    id: SourceId,
    projects: Vec<Project>,
    versions: Vec<Version>,
}

impl FakeSource {
    fn new(id: SourceId) -> Self { /* empty */ }
    fn with(mut self, project: Project, versions: Vec<Version>) -> Self { /* push */ self }
    fn boxed(self) -> BoxSource { Arc::new(self) }
}

#[async_trait]
impl Source for FakeSource {
    fn id(&self) -> SourceId { self.id }
    fn supported_kinds(&self) -> &[ContentKind] { &[/* every kind */] }
    async fn project(&self, id_or_slug: &str) -> Result<Project, crate::sources::Error> {
        self.projects.iter().find(|p| p.id == id_or_slug || p.slug == id_or_slug)
            .cloned().ok_or_else(|| crate::sources::Error::NotFound { source_id: self.id, id: id_or_slug.to_string() })
    }
    // versions/version/search/resolve_by_hash/resolve_by_fingerprint follow the same shape
}
```

Build a `ContentCtx` with `sources: &[fake.boxed()]` and call `content::add` or
`modpacks::import_plan` directly — no HTTP mocking, no wiremock server, and it runs the real
`pick_version` and dependency-walk logic against data the test controls.

A CurseForge-shaped test that needs `as_curseforge()` to return `Some` (a modpack import test,
for instance) still needs a real `CurseForge` client, since `as_curseforge` returns `None` by
default and `FakeSource` does not override it; point that client at a wiremock server with
`CurseForge::with_base_url` instead.

### FakeRunner

Forge and NeoForge run installer processors as child JVMs through the `ProcessRunner` trait
(`crate::loaders::ProcessRunner`, in `gcl-core/src/loaders/processors.rs`). Production code uses
`JavaRunner`, which really spawns `java`. A test passes a fake instead, through
`LoaderCtx::runner: Option<&dyn ProcessRunner>`:

```rust
struct FakeRunner {
    calls: Mutex<Vec<Call>>,
    writes: BTreeMap<String, Vec<(PathBuf, Vec<u8>)>>, // main class -> files it writes
    code: i32,
}

#[async_trait::async_trait]
impl ProcessRunner for FakeRunner {
    async fn run(&self, _java: &Path, classpath: &[PathBuf], main: &str, args: &[String], log: &Path)
        -> Result<i32, std::io::Error> {
        self.calls.lock().unwrap().push(/* record the call */);
        std::fs::write(log, format!("ran {main}\n"))?;
        if self.code == 0 {
            for (path, bytes) in self.writes.get(main).into_iter().flatten() {
                std::fs::write(path, bytes)?;
            }
        }
        Ok(self.code)
    }
}
```

It records every call for assertions and writes the files a real processor's `outputs` promise,
so `outputs_current` sees a correct install afterward. See
`crates/gcl-core/src/loaders/processors_tests.rs` for the full fixture, including how it drives a
non-zero exit code and a missing output.

A test that goes through `Launcher` instead of `loaders::install` injects the same fake with
`Launcher::open_with_endpoints(root, endpoints).with_process_runner(Arc::new(FakeRunner::default()))`.
`install_loader` hands that runner to `LoaderCtx`; with none set, Forge and NeoForge spawn a real
`java` through `JavaRunner`. Set the instance's `jvm.java_path` as well: a configured path is used
as given, so the install never probes or downloads a runtime.

### tests/common/mod.rs

`crates/gcl-core/tests/common/mod.rs` and `crates/gcl-cli/tests/common/mod.rs` each hold the same
small helper set, compiled once per test binary (`#![allow(dead_code)]`, since not every test
file in a binary uses every helper):

- `serve(server, path, body)`: mounts a GET mock returning `body` at `path`.
- `zip_bytes(entries)` / `jar_with_main(main)`: build an in-memory zip, or a jar whose manifest
  declares `Main-Class: <main>`.
- `mock_vanilla(server, id)`: serves a complete, library-free vanilla version (manifest, version
  JSON, client jar, empty asset index, and a `logging` block with its log4j2 file) under one mock
  server, for tests that only need vanilla install to succeed before checking something else.
- `mock_forge(server)` plus `forge_installer_jar`, `FakeRunner`, `fake_java`, and the
  `FORGE_*`/`PATCHED_*`/`UNIVERSAL_*` constants (gcl-core only): a synthetic Forge 47.4.10
  installer for 1.20.1, its libraries, and the runner that stands in for the processor JVM.
  `forge_install.rs` and `launcher_flow.rs` share them.

### Fixture URL rewriting

A fixture JSON still has real Mojang hosts baked into its URLs (`piston-meta.mojang.com`,
`libraries.minecraft.net`, `resources.download.minecraft.net`, `launchermeta.mojang.com`).
Before serving a fixture from wiremock, string-replace the host with the mock server's URI so
the code under test fetches from wiremock for every follow-up request, not just the first:

```rust
let server = MockServer::start().await;
let body = MANIFEST.replace("https://piston-meta.mojang.com", &server.uri());
Mock::given(method("GET")).and(path("/mc/game/version_manifest_v2.json"))
    .respond_with(ResponseTemplate::new(200).set_body_string(body))
    .mount(&server).await;
```

See `crates/gcl-cli/tests/cli.rs::mock_mojang` and `crates/gcl-core/src/java/runtime.rs` tests
for worked examples. Rewrite every host the fixture references, not only the one the current
test happens to hit.

### Flaky timing test

`download_all_keeps_only_two_files_in_flight` (in `crates/gcl-core/src/download/mod.rs`)
asserts on wall-clock timing of concurrent transfers. On a loaded machine it can fail spuriously.
Rerun it once before treating a failure as a real regression.

## Filesystem

- Use `tempfile::tempdir()` for a root. Set `GCL_ROOT` when testing the CLI.
- Never touch the real user root in tests.

## CLI tests

`assert_cmd::Command::cargo_bin("gcl")` with `.env("GCL_ROOT", dir.path())`. Assert exit code and
key output lines. Use `--json` and parse with serde_json for structure.

## e2e

`just e2e` runs `scripts/e2e.sh`: temp root, create instance, install the loader, add a mod
from Modrinth (`gcl content add`, a real network call), dry-run launch, check every classpath
jar exists. It uses the network. Only the e2e-runner agent and humans run it.

- `GCL_E2E_LOADER`: which loader to install (`fabric`, `quilt`, `forge`, `neoforge`). Defaults to
  `fabric`.
- `GCL_E2E_MC_VERSION`: which Minecraft version to use. Defaults to `1.20.1`, except when
  `GCL_E2E_LOADER=neoforge`, where the default is `1.20.2` — NeoForge has no build for 1.20.1
  (see the `modloaders` skill). Set this to override either default.
- `GCL_E2E_MOD`: which Modrinth project to add. Defaults to `sodium` for `fabric`/`quilt`, `jei`
  for `forge`, and `jade` for `neoforge` — Sodium publishes for Fabric and Quilt only, and JEI
  has no NeoForge build for 1.20.2. Set this to override any of these.

`just e2e-modpack` runs `scripts/e2e-modpack.sh`: temp root, `gcl modpack install --source
modrinth --project <pack>` (`GCL_E2E_PACK`, default `fabulously-optimized`), list content,
dry-run launch, check the classpath. Both e2e scripts source `scripts/lib/classpath-check.sh`'s
`check_classpath <launch-output-file>`, which reads the `-cp` line out of a dry-run launch and
fails if any jar on it does not exist on disk or is named twice (BootstrapLauncher rejects a
repeated jar) — `crates/gcl-cli/tests/cli.rs` uses the same
function for its own dry-run assertions.

`modpacks::ImportRequest::extra_hosts` is a test seam, not something either e2e script sets: it
adds hosts to the `.mrpack` download allowlist (`mrpack::ALLOWED_HOSTS`) for one import, so a
unit or CLI test can serve a fake `.mrpack` from its own wiremock server without that host being
one a real pack could ever use. Leave it empty in anything that talks to a real source.

## gcl-ui

- Two layers. Pure helpers are unit tested next to the code; whole flows are driven through the
  Slint testing backend, which needs no display. Prefer a helper test; reach for a flow test when
  the question is "does this button do anything".
- Every module splits its logic into a pure helper and tests that directly:
  `events::apply` (folds a batch of core `Event`s into the task list and log, no window),
  `events::prune_finished` and `toasts::prune`/`push` (age rows and toasts out by a given
  `Instant`), `keys::key_to_screen` and `keys::move_selection` (keyboard rules), `state::RunState`
  (the running-slugs set), and the converters in `src/models/` (`gcl-core` struct to Slint
  struct). Each screen module (`src/screens/*.rs`) keeps its own `tests.rs` the same way.
  `crates/gcl-ui/src/bridge/tests.rs` covers `Bridge`'s threading with `bridge::spawn_job`, the
  windowless half of `Bridge::run`.
- `just run-ui -- --smoke`: opens the real window, sends three synthetic events through the
  event sink, waits 500 ms, then quits. It is the one thing that needs a display — run it by
  hand or under `xvfb-run` locally, not in CI.
- `just ui-preview screens/x.slint` is a manual visual check (slint-viewer, live reload), not an
  automated test. Use it after changing a screen; there is no snapshot to assert on.
- Add a test for new pure logic before wiring it into a callback. If a change can only be tested
  by clicking through the built app, it usually means logic leaked into a `.slint` file or an
  `app.rs` closure that should have stayed in a testable helper.

### GUI flow tests

`crates/gcl-ui/tests/flow_instances.rs` builds the real `AppWindow` over a real `Launcher` and
clicks through it. `tests/support/mod.rs` is the harness:

- `TestApp::new()`: a temp root, a wiremock host serving a synthetic vanilla version and the
  Fabric fixtures, a stand-in `java` shell script at `<root>/fake-java` that records its argument
  list and waits for a signal, `Launcher::open_with_endpoints(...).with_secret_store(MemoryStore)`
  with `config.jvm.java_path` pointing at that script, and `gcl_ui::app::build`.
- `app.click(id)`, `app.type_into(id, text)`, `app.select_combo(id, index)`, `app.el(id)`. Ids are
  `<Component>::<name>` from `docs/research/2026-09-07-ui-element-ids.md`; either `_` or `-`
  works. `click` fails when the control is disabled, which is the whole point.
- `click` and `click_nth` send real pointer events — a move, a press, and a release at the
  element's middle — so the press goes through hit-testing the way a mouse does. That is what
  catches a control under a modal overlay or behind a stale pointer grab. `activate(id)` and
  `activate_nth(id, n)` press through the accessible action instead, for the few controls a
  pointer cannot land on, such as a row inside a `ComboBox` popup.
- `app.wait_until(what, pred, timeout)` / `wait_for(id)` yield to the event loop, so the bridge's
  worker threads post their results back exactly as they do in the app.
- `support::run(flow)` starts the event loop, runs the flow, quits, and re-raises any panic.

Run them with `cargo nextest run -p gcl-ui`, or one at a time with `just test
'binary(flow_settings)'`. `just check` runs them with everything else.

Rules:

- **One Slint backend per process.** `support::init_backend()` calls
  `i_slint_backend_testing::init_integration_test_with_system_time` behind a `Once`, and no
  other backend may be set in that process. So each flow group is its own test binary with one
  `#[test]` inside it, running its sub-flows in order over one window. Four binaries today:
  `flow_instances`, `flow_settings`, `flow_content`, `flow_accounts`. Adding a second `#[test]`
  to one of them is the failure mode to watch for.
- **System time, not mock time.** The mock-time backend deadlocked the timer yield
  `wait_until` runs on. `support::pump()` therefore calls `mock_elapsed_time(Duration::ZERO)`,
  which drives one loop turn and hands the loop no time at all. A control that only saves after
  a debounce interval cannot be driven from a flow test; drag the slider (which saves on
  release) and unit-test the debounced path instead.
- **The stand-in Java is a `sh` script.** `TestApp` writes one at `<root>/fake-java` and points
  `config.jvm.java_path` at it. It records its whole argument list to a file the test reads back
  (`app.java_args()`), then waits for `<root>/stop` so a flow can press Stop while it is still
  up (`app.ask_java_to_stop()`). Nothing downloads a real JVM and nothing starts Minecraft.
- **Every endpoint is mocked or unreachable.** The harness mounts wiremock for Mojang, Fabric,
  Modrinth and the six Microsoft steps, and points every endpoint it does not mock at a
  `.invalid` host, so a request the test did not plan for fails at DNS rather than reaching the
  real service. The secret store is `MemoryStore`, so no keyring is touched.
- **Element names must be in the generated code.** `crates/gcl-ui/build.rs` emits them when
  `PROFILE` is `debug` — the test profile's `PROFILE` is `debug`, so nextest gets them — or with
  `SLINT_EMIT_DEBUG_INFO=1`. `TestApp::new` asserts the id list is not empty, so a build that
  lost them fails loudly instead of reporting "element not found" for everything.
- **A `ComboBox` has no accessible set-value action.** `select_combo` opens the popup with
  `accessible-action-expand`, presses Up until `accessible_value` stops changing, then presses
  Down `index` times.
- **A flow that waits forever must not hang the run.** `.config/nextest.toml` gives every
  `flow_*` binary a slow timeout of five 60-second periods, after which nextest terminates the
  binary and reports the test as timed out.
- **A clipped element cannot be clicked.** A `ScrollView` draws only its viewport, but the
  element tree still reports a row half past the bottom edge, with a position and a size. A
  pointer aimed at its middle then lands on nothing. `scroll_to(id)` therefore scrolls until
  the element sits completely inside every `ScrollView::flickable` it overlaps, and `click`
  fails with both rectangles when it is asked to press something outside one — the whole
  rectangle, not only its middle, because a row drawn cut in half is not a row a user clicks.
  `scroll_to` scrolls *toward* the element: up while it is above the viewport, down while it
  is below, so a row already past the top edge is reached rather than walked away from. It
  checks after its last scroll as well as before each one, so the step that brings the
  element in is never the one that reports failure.
- No flow test opens a display, reaches the network, or touches the keyring.

### Real X input: `just ui-xtest`

The flow tests drive the window through the Slint testing backend, which has no X server, no
window manager, and no pointer grabs. Three defects lived in exactly that gap: a dialog whose
overlay ate the click after an Escape, shortcuts that died with the screen that held the focus,
and a button that only looked pressable. `just ui-xtest` is the check that sees them.

It needs the network, `Xvfb`, `xdpyinfo`, ImageMagick's `import`, and python3-xlib. It builds
`gcl-ui`, starts `Xvfb -displayfd` at 1200x760 — Xvfb picks a free display and names it, so no
fixed number and no stale lock file can break the run — runs the debug GUI over a throwaway
root with `GCL_LOG=info`, drives it with `scripts/ui-xtest.py`, and prints PASS or FAIL by
reading the GUI log for the jobs the run must have raised. The temp root is removed on PASS and
kept on failure, with its path printed.

Every wait is a poll with a bounded timeout, not a fixed sleep: `xdpyinfo -display` for the X
server, the `gui start` line in the GUI log for the app, and `instances/smoke` on disk plus the
`Install loader` job for the install. Each python drive runs under `timeout 180`. The exit trap
kills *and* waits for the Xvfb pid: an Xvfb that is still running holds its lock.

`scripts/ui-xtest.py` sends every event through the XTest extension, so the app sees them as a
user's. Steps, applied in order: `focus[:wm-class]`, `click:X,Y`, `drag:X1,Y1,X2,Y2`,
`type:TEXT`, `key:NAME`, `sleep:SECONDS`, `shot:PATH`. The display comes from
`GCL_XTEST_DISPLAY`, else `DISPLAY`, and `main` opens it, so a server that is not there is
named rather than raising a traceback at import.

- **A key the layout only reaches with Shift is typed with Shift.** `key` reads the keycode's
  level 0 and level 1 keysyms to decide, so `type:My_Pack` types what it says; uppercase
  letters and `_ - . : /` all work. A keysym name the layout does not carry fails loudly
  instead of pressing keycode 0.

- **`focus` first, always.** A bare Xvfb runs no window manager, so nothing hands out the input
  focus and every key goes to the root window. The step finds the window by its WM class
  (`grid-craft-launcher`) and calls `set_input_focus` itself. Without it no `type` or `key`
  step reaches the app.
- Coordinates are literal pixels at 1200x760 with the fluent style. Take a `shot` and read it
  before trusting a coordinate: a dialog grows a row when a loader is chosen, and everything
  below it moves.
- Never `pkill -f` a pattern that also matches your own shell. Kill by pid.

## What not to do

- No network in unit or CLI tests.
- No `sleep` to wait for async work; await the future.
- No tests that depend on ordering or shared global state.
- No gcl-ui test that opens a display, apart from `just ui-xtest`, which is run by hand and is
  not part of `just check`. A flow test builds a real `AppWindow` on the Slint testing
  backend, which draws nothing; keep everything else in a pure helper and test that instead.
