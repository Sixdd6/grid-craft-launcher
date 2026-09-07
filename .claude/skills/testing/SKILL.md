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

`just e2e` runs `scripts/e2e.sh`: temp root, create instance, install the loader, add Sodium
from Modrinth (`gcl content add`, a real network call), dry-run launch, check every classpath
jar exists. It uses the network. Only the e2e-runner agent and humans run it.

- `GCL_E2E_LOADER`: which loader to install (`fabric`, `quilt`, `forge`, `neoforge`). Defaults to
  `fabric`.
- `GCL_E2E_MC_VERSION`: which Minecraft version to use. Defaults to `1.20.1`, except when
  `GCL_E2E_LOADER=neoforge`, where the default is `1.20.2` — NeoForge has no build for 1.20.1
  (see the `modloaders` skill). Set this to override either default.

`just e2e-modpack` runs `scripts/e2e-modpack.sh`: temp root, `gcl modpack install --source
modrinth --project <pack>` (`GCL_E2E_PACK`, default `fabulously-optimized`), list content,
dry-run launch, check the classpath. Both e2e scripts source `scripts/lib/classpath-check.sh`'s
`check_classpath <launch-output-file>`, which reads the `-cp` line out of a dry-run launch and
fails if any jar on it does not exist on disk — `crates/gcl-cli/tests/cli.rs` uses the same
function for its own dry-run assertions.

`modpacks::ImportRequest::extra_hosts` is a test seam, not something either e2e script sets: it
adds hosts to the `.mrpack` download allowlist (`mrpack::ALLOWED_HOSTS`) for one import, so a
unit or CLI test can serve a fake `.mrpack` from its own wiremock server without that host being
one a real pack could ever use. Leave it empty in anything that talks to a real source.

## gcl-ui

- No display in CI, so nothing here builds a real `AppWindow` in a test. Instead, every module
  that would otherwise need one splits its logic into a pure helper and tests that directly:
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

## What not to do

- No network in unit or CLI tests.
- No `sleep` to wait for async work; await the future.
- No tests that depend on ordering or shared global state.
- No gcl-ui test that builds a real `AppWindow` or opens a display; keep logic in a pure helper
  and test that instead.
