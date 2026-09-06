# MVP Plan 2: Loaders, Settings, Offline Accounts, Launch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Install Fabric, Quilt, Forge, and NeoForge into the shared version cache, apply per-instance `options.txt` overrides, manage offline accounts, and launch an instance (dry-run and real) from the CLI, so `just e2e` passes end to end except the content step.

**Architecture:** Four new `gcl-core` modules (`loaders`, `settings`, `auth`, `launch`) over the plan 1 foundation. Loaders write a version JSON with `inheritsFrom` into `cache/versions/<id>.json`; `Mojang::resolve(v, keep_both)` merges it; `plan_install` and `install_version` already handle libraries, natives, and assets for the merged JSON. Forge and NeoForge share one installer module that runs processors through a `ProcessRunner` trait so the logic is unit-testable without Java. `launch` turns an `InstallPlan`, an instance, an account, and JVM settings into a `LaunchCommand`, then spawns Java and streams logs as events.

**Tech Stack:** as plan 1, plus `uuid` v3 (already a dep), `md-5` (workspace), `zip` (workspace).

**Spec:** `docs/SPEC.md` R2.3, R2.6, R4, R5.3, R6.2-R6.4, R9, R10, R11, R12.1 (loader, account, settings, launch), plus `ARCHITECTURE.md` and skills `modloaders`, `instance-model`, `msa-auth` (offline section), `mojang-meta`.

## Global Constraints

- Everything in plan 1's Global Constraints still applies (thiserror in core, anyhow in CLI, no unwrap outside tests, workspace deps, `just check` clean per task, commit per task with the `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` trailer, no network in tests, `paths::safe_join` for every remote-supplied relative path, `paths::write_atomic` for every file under the root).
- Loader version ids: Fabric `fabric-loader-<loader>-<mc>`, Quilt `quilt-loader-<loader>-<mc>`, Forge `<mc>-forge-<forge>`, NeoForge `neoforge-<v>`. These are the `id` values written into `cache/versions/<id>.json` and stored in `InstanceConfig`.
- Forge and NeoForge profiles are resolved with `keep_both_libraries = true`; Fabric and Quilt with `false`. `InstanceConfig.loader` decides; the id sniff (`resolve_auto`) is no longer used by any new code.
- Processors run with the Java chosen for the Minecraft version (`Launcher::ensure_java_for(plan)`), never a hardcoded path.
- `options.txt` is `key:value` per line; overrides replace a matching key's line in place, append missing keys at the end, and touch nothing else.
- Offline UUID: version 3 UUID from MD5 of `OfflinePlayer:<name>` bytes, matching Java `UUID.nameUUIDFromBytes` (set version nibble to 3 and variant bits to RFC 4122). `uuid::Uuid::new_v3` with a namespace is NOT the same and must not be used.
- Launch placeholders (`${auth_player_name}` etc.) as listed in `.claude/skills/mojang-meta/SKILL.md` "Placeholders". Offline: `auth_access_token=0`, `user_type=legacy`, `clientid=""`, `auth_xuid=""`.
- Classpath separator `:` on unix, `;` on windows. Working directory is the instance `.minecraft`.
- No secrets are logged. Launch logs go to `<root>/logs/<slug>-<timestamp>.log` and to events.

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `tests/fixtures/{fabric,quilt,forge,neoforge}/*.json` | recorded loader metadata | 1 |
| `crates/gcl-core/src/loaders/{mod,fabriclike,fabric,quilt}.rs` | loader contract, Fabric and Quilt profile install | 2 |
| `crates/gcl-core/src/loaders/{forgelike,forge,neoforge,processors}.rs` | installer jar handling, processors, Forge and NeoForge | 3, 4 |
| `crates/gcl-core/src/settings/mod.rs` | options.txt parse, override, write | 5 |
| `crates/gcl-core/src/auth/{mod,offline,store}.rs` | account model, offline UUID, accounts.json store | 6 |
| `crates/gcl-core/src/launch/{mod,command,spawn}.rs` | LaunchCommand build, spawn, log streaming | 7 |
| `crates/gcl-core/src/launcher/mod.rs` | `install_instance`, `launch_instance`, loader helpers | 8 |
| `crates/gcl-cli/src/commands/{loader,account,settings,launch,debug}.rs`, `scripts/e2e.sh` | CLI and e2e | 9 |
| `ARCHITECTURE.md`, skills | docs | 10 |

---

### Task 1: Record loader fixtures

**Files:** `tests/fixtures/fabric/loader_1.20.1.json`, `fabric/profile_1.20.1.json`, `quilt/loader_1.20.1.json`, `quilt/profile_1.20.1.json`, `forge/promotions_slim.json`, `forge/maven-metadata.json`, `forge/install_profile_1.20.1-47.3.0.json`, `forge/version_1.20.1-47.3.0.json`, `neoforge/versions.json`, `neoforge/install_profile_21.1.65.json`, `neoforge/version_21.1.65.json`

- [ ] **Step 1: Fabric and Quilt**
```bash
just record-fixture fabric loader_1.20.1 'https://meta.fabricmc.net/v2/versions/loader/1.20.1'
L=$(python3 -c "import json;print(json.load(open('tests/fixtures/fabric/loader_1.20.1.json'))[0]['loader']['version'])")
just record-fixture fabric profile_1.20.1 "https://meta.fabricmc.net/v2/versions/loader/1.20.1/$L/profile/json"
just record-fixture quilt loader_1.20.1 'https://meta.quiltmc.org/v3/versions/loader/1.20.1'
Q=$(python3 -c "import json;print(json.load(open('tests/fixtures/quilt/loader_1.20.1.json'))[0]['loader']['version'])")
just record-fixture quilt profile_1.20.1 "https://meta.quiltmc.org/v3/versions/loader/1.20.1/$Q/profile/json"
```
Trim both `loader_1.20.1.json` arrays to their first 3 entries.

- [ ] **Step 2: Forge metadata and installer JSONs**
```bash
just record-fixture forge promotions_slim 'https://maven.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json'
just record-fixture forge maven-metadata 'https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json'
```
Trim `maven-metadata.json` to keys `1.20.1`, `1.21.1`, `1.12.2` (keep each array whole). Trim `promotions_slim.json` `promos` to keys starting with `1.20.1-`, `1.21.1-`, `1.12.2-`. Then fetch one installer and extract only its two JSON files (the jar itself is not committed):
```bash
F=$(python3 -c "import json;print(json.load(open('tests/fixtures/forge/promotions_slim.json'))['promos']['1.20.1-recommended'])")
curl -fsSL -o /tmp/forge-installer.jar "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-$F/forge-1.20.1-$F-installer.jar"
unzip -p /tmp/forge-installer.jar install_profile.json | python3 -m json.tool > "tests/fixtures/forge/install_profile_1.20.1-$F.json"
unzip -p /tmp/forge-installer.jar version.json | python3 -m json.tool > "tests/fixtures/forge/version_1.20.1-$F.json"
unzip -l /tmp/forge-installer.jar | grep -E 'data/|maven/' > tests/fixtures/forge/installer_listing_1.20.1.txt
rm /tmp/forge-installer.jar
```
Name the files after the actual version you got (the plan says 47.3.0; use what `promotions_slim` returned and note it in the report).

- [ ] **Step 3: NeoForge**
```bash
just record-fixture neoforge versions 'https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge'
```
Trim `versions` to the last 5 entries plus the newest `21.1.x`. Pick the newest `21.1.x` as `$N` and repeat the installer extraction from Step 2 with `https://maven.neoforged.net/releases/net/neoforged/neoforge/$N/neoforge-$N-installer.jar`, saving `install_profile_$N.json`, `version_$N.json`, and `installer_listing_$N.txt`.

- [ ] **Step 4: Validate and commit**
Every file is valid JSON (`python3 -m json.tool`), under 200 KB. Commit `test: record loader fixtures`.

---

### Task 2: Loader contract, Fabric and Quilt

**Files:**
- Create: `crates/gcl-core/src/loaders/mod.rs`, `loaders/fabriclike.rs`, `loaders/fabric.rs`, `loaders/quilt.rs`

**Interfaces:**
```rust
// loaders/mod.rs
pub use crate::instances::model::Loader;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoaderVersion { pub version: String, pub stable: bool, pub recommended: bool }
pub struct LoaderCtx<'a> { pub http: &'a HttpClient, pub root: &'a Root, pub dl: &'a DownloadCtx<'a>, pub java: Option<&'a JavaInstall> /* required for forge-like */ }
pub struct LoaderEndpoints { pub fabric: String, pub quilt: String, pub forge: String, pub neoforge: String } // base URLs; Default = production; tests override
pub fn version_id(loader: Loader, mc: &str, loader_version: &str) -> String;   // the id scheme in Global Constraints
pub fn keep_both_libraries(loader: Loader) -> bool;                               // Forge | NeoForge
pub async fn list_versions(ctx: &LoaderCtx<'_>, ep: &LoaderEndpoints, loader: Loader, mc: &str) -> Result<Vec<LoaderVersion>, Error>;
pub async fn install(ctx: &LoaderCtx<'_>, ep: &LoaderEndpoints, loader: Loader, mc: &str, loader_version: &str) -> Result<String /* version id */, Error>;
// install writes cache/versions/<id>.json (paths::write_atomic) with `inheritsFrom = mc` and `id` = version_id(); returns early (no network) when that file already exists and parses.
#[derive(Debug, thiserror::Error)] pub enum Error { Http(#[from] http::Error), Download(#[from] download::Error), Mojang(#[from] mojang::Error), Paths(#[from] paths::Error), Io { path, source }, Json { url, source }, #[error("no {loader} version {version} for Minecraft {mc}")] NoSuchVersion { loader: Loader, mc: String, version: String }, #[error("Minecraft {0} has no {1} builds")] Unsupported(String, Loader), #[error("legacy Forge installer (pre-1.13) is not supported")] LegacyInstaller, #[error("processor {main} failed with exit {code}: see {log}")] ProcessorFailed { main: String, code: i32, log: PathBuf }, #[error("processor output {path} missing or hash mismatch")] ProcessorOutput { path: PathBuf }, #[error("java runtime required to install {0}")] JavaRequired(Loader), #[error("zip error in {path}: {source}")] Zip { path: PathBuf, source: zip::result::ZipError }, #[error("unsafe path in installer: {0}")] UnsafePath(String) }
// loaders/fabriclike.rs — shared by fabric and quilt
pub async fn list(ctx, base: &str, api_path: &str /* "v2" or "v3" */, mc) -> Result<Vec<LoaderVersion>, Error>; // GET {base}/{api}/versions/loader/{mc} → entries with loader.version, loader.stable; recommended = first stable
pub async fn install(ctx, base, api_path, loader: Loader, mc, loader_version) -> Result<String, Error>;   // GET {base}/{api}/versions/loader/{mc}/{lv}/profile/json → VersionJson; set id = version_id(); ensure inherits_from = Some(mc); write cache/versions/<id>.json
// fabric.rs / quilt.rs: `pub const BASE`, `pub const API` and thin wrappers.
```
Library entries in Fabric/Quilt profiles have `name` + `url` (+ optional `sha1`, `size`); `plan_install` already builds specs from `MavenCoord` and `lib.url` (plan 1), so no download work here beyond writing the JSON.

- [ ] **Step 1: Tests** (`#[cfg(test)]`, wiremock + fixtures): `list_versions(Fabric, "1.20.1")` returns 3 entries, first `recommended`; `install(Fabric, "1.20.1", <first>)` writes `cache/versions/fabric-loader-<lv>-1.20.1.json` whose `inherits_from == Some("1.20.1")` and `main_class` is Knot; a second `install` makes no request (`.expect(1)`); unknown version → 404 mock → `NoSuchVersion`; `Mojang::resolve(loaded profile, false)` over the 1.20.1 fixture (plant it at `cache/versions/1.20.1.json`) yields a JSON whose classpath plan (`plan_install`) includes `fabric-loader` and `intermediary` jars and Knot main class; same for Quilt with `quilt-loader`.
- [ ] **Step 2: Fail, implement, pass. Add `Loaders(#[from] loaders::Error)` to `error.rs`. Commit** `feat(core): fabric and quilt loader install`

---

### Task 3: Installer jar handling and processors (Forge-like core)

**Files:**
- Create: `crates/gcl-core/src/loaders/forgelike.rs`, `crates/gcl-core/src/loaders/processors.rs`

**Interfaces:**
```rust
// forgelike.rs
#[derive(Deserialize, Debug)] pub struct InstallProfile { pub spec: Option<u32>, pub profile: Option<String>, pub version: Option<String>, pub minecraft: Option<String>, pub json: Option<String> /* "/version.json" */, #[serde(default)] pub data: BTreeMap<String, DataEntry>, #[serde(default)] pub processors: Vec<Processor>, #[serde(default)] pub libraries: Vec<Library>, pub version_info: Option<serde_json::Value> /* legacy marker */ }
#[derive(Deserialize, Debug, Clone)] pub struct DataEntry { pub client: String, pub server: String }
#[derive(Deserialize, Debug, Clone)] pub struct Processor { #[serde(default)] pub sides: Vec<String>, pub jar: String, #[serde(default)] pub classpath: Vec<String>, #[serde(default)] pub args: Vec<String>, #[serde(default)] pub outputs: BTreeMap<String, String> }
pub struct InstallerJar { pub path: PathBuf }
impl InstallerJar {
    pub fn open(path: PathBuf) -> Result<Self, Error>;
    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>, Error>;              // blocking; callers use spawn_blocking
    pub fn extract_prefix(&self, prefix: &str /* "maven/" */, dest: &Path) -> Result<Vec<PathBuf>, Error>; // safe_join, skip dirs
    pub fn extract_file(&self, name: &str, dest_dir: &Path) -> Result<PathBuf, Error>;
    pub fn manifest_main_class(&self) -> Result<Option<String>, Error>;           // META-INF/MANIFEST.MF Main-Class (for a processor jar, opened the same way)
}
pub struct DataMap(pub BTreeMap<String, String>);
pub fn build_data_map(profile: &InstallProfile, jar: &InstallerJar, root: &Root, mc: &str, client_jar: &Path, temp: &Path) -> Result<DataMap, Error>;
// for each data entry take `.client`: "[g:a:v]" → libraries_dir/<MavenCoord path>; "'literal'" → literal; "/path" → extract_file(path, temp); plus MINECRAFT_JAR, SIDE=client, INSTALLER, ROOT (root.path), MINECRAFT_VERSION, LIBRARY_DIR
pub fn substitute(arg: &str, data: &DataMap, root: &Root) -> Result<String, Error>; // replace {KEY}; "[g:a:v]" → library path; unknown {KEY} → error
pub fn library_specs(libs: &[Library], root: &Root, rules: &RuleContext) -> Result<Vec<DownloadSpec>, Error>; // reuse mojang::install::library_spec logic (make it pub(crate) there) 
// processors.rs
#[async_trait] pub trait ProcessRunner: Send + Sync { async fn run(&self, java: &Path, classpath: &[PathBuf], main: &str, args: &[String], log: &Path) -> Result<i32, std::io::Error>; }
pub struct JavaRunner;   // tokio::process, writes stdout+stderr to `log`
pub async fn run_processors(runner: &dyn ProcessRunner, java: &Path, profile: &InstallProfile, data: &DataMap, root: &Root, log_dir: &Path, sink: &EventSink) -> Result<(), Error>;
// for each processor with sides empty or containing "client": if all `outputs` exist with expected sha1 (values are "{SHA}" tokens resolved via data map or literal hex) → skip; else classpath = [jar] + classpath (maven coords → libraries_dir), main = manifest_main_class(jar), args substituted, run; non-zero → ProcessorFailed; then verify outputs → ProcessorOutput. Emit TaskStarted/Finished per processor.
```
`Library` here is `mojang::version::Library` (the installer's `libraries` use the same shape with `downloads.artifact`).

- [ ] **Step 1: Tests**: parse the Forge and NeoForge `install_profile_*.json` fixtures (insta snapshot of counts: data keys, processors, libraries; `json == Some("/version.json")`); `build_data_map` over a synthetic profile with all three value forms, a synthetic jar built with the `zip` crate containing `data/client.lzma` and `maven/net/x/y/1/y-1.jar`; `substitute("{MINECRAFT_JAR}")`, `"[net.x:y:1]"`, unknown key → error; `extract_prefix("maven/")` lands `y-1.jar` under `libraries_dir` and rejects an entry named `maven/../evil.jar`; `run_processors` with a `FakeRunner` (records calls, writes the declared outputs with known content) over a two-processor profile: first run executes both, second run executes none (outputs present and hashes match); a runner returning exit 1 → `ProcessorFailed` with the log path; a processor whose output is not written → `ProcessorOutput`.
- [ ] **Step 2: Fail, implement, pass, commit** `feat(core): forge installer parsing and processor runner`

---

### Task 4: Forge and NeoForge install

**Files:**
- Create: `crates/gcl-core/src/loaders/forge.rs`, `crates/gcl-core/src/loaders/neoforge.rs`
- Modify: `loaders/mod.rs` (dispatch), `loaders/forgelike.rs` (`install` orchestration)

**Interfaces:**
```rust
// forge.rs
pub const MAVEN: &str = "https://maven.minecraftforge.net";
pub async fn list(ctx, base, mc) -> Result<Vec<LoaderVersion>, Error>; // GET {base}/net/minecraftforge/forge/maven-metadata.json → map[mc] → versions "<mc>-<forge>" (strip prefix); GET promotions_slim.json → mark recommended/latest; newest first
pub fn installer_url(base: &str, mc: &str, forge: &str) -> String;   // {base}/net/minecraftforge/forge/{mc}-{forge}/forge-{mc}-{forge}-installer.jar
// neoforge.rs
pub const MAVEN: &str = "https://maven.neoforged.net";
pub async fn list(ctx, base, mc) -> Result<Vec<LoaderVersion>, Error>; // GET {base}/api/maven/versions/releases/net/neoforged/neoforge → versions; filter by mc_for_version(v) == mc; newest first; latest = recommended
pub fn mc_for_version(v: &str) -> Option<String>; // "21.1.65" → "1.21.1"; "20.4.190" → "1.20.4"; "21.0.5" → "1.21"
pub fn installer_url(base, v) -> String;          // {base}/releases/net/neoforged/neoforge/{v}/neoforge-{v}-installer.jar
// forgelike.rs
pub async fn install(ctx: &LoaderCtx<'_>, loader: Loader, mc: &str, lv: &str, installer_url: String) -> Result<String, Error>;
// 1. id = version_id(); if cache/versions/<id>.json exists and all processor outputs exist → return id.
// 2. download installer to cache/installers/<file name> (download_one, no sha1, size unknown).
// 3. spawn_blocking: open jar, read install_profile.json; legacy (version_info present, spec absent) → LegacyInstaller; read the file named by `json` (strip leading '/'); extract_prefix("maven/", libraries_dir).
// 4. download_all(library_specs(profile.libraries) + library_specs(version.libraries)) — libraries with no downloads.artifact and no url are skipped (they come from maven/).
// 5. ensure vanilla client jar via mojang::install_version(mc) (uses ctx.dl) → plan.client_jar.
// 6. build_data_map, run_processors(JavaRunner, ctx.java.ok_or(JavaRequired)...), logs under root.logs_dir()/installers/.
// 7. set version.id = id, inherits_from = Some(mc); write cache/versions/<id>.json. Return id.
```

- [ ] **Step 1: Tests**: `forge::list` over fixtures (wiremock) returns 1.20.1 versions newest first with `recommended` set from promotions; `neoforge::mc_for_version` cases; `neoforge::list` filters to 1.21.1; `installer_url` strings; `forgelike::install` end-to-end over wiremock with a synthetic installer jar (built in the test: `install_profile.json` with one processor and a `maven/` entry, `version.json` with `inheritsFrom`, `data/` entry) and a `FakeRunner` injected through a `LoaderCtx.runner: Option<&dyn ProcessRunner>` field (add it; `None` → `JavaRunner`): asserts the version JSON is written with the right id, the maven jar is in `libraries_dir`, and a second install performs no download (`.expect(1)` on the installer mock). Vanilla client jar for step 5 comes from a wiremock-served synthetic version JSON like plan 1's install e2e helper (reuse or copy it into `crates/gcl-core/tests/common/`).
- [ ] **Step 2: Fail, implement, pass, commit** `feat(core): forge and neoforge install`

---

### Task 5: settings module

**Files:**
- Create: `crates/gcl-core/src/settings/mod.rs`

**Interfaces:**
```rust
pub struct OptionsFile { lines: Vec<Line> }  // Line = Pair { key, value } | Other(String)
impl OptionsFile {
    pub fn parse(text: &str) -> OptionsFile;                 // "key:value" split at first ':'; other lines kept verbatim
    pub fn get(&self, key: &str) -> Option<&str>;
    pub fn set(&mut self, key: &str, value: &str);           // replace first matching line, else append
    pub fn remove(&mut self, key: &str) -> bool;
    pub fn to_string(&self) -> String;                        // trailing newline
}
pub fn read(path: &Path) -> Result<OptionsFile, Error>;       // missing → empty
pub fn write(path: &Path, f: &OptionsFile) -> Result<(), Error>; // write_atomic
pub fn apply_overrides(instance: &Instance) -> Result<usize, Error>; // options.txt in game_dir; applies instance.config.settings_overrides; returns keys changed
pub fn apply_preseed(game_dir: &Path, defaults: &BTreeMap<String, String>) -> Result<(), Error>; // only when options.txt is absent (move the plan-1 create logic here and call it from instances::create)
```

- [ ] **Step 1: Tests**: parse/round-trip preserves order and non-pair lines; `set` replaces in place; `set` appends; `apply_overrides` on a planted `options.txt` with `renderDistance:8\nguiScale:2\n` and overrides `{renderDistance: 16, lang: "en_us"}` yields `renderDistance:16\nguiScale:2\nlang:en_us\n`; `apply_preseed` does not touch an existing file.
- [ ] **Step 2: Fail, implement, pass. Add `Settings(#[from])`. Commit** `feat(core): options.txt preseed and keyed overrides`

---

### Task 6: auth module (offline accounts and store)

**Files:**
- Create: `crates/gcl-core/src/auth/mod.rs`, `auth/offline.rs`, `auth/store.rs`

**Interfaces:**
```rust
// mod.rs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)] pub struct Account { pub id: String /* uuid, dashed */, pub name: String, pub kind: AccountKind, #[serde(default)] pub mc_token: Option<String>, #[serde(default)] pub mc_token_expires: Option<String>, #[serde(default)] pub xuid: Option<String> }
#[derive(..)] #[serde(rename_all = "lowercase")] pub enum AccountKind { Offline, Msa }
pub struct LaunchIdentity { pub name: String, pub uuid_undashed: String, pub access_token: String, pub user_type: String, pub xuid: String, pub client_id: String }
impl Account { pub fn launch_identity(&self) -> LaunchIdentity; } // offline: token "0", user_type "legacy", xuid "", client_id ""; msa: from fields (plan 4 fills them)
impl std::fmt::Debug for Account — redact mc_token.
// offline.rs
pub fn offline_uuid(name: &str) -> uuid::Uuid;  // MD5("OfflinePlayer:" + name), set version 3 + RFC4122 variant
pub fn offline_account(name: &str) -> Account;
// store.rs
#[derive(Serialize, Deserialize, Default)] pub struct AccountsFile { pub accounts: Vec<Account>, pub active: Option<String> }
pub struct Accounts { path: PathBuf }
impl Accounts { pub fn new(root: &Root) -> Self; pub fn load(&self) -> Result<AccountsFile, Error>; pub fn save(&self, f: &AccountsFile) -> Result<(), Error>; pub fn add(&self, a: Account) -> Result<Account, Error> /* replaces same id; sets active if none */; pub fn remove(&self, id: &str) -> Result<(), Error>; pub fn select(&self, id_or_name: &str) -> Result<Account, Error>; pub fn active(&self) -> Result<Option<Account>, Error>; pub fn list(&self) -> Result<Vec<Account>, Error>; }
```
Error: `Io { path, source }`, `Json { path, source }`, `NotFound(String)`.

- [ ] **Step 1: Tests**: `offline_uuid("Notch") == b50ad385-829d-3141-a216-7e7d7539ba7f` (known Java value); case-sensitivity (`notch` differs); store add/list/select/remove round-trip in a tempdir; `add` of an existing id replaces; `active()` after removing the active account is `None`; `Debug` of an account with a token does not contain the token.
- [ ] **Step 2: Fail, implement, pass. Add `Auth(#[from])`. Commit** `feat(core): offline accounts and account store`

---

### Task 7: launch module

**Files:**
- Create: `crates/gcl-core/src/launch/mod.rs`, `launch/command.rs`, `launch/spawn.rs`

**Interfaces:**
```rust
// command.rs
pub struct LaunchInputs<'a> { pub plan: &'a InstallPlan, pub instance: &'a Instance, pub identity: &'a LaunchIdentity, pub java: &'a Path, pub root: &'a Root, pub jvm: JvmSettings, pub rules: &'a RuleContext, pub launcher_name: &'a str, pub launcher_version: &'a str, pub resolution: Option<(u32, u32)> }
pub struct JvmSettings { pub min_mib: u32, pub max_mib: u32, pub extra_args: Vec<String> } // resolved from instance override else config defaults (Launcher does the resolution)
#[derive(Debug, Clone, Serialize, PartialEq)] pub struct LaunchCommand { pub program: PathBuf, pub args: Vec<String>, pub cwd: PathBuf, pub env: Vec<(String, String)> }
pub fn build(inputs: &LaunchInputs<'_>, sink: Option<&EventSink>) -> Result<LaunchCommand, Error>;
// classpath = plan.classpath + [plan.client_jar] joined with the OS separator (client jar last; Forge expects that)
// vars: all placeholders from the mojang-meta skill; natives_directory = plan.natives_dir; library_directory = root.libraries_dir(); classpath_separator; version_name = plan.resolved.id; version_type = plan.resolved.kind or "release"; assets_root = root.assets_dir(); assets_index_name = plan.asset_index_id; game_directory = instance.game_dir(); resolution vars only when Some (and set feature has_custom_resolution)
// args order: ["-Xms{min}M", "-Xmx{max}M"] + jvm.extra_args + expanded arguments.jvm (or default_legacy_jvm_args when arguments is None) + [main_class] + expanded arguments.game (or expand_legacy(minecraft_arguments))
// legacy assets: if plan.resolved.assets is "legacy" or "pre-1.6" (or asset index virtual/map_to_resources): call mojang::assets::materialize_legacy before building and point assets_root at assets/virtual/<id>
// main_class missing → Error::MissingMainClass
// spawn.rs
pub struct RunningGame { pub child: tokio::process::Child, pub log_path: PathBuf }
pub async fn spawn(cmd: &LaunchCommand, log_path: PathBuf, sink: EventSink) -> Result<RunningGame, Error>; // stdout+stderr piped, each line → Event::Log{ level: Info / Error for stderr, message } and appended to log file; do not block on the child
pub async fn wait(game: RunningGame) -> Result<i32, Error>;  // exit code; emits Event::Log summary line
```
Error: `MissingMainClass`, `Io { path, source }`, `Spawn { program, source }`, `Legacy(#[from] mojang::assets::LegacyError)`.

- [ ] **Step 1: Tests** (no Java needed): `build` over an `InstallPlan` constructed from the 1.20.1 fixture (`plan_install` with a tempdir root) with an offline identity: insta snapshot of `args` with the tempdir path filtered to `<root>`; asserts: `-Xmx4096M` present, `--username alice`, `--uuid` undashed, `--accessToken 0`, `--userType legacy`, `-Djava.library.path=<natives>`, classpath ends with the client jar, `net.minecraft.client.main.Main` after the jvm args; legacy 1.8.9 fixture → `--tweakClass`-free but `--assetIndex 1.8` and default legacy jvm args; `spawn` with `program = "sh"` and args `["-c", "echo out; echo err 1>&2; exit 3"]` (unix-only test) → two Log events, log file has both lines, `wait` returns 3.
- [ ] **Step 2: Fail, implement, pass. Add `Launch(#[from])`. Commit** `feat(core): launch command build and process spawn`

---

### Task 8: Launcher wiring

**Files:**
- Modify: `crates/gcl-core/src/launcher/mod.rs`

**Interfaces:**
```rust
impl Launcher {
    pub fn loader_endpoints(&self) -> LoaderEndpoints;   // env overrides GCL_FABRIC_BASE_URL, GCL_QUILT_BASE_URL, GCL_FORGE_BASE_URL, GCL_NEOFORGE_BASE_URL (test-only)
    pub fn list_loader_versions(&self, loader: Loader, mc: &str) -> Result<Vec<LoaderVersion>, crate::Error>;
    pub fn install_loader(&self, slug: &str) -> Result<String, crate::Error>;       // reads instance; loader None → Ok(mc id); resolves "recommended"/"latest"/None loader_version to a concrete one and saves it into instance.toml; ensures java for forge-like via install_version(mc) → ensure_java_for; calls loaders::install; returns version id
    pub fn install_instance(&self, slug: &str) -> Result<InstallPlan, crate::Error>; // install_loader then mojang.version(id) → resolve(v, keep_both_libraries(loader)) → plan_install → download_all specs → assets → natives (reuse install_version_with by adding `install_resolved(m, dl, resolved: VersionJson)` in mojang::install and have install_version_with call it)
    pub fn accounts(&self) -> Accounts;
    pub fn launch_instance(&self, slug: &str, account: Option<&str> /* id or name; None → active */, offline_user: Option<&str> /* creates/uses offline account */, dry_run: bool) -> Result<LaunchOutcome, crate::Error>;
    // resolves account (offline_user wins; else account; else active; else Error::NoAccount), install_instance, ensure_java (instance java_path override wins), settings::apply_overrides, build; dry_run → LaunchOutcome::DryRun(cmd); else spawn, set last_launched, wait → LaunchOutcome::Exited { code, log_path }
}
pub enum LaunchOutcome { DryRun(LaunchCommand), Exited { code: i32, log_path: PathBuf } }
```
Add `NoAccount` to `crate::Error` (or an `auth::Error::NoAccount`).

- [ ] **Step 1: Tests**: `install_loader` on an instance with loader Fabric and `loader_version: None` over wiremock (fabric fixtures) sets `loader_version` to the recommended one and writes the profile; `launch_instance(dry_run)` over wiremock-served vanilla (plan 1's synthetic version helper) with `offline_user: Some("tester")` returns `DryRun` whose args contain `--username tester` and whose `cwd` is the instance game dir, and `options.txt` received the instance overrides; `launch_instance` with no account and no offline user → `NoAccount`.
- [ ] **Step 2: Fail, implement, pass, commit** `feat(core): launcher install and launch orchestration`

---

### Task 9: CLI and e2e

**Files:**
- Create: `crates/gcl-cli/src/commands/{loader,account,settings,launch}.rs`
- Modify: `crates/gcl-cli/src/main.rs`, `commands/mod.rs`, `commands/debug.rs`, `commands/instance.rs`, `scripts/e2e.sh`, `crates/gcl-cli/tests/cli.rs`

**CLI surface:**
```
gcl loader list <mc> --loader fabric|quilt|forge|neoforge
gcl loader install <slug>                              -> installs the instance's loader (and vanilla); prints version id
gcl account add-offline <name>                         -> prints id; becomes active if first
gcl account list | select <id|name> | remove <id|name>
gcl settings show <slug>                               -> current options.txt pairs and the override map
gcl settings set <slug> <key> <value> | unset <slug> <key>
gcl settings defaults show | set <key> <value> | unset <key>   -> config.game_defaults
gcl launch <slug> [--account <id|name>] [--offline-user <name>] [--dry-run]
gcl debug verify-source fabric|quilt|forge|neoforge     -> live list for 1.20.1 and profile/installer metadata parse; PASS/FAIL
```
`--dry-run` prints the program, then one arg per line (text) or the `LaunchCommand` JSON. Without `--dry-run` the CLI streams `Event::Log` lines to stdout and exits with the game's exit code.

`scripts/e2e.sh`: keep the step order; make the "add a mod" step conditional: `if $GCL content --help >/dev/null 2>&1; then ... else echo "SKIP content add (plan 3)"; fi`. The dry-run step uses `--offline-user e2e-tester --dry-run`. The classpath check parses the printed args (one per line: find the `-cp` line and take the next line, split on `:`).

- [ ] **Step 1: Tests** (`tests/cli.rs`, wiremock for Mojang + Fabric via the env base URLs): `loader list 1.20.1 --loader fabric --json` shows 3 versions; `account add-offline alice` then `account list --json` has one active; `settings set demo renderDistance 16` then `settings show demo --json` reflects it; `launch demo --offline-user bob --dry-run` (instance created with loader none over the synthetic vanilla) prints `--username` and `bob`; `launch nope --dry-run` exits 1.
- [ ] **Step 2: Implement, `just check`, then run `just e2e` live** (network; Fabric 1.20.1 install and vanilla assets: several hundred MB into a temp root; acceptable once). Expected: PASS lines for create, loader, SKIP content, dry-run, classpath. Paste the tail in the report.
- [ ] **Step 3: Commit** `feat(cli): loader, account, settings, launch commands; e2e passes`

---

### Task 10: Docs

- [ ] Update `ARCHITECTURE.md` (loaders, settings, auth, launch rows become real with actual dependencies; note `LoaderCtx.runner` test seam and the four loader base-URL env overrides), `.claude/skills/modloaders/SKILL.md` (match the implemented contract: `LoaderEndpoints`, `version_id`, `ProcessRunner`, output-skip rule), `.claude/skills/testing/SKILL.md` (env overrides list, FakeRunner pattern), `.claude/skills/instance-model/SKILL.md` (settings module owns options.txt now), `.claude/skills/msa-auth/SKILL.md` (store layout as implemented). Run `just check && just deny && just lint-claude && just verify-api fabric && just verify-api neoforge`. Commit `docs: loaders, settings, auth, launch`.
