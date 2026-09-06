# MVP Plan 1: Core Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `gcl-core` modules every later plan depends on (events, paths, config, http, download cache, Mojang metadata, Java runtimes, the `Launcher` handle, instances) and expose them through `gcl` so a vanilla Minecraft version can be fully installed into the cache and an instance can be created and listed.

**Architecture:** Library-first. Each module is a directory under `crates/gcl-core/src/` with `mod.rs` documenting its interface, a `thiserror` error enum, serde structs for remote JSON, and unit tests against fixtures in `tests/fixtures/`. `Launcher` owns the tokio runtime, config, `HttpClient`, and the event channel. `gcl-cli` calls `Launcher` methods and prints events.

**Tech Stack:** Rust 1.97, tokio 1.53, reqwest 0.13 (rustls), serde/serde_json/toml, sha1, tempfile, wiremock 0.6, insta 1.48, assert_cmd, clap 4.6.

**Spec:** `docs/SPEC.md` (R1, R2.1, R2.2, R2.4, R3, R5, R12.1 partial, R12.2), `ARCHITECTURE.md`, skills `rust-conventions`, `testing`, `mojang-meta`, `instance-model`, `download-cache`.

## Global Constraints

- `thiserror` enums in `gcl-core`, `anyhow` in binaries. No `unwrap`/`expect`/`panic!` outside tests and `build.rs`.
- One tokio runtime, owned by `Launcher`. Blocking work (hashing whole files, zip, copies) under `spawn_blocking`.
- `HttpClient` is host-agnostic. Modules that fetch take a base URL so tests can point them at wiremock. Production base URLs are constants in the owning module.
- No network in unit or CLI tests. Fixtures under `tests/fixtures/<source>/<name>.json`.
- Every download verifies sha1 when known, else size when known. Three attempts. `.part` files, rename on success.
- App root: `GCL_ROOT` env → `config.toml` `root` → `directories::ProjectDirs::from("", "", "grid-craft-launcher").data_dir()`.
- Layout under root exactly as `ARCHITECTURE.md` "App root layout".
- `options.txt` uses `key:value` lines (not `=`).
- Secrets `CURSEFORGE_API_KEY`, `GCL_MSA_CLIENT_ID`: env first, then `config.toml`; never logged.
- Public items get a one-line doc comment. `just check` passes after every task. Commit after every task with the `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` trailer.
- Add dependencies to `[workspace.dependencies]` first; crates use `dep.workspace = true`.
- CLI output: plain text by default, JSON with `--json` (global flag).

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `tests/fixtures/mojang/*.json`, `tests/fixtures/README.md` | recorded Mojang responses | 1 |
| `crates/gcl-core/src/error.rs`, `lib.rs` | crate error enum, module declarations | 2 |
| `crates/gcl-core/src/events/mod.rs` | `Event`, `EventSink`, `TaskId` | 2 |
| `crates/gcl-core/src/paths/mod.rs` | `Root`, subdir helpers, `slugify` | 3 |
| `crates/gcl-core/src/config/mod.rs` | `Config`, load/save, env overrides | 4 |
| `crates/gcl-core/src/http/mod.rs` | `HttpClient` | 5 |
| `crates/gcl-core/src/download/mod.rs`, `download/hash.rs` | `DownloadSpec`, object cache, queue | 6 |
| `crates/gcl-core/src/mojang/{mod,manifest,version,rules,args,assets,install}.rs` | Mojang metadata and vanilla install | 7, 8, 9 |
| `crates/gcl-core/src/java/{mod,detect,runtime}.rs` | Java detection and Mojang runtimes | 10 |
| `crates/gcl-core/src/instances/mod.rs`, `instances/model.rs` | instance layout and `instance.toml` | 11 |
| `crates/gcl-core/src/launcher/mod.rs` | `Launcher` handle | 12 |
| `crates/gcl-cli/src/{main,output,commands/*}.rs` | CLI commands | 13 |

---

### Task 1: Record Mojang fixtures

**Files:**
- Create: `tests/fixtures/mojang/version_manifest_v2.json`, `tests/fixtures/mojang/1.20.1.json`, `tests/fixtures/mojang/1.8.9.json`, `tests/fixtures/mojang/asset_index_1.20.json`, `tests/fixtures/mojang/java_runtime_all.json`

- [ ] **Step 1: Record the manifest and trim it**

```bash
just record-fixture mojang version_manifest_v2 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
```
Then trim `versions` to the entries for `1.21.1`, `1.20.1`, `1.8.9`, and the first snapshot, with `python3`:
```bash
python3 - <<'EOF'
import json
p='tests/fixtures/mojang/version_manifest_v2.json'; d=json.load(open(p))
keep={'1.21.1','1.20.1','1.8.9'}
snap=next(v for v in d['versions'] if v['type']=='snapshot')
d['versions']=[v for v in d['versions'] if v['id'] in keep]+[snap]
json.dump(d,open(p,'w'),indent=2)
EOF
```

- [ ] **Step 2: Record two version JSONs and one asset index**

Get the URLs from the trimmed manifest (`url` field of `1.20.1` and `1.8.9`), then:
```bash
just record-fixture mojang 1.20.1 '<url of 1.20.1>'
just record-fixture mojang 1.8.9 '<url of 1.8.9>'
just record-fixture mojang asset_index_1.20 "$(python3 -c "import json;print(json.load(open('tests/fixtures/mojang/1.20.1.json'))['assetIndex']['url'])")"
```
Trim the asset index `objects` to its first 20 entries with python3 (same pattern as Step 1).

- [ ] **Step 3: Record the Java runtime manifest**

```bash
just record-fixture mojang java_runtime_all 'https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json'
```
Trim to platform keys `linux`, `windows-x64`, `mac-os`, `mac-os-arm64`, and within each keep only `java-runtime-gamma` and `java-runtime-delta`.

- [ ] **Step 4: Check sizes and commit**

Run: `du -h tests/fixtures/mojang/*` — every file under 200 KB (1.20.1.json is about 60 KB untrimmed; keep it whole).
```bash
git add tests/fixtures && git commit -m "test: record Mojang fixtures

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Crate error type and events module

**Files:**
- Create: `crates/gcl-core/src/error.rs`, `crates/gcl-core/src/events/mod.rs`
- Modify: `crates/gcl-core/src/lib.rs`, `crates/gcl-core/Cargo.toml`, `Cargo.toml`

**Interfaces:**
- Produces:
  ```rust
  pub type TaskId = u64;
  pub enum Event {
      TaskStarted { id: TaskId, label: String, total_bytes: Option<u64> },
      TaskProgress { id: TaskId, done_bytes: u64, total_bytes: Option<u64> },
      TaskFinished { id: TaskId },
      TaskFailed { id: TaskId, error: String },
      Log { level: LogLevel, message: String },
      Warning(String),
  }
  pub enum LogLevel { Debug, Info, Warn, Error }
  pub type EventSink = tokio::sync::mpsc::UnboundedSender<Event>;
  pub fn null_sink() -> EventSink;   // drops everything; for tests
  pub fn next_task_id() -> TaskId;   // AtomicU64 counter
  pub struct TaskHandle { id: TaskId, sink: EventSink }  // start(sink,label,total) -> Self; progress(done); finish(); fail(err)
  ```
  `gcl_core::Error` in `error.rs`: `#[derive(Debug, thiserror::Error)] pub enum Error { Io(#[from] std::io::Error), Http(#[from] http::Error), Download(#[from] download::Error), Mojang(#[from] mojang::Error), Java(#[from] java::Error), Config(#[from] config::Error), Instance(#[from] instances::Error), Paths(#[from] paths::Error) }` — add variants as modules land (Tasks 3-11 each add theirs).

- [ ] **Step 1: Add workspace deps to gcl-core**

`crates/gcl-core/Cargo.toml` `[dependencies]`: `thiserror`, `tokio`, `serde`, `serde_json`, `toml`, `tracing`, `directories`. `[dev-dependencies]`: `tempfile`, `insta`, `wiremock`. All `.workspace = true`. Add `tokio-util = { version = "0.7", features = ["rt"] }` and `bytes = "1"` and `futures-util = "0.3"` to `[workspace.dependencies]` (later tasks use them).

- [ ] **Step 2: Write the test for TaskHandle**

In `events/mod.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn task_handle_emits_started_progress_finished() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let t = TaskHandle::start(&tx, "download foo", Some(10));
        t.progress(4);
        t.finish();
        let a = rx.recv().await.unwrap();
        assert!(matches!(a, Event::TaskStarted { label, total_bytes: Some(10), .. } if label == "download foo"));
        assert!(matches!(rx.recv().await.unwrap(), Event::TaskProgress { done_bytes: 4, .. }));
        assert!(matches!(rx.recv().await.unwrap(), Event::TaskFinished { .. }));
    }
    #[test]
    fn task_ids_increase() { assert!(next_task_id() < next_task_id()); }
}
```

- [ ] **Step 3: Run, see it fail, implement**

`cargo nextest run -p gcl-core` → compile error (module missing). Implement `events/mod.rs` per the interface. `TaskHandle::start` sends `TaskStarted`; `progress` sends `TaskProgress` with the handle's last known total; `finish`/`fail` consume `self`. Sends ignore a closed receiver (`let _ = self.sink.send(..)`). `null_sink()` creates a channel and drops the receiver.

- [ ] **Step 4: lib.rs**

```rust
pub mod error;
pub mod events;
pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;
```
plus the existing `VERSION` and `USER_AGENT`. `error.rs` starts with only `Io` and grows per task.

- [ ] **Step 5: `just check`, commit** `feat(core): events and error type`

---

### Task 3: paths module

**Files:**
- Create: `crates/gcl-core/src/paths/mod.rs`

**Interfaces:**
```rust
pub struct Root(PathBuf);
impl Root {
    pub fn resolve(config_override: Option<&Path>) -> Result<Root, Error>; // GCL_ROOT env > override > ProjectDirs data_dir
    pub fn from_path(p: impl Into<PathBuf>) -> Root;
    pub fn path(&self) -> &Path;
    pub fn config_file(&self) -> PathBuf;      // config.toml
    pub fn accounts_file(&self) -> PathBuf;    // accounts.json
    pub fn instances_dir(&self) -> PathBuf;
    pub fn instance_dir(&self, slug: &str) -> PathBuf;
    pub fn cache_dir(&self) -> PathBuf;
    pub fn versions_dir(&self) -> PathBuf;     // cache/versions
    pub fn libraries_dir(&self) -> PathBuf;    // cache/libraries
    pub fn assets_dir(&self) -> PathBuf;       // cache/assets
    pub fn natives_dir(&self, version_id: &str) -> PathBuf; // cache/natives/<id>
    pub fn runtimes_dir(&self) -> PathBuf;     // cache/runtimes
    pub fn installers_dir(&self) -> PathBuf;   // cache/installers
    pub fn objects_dir(&self) -> PathBuf;      // cache/objects
    pub fn object_path(&self, sha1: &str) -> PathBuf; // cache/objects/ab/abcdef...
    pub fn logs_dir(&self) -> PathBuf;
    pub fn ensure_layout(&self) -> Result<(), Error>; // create_dir_all for every dir above except instance_dir
}
pub fn slugify(name: &str) -> String; // lowercase, [a-z0-9-], collapse runs of '-', trim '-', "instance" if empty
pub fn unique_slug(base: &str, exists: impl Fn(&str) -> bool) -> String; // base, base-2, base-3 ...
#[derive(Debug, thiserror::Error)] pub enum Error { #[error("no data directory available on this platform")] NoDataDir, #[error("io error at {path}: {source}")] Io { path: PathBuf, source: std::io::Error } }
```

- [ ] **Step 1: Tests first** (`#[cfg(test)]` in `mod.rs`): `slugify("My Fabric 1.20.1!") == "my-fabric-1-20-1"`, `slugify("") == "instance"`, `unique_slug("x", |s| s == "x" || s == "x-2") == "x-3"`, `object_path("abcdef0123")` ends with `objects/ab/abcdef0123`, `resolve` honors `GCL_ROOT` (set env inside the test with `temp_env`-free approach: call a private `resolve_with(env_root: Option<PathBuf>, override)` that `resolve` wraps; test the private fn), `ensure_layout` creates every directory in a `tempdir`.
- [ ] **Step 2: Run tests, fail, implement, pass.**
- [ ] **Step 3: Add `Paths(#[from] paths::Error)` to `error.rs`, `pub mod paths;`, `just check`, commit** `feat(core): paths and root layout`

---

### Task 4: config module

**Files:**
- Create: `crates/gcl-core/src/config/mod.rs`

**Interfaces:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub root: Option<PathBuf>,
    pub parallel_downloads: usize,           // default 8
    pub jvm: JvmDefaults,                     // { min_mib: 1024, max_mib: 4096, java_path: Option<PathBuf> }
    pub keys: Keys,                           // { curseforge_api_key: Option<String>, msa_client_id: Option<String> }
    pub game_defaults: BTreeMap<String, String>, // options.txt preseed
}
impl Config {
    pub fn load(path: &Path) -> Result<Config, Error>;  // missing file -> Config::default()
    pub fn save(&self, path: &Path) -> Result<(), Error>; // create parent dirs, write toml
    pub fn curseforge_api_key(&self) -> Option<String>; // env CURSEFORGE_API_KEY first
    pub fn msa_client_id(&self) -> Option<String>;      // env GCL_MSA_CLIENT_ID first
}
impl Default for Config { ... }
// Debug for Keys must redact: implement Debug by hand printing "<set>"/"<unset>".
```
Error: `Io { path, source }`, `Parse { path, source: toml::de::Error }`, `Serialize(toml::ser::Error)`.

- [ ] **Step 1: Tests**: default round-trips through save/load in a tempdir; a file with only `[jvm]\nmax_mib = 8192` loads with other defaults intact; `game_defaults` keys survive; `format!("{:?}", Keys { curseforge_api_key: Some("abc".into()), .. })` does not contain `abc`; `curseforge_api_key()` prefers env (set with `std::env::set_var` inside a test guarded by a `static MUTEX` since env is process-global; remove after).
- [ ] **Step 2: Fail, implement, pass. Add `Config(#[from] config::Error)`. Commit** `feat(core): config file with env overrides`

---

### Task 5: http module

**Files:**
- Create: `crates/gcl-core/src/http/mod.rs`
- Modify: `Cargo.toml` (add `reqwest = { version = "0.13", default-features = false, features = ["rustls-tls", "stream", "json", "gzip"] }`), `crates/gcl-core/Cargo.toml`

**Interfaces:**
```rust
#[derive(Clone)]
pub struct HttpClient { inner: reqwest::Client, retries: u32 }
impl HttpClient {
    pub fn new() -> Result<Self, Error>;             // USER_AGENT, gzip, connect_timeout 30s, no total timeout
    pub fn with_retries(self, n: u32) -> Self;        // default 3
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, Error>;
    pub async fn get_json_with_headers<T: DeserializeOwned>(&self, url: &str, headers: &[(&str, &str)]) -> Result<T, Error>;
    pub async fn get_bytes(&self, url: &str) -> Result<bytes::Bytes, Error>;
    pub async fn get_text_if_changed(&self, url: &str, etag: Option<&str>) -> Result<Option<(String, Option<String>)>, Error>; // 304 -> None; else Some((body, new_etag))
    pub async fn stream_to_file(&self, url: &str, dest: &Path, expected_size: Option<u64>, on_chunk: &mut dyn FnMut(u64)) -> Result<StreamResult, Error>; // writes dest, returns { sha1: String, size: u64 }
}
pub struct StreamResult { pub sha1: String, pub size: u64 }
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("request to {url} failed: {source}")] Request { url: String, source: reqwest::Error },
    #[error("{url} returned HTTP {status}")] Status { url: String, status: u16 },
    #[error("{url}: invalid JSON: {source}")] Json { url: String, source: serde_json::Error },
    #[error("io error at {path}: {source}")] Io { path: PathBuf, source: std::io::Error },
    #[error("retries exhausted for {url}: last error: {last}")] RetriesExhausted { url: String, last: String },
}
```
Retry policy: on connect/timeout errors, 5xx, and 429: wait 500 ms, 2 s, 8 s (honor `Retry-After` seconds and `X-Ratelimit-Reset` when larger). Never retry 4xx other than 429. `stream_to_file` writes to `dest` directly (the caller handles `.part` naming), hashes with `sha1::Sha1` as chunks arrive, calls `on_chunk(bytes_so_far)`.

- [ ] **Step 1: Tests with wiremock** (`#[tokio::test]`): `get_json` parses; 500 then 200 → succeeds with retries (use `.up_to_n_times(1)` mock then a 200 mock, and `with_retries(3)`); 404 → `Error::Status` with no retry (mock `.expect(1)`); 429 with `Retry-After: 0` retries; `stream_to_file` writes bytes and returns matching sha1 (compute expected with `sha1` in the test); `get_text_if_changed` returns `None` on 304 when sending `If-None-Match`. Check the `User-Agent` header with a `header("user-agent", USER_AGENT)` matcher on one mock.
- [ ] **Step 2: Fail, implement, pass. Add `Http(#[from] http::Error)`. Commit** `feat(core): http client with retries and streaming`

---

### Task 6: download module

**Files:**
- Create: `crates/gcl-core/src/download/mod.rs`, `crates/gcl-core/src/download/hash.rs`

**Interfaces:**
```rust
pub struct DownloadSpec { pub url: String, pub sha1: Option<String>, pub size: Option<u64>, pub dest: PathBuf, pub label: String }
pub struct DownloadCtx<'a> { pub http: &'a HttpClient, pub root: &'a Root, pub sink: &'a EventSink, pub cancel: &'a CancellationToken, pub parallel: usize }
pub async fn download_one(ctx: &DownloadCtx<'_>, spec: &DownloadSpec) -> Result<PathBuf, Error>;
pub async fn download_all(ctx: &DownloadCtx<'_>, specs: Vec<DownloadSpec>) -> Result<(), Error>; // semaphore(parallel), first hard error cancels rest
pub fn link_or_copy(src: &Path, dest: &Path) -> std::io::Result<()>; // create parents; hard_link, on error copy
pub fn cleanup_partials(root: &Root) -> std::io::Result<usize>;   // removes *.part under cache/, returns count
// hash.rs
pub fn sha1_file(path: &Path) -> std::io::Result<String>;         // streaming, blocking; call under spawn_blocking
pub fn sha1_hex(bytes: &[u8]) -> String;
#[derive(Debug, thiserror::Error)] pub enum Error {
  #[error("hash mismatch for {url}: expected {expected}, got {actual}")] HashMismatch { url: String, expected: String, actual: String },
  #[error("size mismatch for {url}: expected {expected}, got {actual}")] SizeMismatch { url: String, expected: u64, actual: u64 },
  #[error("cancelled")] Cancelled,
  #[error(transparent)] Http(#[from] http::Error),
  #[error("io error at {path}: {source}")] Io { path: PathBuf, source: std::io::Error },
}
```
Algorithm for `download_one`:
1. If `spec.dest` exists and (`sha1` known and `sha1_file(dest) == sha1`, or `sha1` unknown and `size` known and file size matches): return dest.
2. If `sha1` known and `root.object_path(sha1)` exists: `link_or_copy(object, dest)`; return.
3. Else, `TaskHandle::start`; up to 3 attempts: `stream_to_file` to `<object_path or dest>.part`; check sha1 (or size); on mismatch delete `.part`, emit `Warning`, retry. On success rename `.part` → object path (when sha1 known; if sha1 unknown, compute from `StreamResult.sha1` and store there anyway), then `link_or_copy` into `dest`. `finish()` or `fail()`.
4. Check `cancel.is_cancelled()` before each attempt → `Error::Cancelled`.

- [ ] **Step 1: Tests** with wiremock + tempdir root: known-sha1 download lands in `objects/` and `dest`, second call with a different `dest` makes no HTTP request (mock `.expect(1)`); wrong sha1 served → three requests then `HashMismatch` and no `.part` left; size-only spec works; `download_all` with 5 specs and `parallel: 2` completes; a spec that 404s cancels the batch and returns `Http(Status)`; `cleanup_partials` removes a planted `.part`.
- [ ] **Step 2: Fail, implement, pass. Add `Download(#[from] download::Error)`. Commit** `feat(core): content-addressed download cache`

---

### Task 7: mojang manifest, version JSON types, rules

**Files:**
- Create: `crates/gcl-core/src/mojang/mod.rs`, `mojang/manifest.rs`, `mojang/version.rs`, `mojang/rules.rs`

**Interfaces:**
```rust
// mod.rs
pub const PISTON_META: &str = "https://piston-meta.mojang.com";
pub const MANIFEST_PATH: &str = "/mc/game/version_manifest_v2.json";
pub struct Mojang { http: HttpClient, base_url: String, root: Root }
impl Mojang {
    pub fn new(http: HttpClient, root: Root) -> Self;                      // base PISTON_META
    pub fn with_base_url(http: HttpClient, root: Root, base: String) -> Self;
    pub async fn manifest(&self) -> Result<VersionManifest, Error>;      // ETag cache: cache/versions/manifest.json + manifest.etag; on 304 read the cached file; on network error with a cached file, return cached and emit nothing (caller warns)
    pub async fn version(&self, id: &str) -> Result<VersionJson, Error>; // cache/versions/<id>.json by sha1 from manifest; if cached file exists return it without network
    pub fn load_cached_version(&self, id: &str) -> Result<Option<VersionJson>, Error>; // for loader profiles saved by later plans
    pub fn resolve(&self, v: VersionJson) -> Result<VersionJson, Error>; // follow inheritsFrom via load_cached_version, merge (Task 8)
}
// manifest.rs
#[derive(Deserialize, Serialize, Debug, Clone)] pub struct VersionManifest { pub latest: Latest, pub versions: Vec<ManifestEntry> }
pub struct Latest { pub release: String, pub snapshot: String }
pub struct ManifestEntry { pub id: String, #[serde(rename="type")] pub kind: VersionType, pub url: String, pub time: String, #[serde(rename="releaseTime")] pub release_time: String, pub sha1: String, #[serde(rename="complianceLevel", default)] pub compliance_level: u8 }
#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)] #[serde(rename_all="snake_case")] pub enum VersionType { Release, Snapshot, OldBeta, OldAlpha }
// version.rs — serde structs for the version JSON, all Option/default where the field is absent in 1.8.9 or loader profiles:
pub struct VersionJson { pub id: String, pub inherits_from: Option<String>, pub main_class: Option<String>, pub arguments: Option<Arguments>, pub minecraft_arguments: Option<String>, pub libraries: Vec<Library>, pub downloads: Option<Downloads>, pub asset_index: Option<AssetIndexRef>, pub assets: Option<String>, pub java_version: Option<JavaVersion>, pub logging: Option<serde_json::Value>, #[serde(rename="type")] pub kind: Option<String>, pub release_time: Option<String> }
pub struct Arguments { pub game: Vec<Argument>, pub jvm: Vec<Argument> }
#[serde(untagged)] pub enum Argument { Plain(String), Conditional { rules: Vec<Rule>, value: ArgValue } }
#[serde(untagged)] pub enum ArgValue { One(String), Many(Vec<String>) }
pub struct Library { pub name: String, pub downloads: Option<LibraryDownloads>, pub url: Option<String>, pub sha1: Option<String>, pub size: Option<u64>, pub rules: Vec<Rule>, pub natives: Option<BTreeMap<String, String>>, pub extract: Option<Extract> }
pub struct LibraryDownloads { pub artifact: Option<Artifact>, pub classifiers: Option<BTreeMap<String, Artifact>> }
pub struct Artifact { pub path: Option<String>, pub sha1: String, pub size: u64, pub url: String }
pub struct Extract { pub exclude: Vec<String> }
pub struct Downloads { pub client: Artifact, pub server: Option<Artifact>, pub client_mappings: Option<Artifact> }
pub struct AssetIndexRef { pub id: String, pub sha1: String, pub size: u64, pub total_size: Option<u64>, pub url: String }
pub struct JavaVersion { pub component: String, pub major_version: u32 }
pub struct MavenCoord { pub group: String, pub artifact: String, pub version: String, pub classifier: Option<String>, pub ext: String }
impl MavenCoord { pub fn parse(name: &str) -> Result<Self, Error>; pub fn path(&self) -> String; /* group/with/slashes/artifact/version/artifact-version[-classifier].ext */ pub fn key(&self) -> String /* group:artifact */ }
// rules.rs
pub struct Rule { pub action: Action, pub os: Option<OsRule>, pub features: Option<BTreeMap<String, bool>> }
#[serde(rename_all="lowercase")] pub enum Action { Allow, Disallow }
pub struct OsRule { pub name: Option<String>, pub version: Option<String>, pub arch: Option<String> }
pub struct RuleContext { pub os_name: &'static str /* linux|windows|osx */, pub os_version: String, pub arch: &'static str /* x86|x86_64|arm64 */, pub features: BTreeMap<String, bool> }
impl RuleContext { pub fn current() -> Self; }
pub fn rules_allow(rules: &[Rule], ctx: &RuleContext) -> bool; // empty -> true; else start false, walk in order, last match wins
```
Error enum: `Http`, `Io { path, source }`, `Json { path, source }`, `UnknownVersion(String)`, `BadMavenCoord(String)`, `InheritanceLoop(String)`, `MissingField { version: String, field: &'static str }`.

- [ ] **Step 1: Tests**: deserialize `tests/fixtures/mojang/1.20.1.json` and `1.8.9.json` (insta snapshot of `id`, `main_class`, library count, `java_version`); `MavenCoord::parse("org.lwjgl:lwjgl:3.3.1:natives-linux")` → path `org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar`; parse with `@zip` ext suffix; `rules_allow` cases: empty → true; `[allow os linux]` on linux ctx → true, on windows → false; `[allow, disallow os osx]` on osx → false; feature rule with unknown feature → false; `Mojang::manifest` against wiremock serving the fixture writes the cache file, second call with mock returning 304 reads the cache (`.expect(1)` for the 200 mock, a second mock for 304 matching `If-None-Match`); `version("1.20.1")` fetches once, then serves from cache with no request.
- [ ] **Step 2: Fail, implement, pass. Add `Mojang(#[from] mojang::Error)`. Commit** `feat(core): mojang manifest, version json, rules`

---

### Task 8: inheritsFrom merge and argument templating

**Files:**
- Create: `crates/gcl-core/src/mojang/args.rs`
- Modify: `mojang/mod.rs` (implement `resolve`)

**Interfaces:**
```rust
// mod.rs
pub fn merge(parent: VersionJson, child: VersionJson) -> VersionJson;
// rules: child id/main_class/kind win when present; arguments: parent.game ++ child.game, parent.jvm ++ child.jvm (old minecraft_arguments: child wins if present); libraries: index parent by MavenCoord::key(); for each child lib: if the child's id starts with "forge" or contains "forge" or "neoforge" (check child.id lowercase), push (keep both); else replace the parent entry with the same key, or push; downloads/asset_index/assets/java_version/logging: child if Some else parent.
// args.rs
pub struct ArgContext { pub vars: BTreeMap<String, String> }  // placeholder name -> value (without ${})
pub fn expand_arguments(args: &[Argument], rules: &RuleContext, ctx: &ArgContext, sink: Option<&EventSink>) -> Vec<String>;
// Conditional args included only when rules_allow; each string has ${name} replaced; unknown placeholders left as-is and a Warning emitted once per name.
pub fn expand_legacy(minecraft_arguments: &str, ctx: &ArgContext) -> Vec<String>; // split_whitespace then substitute
pub fn default_legacy_jvm_args() -> Vec<String>; // ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"]
```

- [ ] **Step 1: Tests**: merge a hand-built child `{ id: "fabric-loader-0.15.11-1.20.1", inherits_from: "1.20.1", main_class: Some("net.fabricmc.loader.impl.launch.knot.KnotClient"), libraries: [asm 9.6] }` over the 1.20.1 fixture (which has asm 9.3 under `org.ow2.asm:asm`): result has exactly one `org.ow2.asm:asm` and its version is 9.6, main class is Knot, game args count equals parent's; same child with id `1.20.1-forge-47.2.0` keeps both asm entries; `resolve` on a version whose `inherits_from` is itself → `InheritanceLoop`; `expand_arguments` on the 1.20.1 fixture's game args with vars for every placeholder yields a vec containing `--username`, `alice` adjacent and no `${`; a conditional jvm arg with `os.name: osx` is absent on a linux `RuleContext`; unknown placeholder stays and one Warning is sent.
- [ ] **Step 2: Fail, implement, pass, commit** `feat(core): version inheritance merge and argument expansion`

---

### Task 9: assets and vanilla install

**Files:**
- Create: `crates/gcl-core/src/mojang/assets.rs`, `crates/gcl-core/src/mojang/install.rs`

**Interfaces:**
```rust
// assets.rs
pub const RESOURCES_BASE: &str = "https://resources.download.minecraft.net";
#[derive(Deserialize)] pub struct AssetIndex { pub objects: BTreeMap<String, AssetObject>, #[serde(default)] pub virtual_: bool /* rename "virtual" */, #[serde(default)] pub map_to_resources: bool }
pub struct AssetObject { pub hash: String, pub size: u64 }
pub fn asset_specs(index: &AssetIndex, root: &Root, resources_base: &str) -> Vec<DownloadSpec>; // dest cache/assets/objects/ab/abcdef, url base/ab/abcdef, label "asset <path>"
pub fn materialize_legacy(index: &AssetIndex, root: &Root, index_id: &str, game_dir: &Path) -> std::io::Result<()>; // when virtual_ or map_to_resources: link_or_copy each object to assets/virtual/legacy/<path> or <game_dir>/resources/<path>
// install.rs
pub struct InstallPlan { pub client_jar: PathBuf, pub classpath: Vec<PathBuf>, pub natives: Vec<(PathBuf, Vec<String>)> /* jar, exclude */, pub specs: Vec<DownloadSpec>, pub asset_index_id: String, pub java_major: u32, pub natives_dir: PathBuf }
pub fn plan_install(v: &VersionJson, root: &Root, rules: &RuleContext, libraries_base_override: Option<&str>) -> Result<InstallPlan, Error>;
// client jar dest: cache/versions/<id>/<id>.jar (sha1 from downloads.client); libraries: for each lib where rules_allow: artifact = downloads.artifact if present, else build from MavenCoord with url = lib.url.unwrap_or("https://libraries.minecraft.net/") + path, sha1 = lib.sha1, size = lib.size; dest = libraries_dir/<path>; add to classpath; if lib.natives has ctx.os_name and classifiers has that key: add a DownloadSpec for the classifier and push (dest, extract.exclude) to natives. java_major = java_version.major_version or 8.
pub async fn install_version(m: &Mojang, dl: &DownloadCtx<'_>, id: &str) -> Result<InstallPlan, Error>;
// steps: version(id) -> resolve -> plan -> download_all(client + libs + natives) -> fetch asset index (cache/assets/indexes/<id>.json, verify sha1) -> download_all(asset_specs) -> extract natives (spawn_blocking, zip crate, skip excluded prefixes and directories) into plan.natives_dir -> return plan
```
Add `zip = "8"` (or the latest stable 8.x) with `default-features = false, features = ["deflate"]` to workspace deps.

- [ ] **Step 1: Tests**: `asset_specs` on the trimmed `asset_index_1.20.json` fixture yields 20 specs with correct url/dest for the first entry; `plan_install` on 1.20.1 fixture (linux ctx): every classpath entry is under `libraries_dir`, `lwjgl` natives appear as normal libs (modern format, so `natives` vec is empty), client jar path correct, `java_major == 17`; on 1.8.9 fixture: `natives` non-empty with an lwjgl natives-linux entry and `exclude` contains `META-INF/`; `install_version` end to end against wiremock serving: version JSON (from fixture, with all `url` fields rewritten to the mock server by a test helper that string-replaces `https://piston-meta.mojang.com`, `https://libraries.minecraft.net`, `https://resources.download.minecraft.net`, `https://launcher.mojang.com` with the mock URI), and for every artifact URL a mock returning bytes whose sha1 matches — to keep this tractable, the test builds a tiny synthetic VersionJson with 2 libraries, 1 client jar, 1 native jar (a real zip made with the `zip` crate containing `a.so` and `META-INF/x`), and an asset index with 2 objects; asserts files exist and `natives_dir` contains `a.so` but not `META-INF`.
- [ ] **Step 2: Fail, implement, pass, commit** `feat(core): assets and vanilla version install`

---

### Task 10: java module

**Files:**
- Create: `crates/gcl-core/src/java/mod.rs`, `java/detect.rs`, `java/runtime.rs`

**Interfaces:**
```rust
pub struct JavaInstall { pub path: PathBuf /* the java binary */, pub major: u32, pub version: String, pub vendor: String, pub source: JavaSource }
pub enum JavaSource { Path, JavaHome, Mojang, Manual, System }
pub async fn detect_all() -> Vec<JavaInstall>;       // JAVA_HOME/bin/java, `java` on PATH, /usr/lib/jvm/*/bin/java, ~/.jdks/*/bin/java, <root>/cache/runtimes/**/bin/java; dedupe by canonical path; runs `java -XshowSettings:properties -version` under tokio::process with 10 s timeout; parse `java.version` and `java.vendor`
pub fn parse_java_version(s: &str) -> Option<(u32, String)>; // "1.8.0_392" -> 8; "17.0.9" -> 17; "25.0.4.1" -> 25
pub fn pick(installs: &[JavaInstall], major: u32) -> Option<&JavaInstall>; // exact major first, else lowest major >= wanted
pub const RUNTIME_MANIFEST: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
pub fn platform_key() -> Option<&'static str>; // linux, linux-i386, windows-x64, windows-x86, windows-arm64, mac-os, mac-os-arm64
pub async fn install_runtime(http: &HttpClient, dl: &DownloadCtx<'_>, manifest_url: &str, component: &str) -> Result<JavaInstall, Error>;
// fetch all.json -> [platform_key][component][0].manifest.url -> files map { "<path>": { type: "file"|"directory"|"link", downloads: { raw: { sha1, size, url } }, executable: bool } } -> DownloadSpecs into cache/runtimes/<component>/<platform>/<path>; after download set executable bit on unix; return JavaInstall for bin/java (Contents/Home/bin/java on mac-os)
pub fn component_for_major(major: u32) -> &'static str; // 8 -> "jre-legacy", 16 -> "java-runtime-alpha", 17 -> "java-runtime-gamma", 21 -> "java-runtime-delta", else "java-runtime-delta"
```
Error: `Http`, `Download`, `Io`, `NoRuntimeForPlatform { platform, component }`, `UnsupportedPlatform`, `Probe { path, reason }`.

- [ ] **Step 1: Tests**: `parse_java_version` cases above; `pick` prefers exact then next higher; `install_runtime` against wiremock serving the trimmed `java_runtime_all.json` fixture (URLs rewritten to mock) plus a synthetic file manifest with two files (one executable) → files exist, executable bit set on unix, returned path ends with `bin/java`; `detect_all` on this machine finds at least one install (this test is `#[ignore]`-free but tolerant: assert the function returns without error; if `java` exists on PATH assert `len() >= 1`).
- [ ] **Step 2: Fail, implement, pass. Add `Java(#[from] java::Error)`. Commit** `feat(core): java detection and mojang runtimes`

---

### Task 11: instances module

**Files:**
- Create: `crates/gcl-core/src/instances/mod.rs`, `instances/model.rs`

**Interfaces:**
```rust
// model.rs
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)] #[serde(default)]
pub struct InstanceConfig { pub name: String, pub minecraft: String, pub loader: Loader, pub loader_version: Option<String>, pub created: String /* RFC3339 */, pub last_launched: Option<String>, pub jvm: InstanceJvm, pub settings_overrides: BTreeMap<String, String>, pub content: Vec<ContentEntry> }
#[serde(rename_all="lowercase")] pub enum Loader { None, Fabric, Quilt, Forge, NeoForge }
pub struct InstanceJvm { pub min_mib: Option<u32>, pub max_mib: Option<u32>, pub java_path: Option<PathBuf>, pub extra_args: Vec<String> }
pub struct ContentEntry { pub source: String, pub project_id: String, pub version_id: String, pub file_name: String, pub sha1: String, pub kind: ContentKind, pub enabled: bool }
#[serde(rename_all="lowercase")] pub enum ContentKind { Mod, ResourcePack, Shader, DataPack, World }
// mod.rs
pub struct Instance { pub slug: String, pub dir: PathBuf, pub config: InstanceConfig }
impl Instance { pub fn game_dir(&self) -> PathBuf /* dir/.minecraft */; pub fn config_path(&self) -> PathBuf; pub fn save(&self) -> Result<(), Error>; }
pub struct Instances { root: Root }
impl Instances {
    pub fn new(root: Root) -> Self;
    pub fn create(&self, name: &str, minecraft: &str, loader: Loader, loader_version: Option<String>, game_defaults: &BTreeMap<String, String>) -> Result<Instance, Error>; // unique slug; mkdir dir and .minecraft; write instance.toml; write options.txt from game_defaults as key:value lines if non-empty
    pub fn list(&self) -> Result<Vec<Instance>, Error>;   // sorted by name; skip dirs without instance.toml with a tracing::warn
    pub fn get(&self, slug: &str) -> Result<Instance, Error>; // NotFound
    pub fn rename(&self, slug: &str, new_name: &str) -> Result<Instance, Error>; // changes name only, slug unchanged
    pub fn delete(&self, slug: &str) -> Result<(), Error>;   // remove_dir_all
}
```
Error: `NotFound(String)`, `AlreadyExists(String)`, `Io { path, source }`, `Parse { path, source: toml::de::Error }`, `Serialize(toml::ser::Error)`.

- [ ] **Step 1: Tests** (tempdir root): create writes `instance.toml` that round-trips through `toml` (insta snapshot of the file text with `created` redacted via `insta::with_settings!` filters); creating "My Pack" twice yields slugs `my-pack` and `my-pack-2`; `options.txt` has `renderDistance:12` when defaults given and does not exist when empty; `list` returns both sorted; `rename` keeps slug; `delete` removes dir and `get` then returns `NotFound`.
- [ ] **Step 2: Fail, implement, pass. Add `Instance(#[from] instances::Error)`. Commit** `feat(core): instances and instance.toml`

---

### Task 12: launcher handle

**Files:**
- Create: `crates/gcl-core/src/launcher/mod.rs`
- Modify: `lib.rs` (`pub use launcher::Launcher;`), `ARCHITECTURE.md` (no change needed if the `launcher` row exists; confirm)

**Interfaces:**
```rust
pub struct Launcher { runtime: tokio::runtime::Runtime, root: Root, config: Config, http: HttpClient, events: EventSink, cancel: CancellationToken }
impl Launcher {
    pub fn new(root_override: Option<PathBuf>) -> Result<(Launcher, tokio::sync::mpsc::UnboundedReceiver<Event>), Error>;
    // build runtime (multi-thread, name "gcl"); Root::resolve(root_override) then load Config from root.config_file(); if config.root is Some and differs and no GCL_ROOT/override, re-resolve with it; ensure_layout; cleanup_partials; HttpClient::new(); channel
    pub fn root(&self) -> &Root; pub fn config(&self) -> &Config; pub fn config_mut(&mut self) -> &mut Config; pub fn save_config(&self) -> Result<(), Error>;
    pub fn events(&self) -> &EventSink; pub fn cancel_token(&self) -> CancellationToken;
    pub fn block_on<F: Future>(&self, f: F) -> F::Output;
    pub fn mojang(&self) -> Mojang; pub fn instances(&self) -> Instances;
    pub fn download_ctx(&self) -> DownloadCtx<'_>;
    // Convenience, all blocking (they call block_on):
    pub fn list_versions(&self) -> Result<VersionManifest, Error>;
    pub fn install_version(&self, id: &str) -> Result<InstallPlan, Error>;
    pub fn detect_java(&self) -> Vec<JavaInstall>;
    pub fn ensure_java(&self, major: u32) -> Result<JavaInstall, Error>; // pick from detect_all, else install_runtime(component_for_major)
}
```

- [ ] **Step 1: Test**: `Launcher::new(Some(tempdir))` creates the layout and a default config when none exists; `save_config` then `new` again loads it; `list_versions` is not unit-tested here (network) — covered by CLI tests with wiremock via `GCL_MOJANG_BASE_URL` env override read in `Launcher::mojang()` (document this env var as test-only in the `testing` skill).
- [ ] **Step 2: Fail, implement, pass, commit** `feat(core): launcher handle`

---

### Task 13: CLI commands

**Files:**
- Create: `crates/gcl-cli/src/output.rs`, `crates/gcl-cli/src/commands/mod.rs`, `commands/version.rs`, `commands/instance.rs`, `commands/java.rs`, `commands/config.rs`, `commands/debug.rs`
- Modify: `crates/gcl-cli/src/main.rs`, `crates/gcl-cli/Cargo.toml` (add `serde_json`, `tokio` if needed, `serde`), `crates/gcl-cli/tests/cli.rs`

**Interfaces (CLI surface):**
```
gcl [--json] [--root <path>] <command>
  version list [--snapshots]               -> table: id, type, releaseTime; json: manifest entries
  version install <id>                     -> installs vanilla into cache; prints progress lines (or json events); final line "installed <id>"
  instance create <name> --minecraft <id> [--loader none|fabric|quilt|forge|neoforge] [--loader-version <v>]
  instance list                            -> slug, name, minecraft, loader
  instance delete <slug> --yes
  instance rename <slug> <new name>
  java list                                -> path, major, version, vendor
  java ensure <major>                      -> prints the chosen or installed path
  config show                              -> toml (keys redacted)
  config set-root <path>
  config set-jvm --min <mib> --max <mib>
  debug verify-source <source>             -> for "mojang": fetch live manifest + latest release version JSON, parse, print PASS/FAIL per endpoint; other sources still exit 2 "not implemented"
```
`output.rs`: `enum Format { Text, Json }`, `fn print_events(rx, format)` runs on a std thread and renders `Event`s (text: `[label] 45%`, one line per TaskStarted/Finished/Failed; json: one JSON object per line). Errors print `{:#}` to stderr, exit 1.

- [ ] **Step 1: Tests** (`tests/cli.rs`, `GCL_ROOT` = tempdir, `GCL_MOJANG_BASE_URL` = wiremock serving the manifest fixture): `version list --json` prints an array with `1.20.1`; `instance create "Test" --minecraft 1.20.1` then `instance list --json` shows slug `test`; `instance delete test --yes` then list is empty; `config set-jvm --min 1024 --max 4096` then `config show` contains `max_mib = 4096`; `java list` exits 0.
- [ ] **Step 2: Fail, implement, pass, commit** `feat(cli): version, instance, java, config commands`

---

### Task 14: Docs and verification

- [ ] **Step 1:** Update `ARCHITECTURE.md` module table if any module's dependencies differ from what was built. Update `.claude/skills/testing/SKILL.md` with the `GCL_MOJANG_BASE_URL` test-only override and the URL-rewrite helper pattern.
- [ ] **Step 2:** Run `just check && just deny && just lint-claude`. Run the api-verifier flow by hand once: `just verify-api mojang` — expect PASS lines for manifest and version.
- [ ] **Step 3:** Commit `docs: architecture and testing updates for core foundation`. Report: modules built, test count, what `just verify-api mojang` printed.
