# MVP Plan 3: Sources, Content, Modpacks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Search and install mods, resource packs, shaders, data packs, and worlds from Modrinth and CurseForge into instances (with required dependencies, enable/disable, update checks, and a manual-download path for CurseForge files that forbid third-party distribution), and import modpacks (`.mrpack` and CurseForge zips) from either catalog or a local file into a new instance. `just e2e` runs its content step for real.

**Architecture:** A `sources` module with a `Source` trait and two clients (`modrinth`, `curseforge`) that only talk HTTP and return plain structs. `instances::content` places cached files into the right instance folder and records `ContentEntry`s. `Launcher` orchestrates: choose a compatible version, download through the cache, resolve required dependencies, install. `modpacks` parses both pack formats and drives `loaders::install` + the content path to build a new instance. No source client touches instances.

**Tech Stack:** as before, plus `murmur2` (CurseForge fingerprints), `async-trait` (already a dep), `zip` (pack archives).

**Spec:** `docs/SPEC.md` R2.5, R2.6, R7.1-R7.7, R8.1-R8.3, R12.1 (content, modpack), `ARCHITECTURE.md`, skills `mod-sources`, `modpack-formats`, `download-cache`, `instance-model`.

## Global Constraints

- Everything from plans 1 and 2 (thiserror in core, anyhow in CLI, no unwrap outside tests, workspace deps, `just check` per task, commit per task with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`, no network in tests, `paths::safe_join` for every remote path, `paths::write_atomic` for writes, `HttpClient` host-agnostic with source clients owning base URLs).
- `CURSEFORGE_API_KEY` from `Config::curseforge_api_key()`; absent → CurseForge is unavailable: `Launcher::sources()` returns only Modrinth, the CLI prints `curseforge: disabled (no CURSEFORGE_API_KEY)` when asked for it, nothing else fails. The key travels only in the `x-api-key` header to the CurseForge base URL; never logged (tracing spans skip it).
- Null CurseForge `downloadUrl` → `sources::Error::ManualDownload { page_url, file_name, fingerprint, sha1 }`; no CDN URL is ever constructed (ruling 2026-09-06).
- Install targets under `.minecraft/`: mod → `mods/`, resourcepack → `resourcepacks/`, shader → `shaderpacks/`, datapack → `saves/<world>/datapacks/` (world required), world → `saves/<name>/` (zip extracted; the zip's top-level folder becomes the save; `safe_join` on every entry). Files are hard-linked from `cache/objects/` (copy fallback).
- One `ContentEntry` per installed project per instance. Adding a newer version replaces the old file and entry. Disable renames `<file>` to `<file>.disabled`.
- Version compatibility: a candidate version must list the instance's Minecraft version; for `mod` it must also list the instance's loader (Fabric instances also accept `quilt`-only? No: Quilt instances accept `fabric` or `quilt` mods; Fabric instances accept only `fabric`; NeoForge instances accept `neoforge`, and for MC 1.20.1 also `forge`; Forge instances accept only `forge`). Resource packs, shaders, data packs, worlds ignore the loader. Prefer `release` over `beta` over `alpha`, newest first.
- Required dependencies (Modrinth `dependency_type == "required"` with a `project_id`; CurseForge `relationType == 3`) are installed recursively, depth limit 10, skipping projects already installed in the instance. Optional and embedded dependencies are ignored.
- Modrinth `User-Agent` = `gcl_core::USER_AGENT`; rate-limit headers honored by `HttpClient` (429 backoff exists).
- Fixtures: `tests/fixtures/modrinth/*` are live recordings; `tests/fixtures/curseforge/*` are SYNTHETIC (built from the public docs, no key available) and the README there says so. `api-verifier` re-records them when a key exists.
- CLI: text default, `--json` everywhere; exit 1 on error, 2 for not implemented.

---

## File map

| Path | Responsibility | Task |
|---|---|---|
| `tests/fixtures/{modrinth,curseforge}/*` | recorded and synthetic responses | 1 |
| `crates/gcl-core/src/sources/{mod,types}.rs` | `Source` trait, shared types, errors | 2 |
| `crates/gcl-core/src/sources/modrinth.rs` | Modrinth client | 3 |
| `crates/gcl-core/src/sources/curseforge.rs`, `sources/fingerprint.rs` | CurseForge client, murmur2 | 4 |
| `crates/gcl-core/src/instances/content.rs` | placing files, entries, enable/disable, remove | 5 |
| `crates/gcl-core/src/content/mod.rs` | version selection, dependency resolution, add/update/import-manual | 6 |
| `crates/gcl-core/src/modpacks/{mod,mrpack,curseforge}.rs` | pack detection, parsing, import | 7 |
| `crates/gcl-core/src/launcher/mod.rs` | `sources()`, `search`, `add_content`, ..., `import_modpack*` | 8 |
| `crates/gcl-cli/src/commands/{content,modpack,debug}.rs`, `scripts/e2e.sh` | CLI, e2e | 9 |
| `ARCHITECTURE.md`, skills, `docs/research` | docs | 10 |

---

### Task 1: Commit fixtures

**Files:** `tests/fixtures/modrinth/*.json`, `tests/fixtures/curseforge/*.json`, `tests/fixtures/curseforge/README.md`

- [ ] **Step 1:** The controller pre-staged these files (uncommitted). Check they exist and are valid JSON: `for f in tests/fixtures/modrinth/*.json tests/fixtures/curseforge/*.json; do python3 -m json.tool "$f" >/dev/null || echo BAD $f; done`. Any missing Modrinth file: record it with `just record-fixture modrinth <name> '<url>'` using the URLs in the mod-sources skill. If `search_types.json` says `world` is not a project type, note it for Task 3.
- [ ] **Step 2:** Every file under 100 KB (trim arrays to 3 entries if not). Commit `test: modrinth recordings and synthetic curseforge fixtures`.

---

### Task 2: sources types and trait

**Files:** `crates/gcl-core/src/sources/mod.rs`, `crates/gcl-core/src/sources/types.rs`

**Interfaces:**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all = "lowercase")]
pub enum SourceId { Modrinth, CurseForge }
impl SourceId { pub fn parse(s: &str) -> Option<Self>; } // "modrinth" | "curseforge"; Display lowercase
pub use crate::instances::model::ContentKind;   // Mod, ResourcePack, Shader, DataPack, World
#[derive(Debug, Clone, Default)] pub struct SearchQuery { pub text: String, pub kind: Option<ContentKind>, pub minecraft: Option<String>, pub loader: Option<Loader>, pub offset: u32, pub limit: u32 /* default 20 */ }
#[derive(Debug, Clone, Serialize)] pub struct SearchHit { pub source: SourceId, pub project_id: String, pub slug: String, pub title: String, pub description: String, pub author: String, pub kind: ContentKind, pub downloads: u64, pub icon_url: Option<String>, pub page_url: String }
#[derive(Debug, Clone, Serialize)] pub struct SearchPage { pub hits: Vec<SearchHit>, pub total: u64, pub offset: u32 }
#[derive(Debug, Clone, Serialize)] pub struct Project { pub source: SourceId, pub id: String, pub slug: String, pub title: String, pub description: String, pub kind: ContentKind, pub page_url: String }
#[derive(Debug, Clone, Serialize)] pub struct Version { pub source: SourceId, pub project_id: String, pub id: String, pub name: String, pub number: String, pub kind: ReleaseKind, pub game_versions: Vec<String>, pub loaders: Vec<String> /* lowercase: fabric, quilt, forge, neoforge, minecraft, datapack, iris, optifine... */, pub published: String, pub files: Vec<VersionFile>, pub dependencies: Vec<Dependency> }
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)] pub enum ReleaseKind { Alpha, Beta, Release }
#[derive(Debug, Clone, Serialize)] pub struct VersionFile { pub url: Option<String>, pub file_name: String, pub size: Option<u64>, pub sha1: Option<String>, pub sha512: Option<String>, pub fingerprint: Option<u32>, pub primary: bool }
#[derive(Debug, Clone, Serialize)] pub struct Dependency { pub project_id: Option<String>, pub version_id: Option<String>, pub kind: DependencyKind }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)] pub enum DependencyKind { Required, Optional, Incompatible, Embedded }
#[derive(Debug, Clone)] pub struct VersionFilter { pub minecraft: Option<String>, pub loaders: Vec<String> }
#[async_trait] pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;
    fn supported_kinds(&self) -> &[ContentKind];
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, Error>;
    async fn project(&self, id_or_slug: &str) -> Result<Project, Error>;
    async fn versions(&self, project_id: &str, f: &VersionFilter) -> Result<Vec<Version>, Error>;
    async fn version(&self, version_id: &str) -> Result<Version, Error>;
    async fn resolve_by_hash(&self, sha1: &[String]) -> Result<Vec<Version>, Error>;   // CurseForge: unsupported → Ok(vec![]) (fingerprint path is separate)
    async fn resolve_by_fingerprint(&self, fps: &[u32]) -> Result<Vec<Version>, Error>; // Modrinth: Ok(vec![])
}
pub type BoxSource = std::sync::Arc<dyn Source>;
#[derive(Debug, thiserror::Error)] pub enum Error { Http(#[from] http::Error), #[error("{source}: project {id} not found")] NotFound { source: SourceId, id: String }, #[error("{source} does not support {kind:?}")] UnsupportedKind { source: SourceId, kind: ContentKind }, #[error("{source} is disabled: {reason}")] Disabled { source: SourceId, reason: String }, #[error("file {file_name} must be downloaded by hand from {page_url}")] ManualDownload { page_url: String, file_name: String, fingerprint: Option<u32>, sha1: Option<String> }, #[error("{source}: unexpected response for {what}: {detail}")] BadResponse { source: SourceId, what: &'static str, detail: String }, Io { path, source }, Json { path, source } }
pub fn page_url(source: SourceId, kind: ContentKind, slug: &str) -> String; // modrinth: https://modrinth.com/<mod|modpack|resourcepack|shader|datapack|world>/<slug>; curseforge: https://www.curseforge.com/minecraft/<mc-mods|modpacks|texture-packs|shaders|data-packs|worlds>/<slug>
```

- [ ] **Step 1: Tests**: `SourceId::parse` round-trip; `page_url` for every kind on both sources; `ReleaseKind` ordering (`Release > Beta > Alpha`).
- [ ] **Step 2: Implement; add `Sources(#[from] sources::Error)` to `error.rs`; commit** `feat(core): source trait and types`

---

### Task 3: Modrinth client

**Files:** `crates/gcl-core/src/sources/modrinth.rs`

**Interfaces:**
```rust
pub const BASE: &str = "https://api.modrinth.com/v2";
pub struct Modrinth { http: HttpClient, base: String }
impl Modrinth { pub fn new(http: HttpClient) -> Self; pub fn with_base_url(http: HttpClient, base: String) -> Self; }
impl Source for Modrinth { ... }
// search: GET {base}/search?query=<text>&facets=<json>&limit=<n>&offset=<n>&index=relevance ; facets = [["project_type:<t>"]] + [["versions:<mc>"]] + [["categories:<loader>"]] (loader facet only when kind == Mod); project_type mapping: Mod→mod, ResourcePack→resourcepack, Shader→shader, DataPack→datapack, World→world (if search_types.json says world is unsupported, `supported_kinds` omits World and `search` with World → UnsupportedKind)
// project: GET {base}/project/<id_or_slug>; 404 → NotFound
// versions: GET {base}/project/<id>/version?game_versions=["<mc>"]&loaders=[...]&include_changelog=false — pass loaders only when non-empty; map version_type release|beta|alpha; files: hashes.sha1/sha512, url, filename, size, primary
// version: GET {base}/version/<id>
// resolve_by_hash: POST {base}/version_files { hashes, algorithm: "sha1" } → map values
// dependencies: dependency_type required|optional|incompatible|embedded with project_id/version_id
```

- [ ] **Step 1: Tests** with wiremock + fixtures: `search` query string contains the encoded facets for kind Mod + mc + loader; parses `search_sodium.json` into hits with `page_url`; `project("sodium")`; `versions` over `versions_sodium_1.20.1_fabric.json` gives files with sha1 and a required dependency (if the fixture has one; else assert `dependencies` parses as empty); `resolve_by_hash` over `version_files_lookup.json`; 404 → `NotFound`; `search_types.json` decides the `World` case (test `supported_kinds`).
- [ ] **Step 2: Implement, commit** `feat(core): modrinth source client`

---

### Task 4: CurseForge client and fingerprint

**Files:** `crates/gcl-core/src/sources/curseforge.rs`, `crates/gcl-core/src/sources/fingerprint.rs`

**Interfaces:**
```rust
// fingerprint.rs
pub fn curseforge_fingerprint(bytes: &[u8]) -> u32;   // strip bytes 9,10,13,32; murmur2 (32-bit, the original, seed 1) via the `murmur2` crate (check its API: `murmur2::murmur2(&data, 1)`); test vector: compute for a fixed byte string and pin it; also assert whitespace stripping changes the result
pub fn fingerprint_file(path: &Path) -> std::io::Result<u32>; // blocking; callers spawn_blocking
// curseforge.rs
pub const BASE: &str = "https://api.curseforge.com";
pub const GAME_ID: u32 = 432;
pub struct CurseForge { http: HttpClient, base: String, api_key: String, classes: tokio::sync::OnceCell<ClassIds> }
#[derive(Serialize, Deserialize, Clone, Debug)] pub struct ClassIds { pub mods: u32, pub modpacks: u32, pub resource_packs: u32, pub worlds: u32, pub shaders: Option<u32>, pub data_packs: Option<u32> }
impl CurseForge {
    pub fn new(http: HttpClient, api_key: String) -> Self; pub fn with_base_url(http, api_key, base) -> Self;
    pub async fn class_ids(&self) -> Result<ClassIds, Error>;  // GET {base}/v1/categories?gameId=432&classesOnly=true → by slug: mc-mods|mods→mods, modpacks, texture-packs|resource-packs→resource_packs, worlds, shaders, data-packs; missing shaders/data-packs → None; cached in OnceCell (no disk cache in MVP)
    pub async fn mod_files(&self, mod_id: u32, f: &VersionFilter) -> Result<Vec<Version>, Error>;
    pub async fn files_batch(&self, file_ids: &[u32]) -> Result<Vec<Version>, Error>;   // POST /v1/mods/files in chunks of 50
    pub async fn mods_batch(&self, mod_ids: &[u32]) -> Result<Vec<Project>, Error>;     // POST /v1/mods
}
impl Source for CurseForge { ... }
// headers: x-api-key on every request (get_json_with_headers; add post_json_with_headers to HttpClient in this task)
// search: GET /v1/mods/search?gameId=432&classId=<id>&searchFilter=<text>&gameVersion=<mc>&modLoaderType=<1|4|5|6 when kind==Mod>&pageSize=<limit>&index=<offset>&sortField=2&sortOrder=desc
// project: GET /v1/mods/<id> (numeric only; a slug → search with slug filter `slug=<s>` and take the exact match; else NotFound)
// versions: mod_files with gameVersion + modLoaderType (mods only); each file → Version { id: file id, number: displayName, kind from releaseType 1 release 2 beta 3 alpha, game_versions from gameVersions (strings that look like versions), loaders from gameVersions lowercase entries that are loader names, files: [VersionFile { url: downloadUrl (None when null), sha1 from hashes algo 1, fingerprint: fileFingerprint, size: fileLength, primary: true }], dependencies from relationType (3 Required, 2 Optional, 5 Incompatible, 1 Embedded; others ignored) with project_id = modId }
// version(id): GET /v1/mods/files? no — file ids alone: POST /v1/mods/files { fileIds: [id] } → one
// resolve_by_fingerprint: POST /v1/fingerprints { fingerprints } → exactMatches[].file → Version; unmatched ignored
// download url: when `downloadUrl` is null the VersionFile.url is None; the content layer converts that to Error::ManualDownload with page_url = https://www.curseforge.com/minecraft/<class-slug>/<mod-slug>/files/<fileId>
// kind mapping for hits/projects: classId → ContentKind via class_ids(); unknown class → skip the hit
```

- [ ] **Step 1: Tests** with wiremock + synthetic fixtures: request carries `x-api-key` header (matcher) and never logs it (no assertion possible on logs; ensure `#[instrument(skip(self))]`); `class_ids` parses `categories_classes.json` and caches (`.expect(1)`); `search` maps class ids to kinds and builds the query; `mod_files` over `get_mod_files.json` yields one `VersionFile` with `url: None`, sha1 from algo 1, fingerprint set, a Required dependency; `files_batch` chunks (`.expect(2)` with 51 ids); `resolve_by_fingerprint` over `fingerprints.json`; `curseforge_fingerprint` vector + whitespace test.
- [ ] **Step 2: Implement, commit** `feat(core): curseforge source client and fingerprint`

---

### Task 5: instances::content (placing files)

**Files:** `crates/gcl-core/src/instances/content.rs`, modify `instances/mod.rs`

**Interfaces:**
```rust
pub fn target_dir(game_dir: &Path, kind: ContentKind, world: Option<&str>) -> Result<PathBuf, Error>; // DataPack requires world (Error::WorldRequired); safe_join on world
pub struct Placed { pub path: PathBuf, pub entry: ContentEntry }
pub fn place_file(instance: &mut Instance, object: &Path /* cache/objects/.. */, entry: ContentEntry) -> Result<Placed, Error>;
// dest = target_dir/<entry.file_name> (safe_join); if an entry with the same (source, project_id) exists: remove its old file (or .disabled twin) and replace the entry; link_or_copy; push entry; instance.save()
pub fn place_world(instance: &mut Instance, zip_path: &Path, entry: ContentEntry) -> Result<Placed, Error>; // extract under spawn_blocking? this fn is sync; caller wraps. Top-level folder(s) in the zip land under saves/; safe_join each entry; entry.world = Some(folder name)
pub fn set_enabled(instance: &mut Instance, project_id: &str, enabled: bool) -> Result<PathBuf, Error>; // rename <file> <-> <file>.disabled; update entry; save
pub fn remove(instance: &mut Instance, project_id: &str) -> Result<(), Error>; // delete file (either twin) and entry; save; NotFound
pub fn installed(instance: &Instance, source: SourceId, project_id: &str) -> Option<&ContentEntry>;
pub fn file_path(instance: &Instance, entry: &ContentEntry) -> Result<PathBuf, Error>; // current path incl. .disabled
```
Errors added to `instances::Error`: `WorldRequired`, `ContentNotFound(String)`, `Paths(#[from])`, `Zip { path, source }`.

- [ ] **Step 1: Tests** (tempdir instance): `target_dir` for every kind; `place_file` links a planted object into `mods/`, records the entry, and a second place for the same project with a new file name removes the old file; `set_enabled(false)` renames to `.disabled` and `installed()` shows `enabled == false`; `remove` deletes; `place_world` extracts a synthetic zip with `MyWorld/level.dat` into `saves/MyWorld/level.dat` and rejects `../x`; datapack without world → `WorldRequired`.
- [ ] **Step 2: Implement, commit** `feat(core): instance content placement`

---

### Task 6: content orchestration

**Files:** `crates/gcl-core/src/content/mod.rs`

**Interfaces:**
```rust
pub struct ContentCtx<'a> { pub sources: &'a [BoxSource], pub dl: &'a DownloadCtx<'a>, pub root: &'a Root, pub sink: &'a EventSink }
pub fn compatible_loaders(loader: Loader, minecraft: &str) -> Vec<&'static str>; // rules from Global Constraints
pub fn pick_version(versions: &[Version], kind: ContentKind, minecraft: &str, loader: Loader, want: Option<&str> /* version id or number */) -> Option<Version>;
pub struct AddRequest { pub source: SourceId, pub project: String /* id or slug */, pub version: Option<String>, pub kind: Option<ContentKind> /* None → from project */, pub world: Option<String> }
pub struct AddOutcome { pub installed: Vec<ContentEntry>, pub skipped: Vec<String> /* already installed project ids */, pub manual: Vec<ManualDownload> }
pub struct ManualDownload { pub project_id: String, pub version_id: String, pub file_name: String, pub page_url: String, pub fingerprint: Option<u32>, pub sha1: Option<String> }
pub async fn add(ctx: &ContentCtx<'_>, instance: &mut Instance, req: AddRequest) -> Result<AddOutcome, Error>;
// 1. source = ctx.sources.find(id) else Error::SourceUnavailable; 2. project(req.project) → kind; 3. versions(project.id, filter{ minecraft, loaders: compatible (mods only) }) → pick_version → Error::NoCompatibleVersion { project, minecraft, loader }; 4. for the primary file: if url None → push ManualDownload and continue; else DownloadSpec { url, sha1, size, dest: root.object_path(sha1)? when sha1 known else objects/tmp staging (download_one handles) , label } → download_one → place_file (World → place_world) with ContentEntry { source, project_id, version_id, file_name, sha1: computed, fingerprint (CF), kind, enabled: true, world }; 5. queue Required deps (project_id Some) not installed and not queued, depth+1 ≤ 10, same rules; 6. emit progress per file.
pub async fn check_updates(ctx: &ContentCtx<'_>, instance: &Instance) -> Result<Vec<UpdateCandidate>, Error>; // per entry: versions(project) → pick_version(None) → if id != entry.version_id → candidate { entry, new: Version }
pub async fn apply_update(ctx, instance: &mut Instance, candidate: &UpdateCandidate) -> Result<AddOutcome, Error>; // add with version = candidate.new.id
pub async fn import_manual(ctx, instance: &mut Instance, pending: &ManualDownload, file: &Path, kind: ContentKind) -> Result<ContentEntry, Error>; // verify: fingerprint (CF) or sha1 (Modrinth) must match else Error::VerificationFailed; copy into objects (sha1 computed) then place_file
```
Errors: `Sources(#[from])`, `Download(#[from])`, `Instance(#[from])`, `SourceUnavailable(SourceId)`, `NoCompatibleVersion { project, minecraft, loader }`, `VerificationFailed { file, expected, actual }`, `DependencyDepth(String)`.

- [ ] **Step 1: Tests**: `compatible_loaders` table (Fabric→[fabric]; Quilt→[quilt,fabric]; Forge→[forge]; NeoForge 1.20.1→[neoforge,forge]; NeoForge 1.21.1→[neoforge]; None→[]); `pick_version` prefers release and newest, honors `want` by id or number, filters mc and loader for mods but not for resource packs; `add` end-to-end with a `FakeSource` (implements `Source` in-memory: one project with a required dependency, files served by wiremock) over a tempdir instance: installs both, second `add` skips both; a fake with `url: None` yields one `ManualDownload` and no file; `check_updates` finds a newer version; `import_manual` with a wrong fingerprint → `VerificationFailed`, right one → placed.
- [ ] **Step 2: Implement, add `Content(#[from])` to `error.rs`, commit** `feat(core): content add, dependencies, updates, manual import`

---

### Task 7: modpacks

**Files:** `crates/gcl-core/src/modpacks/{mod,mrpack,curseforge}.rs`

**Interfaces:**
```rust
pub enum PackFormat { Mrpack, CurseForge }
pub fn detect(zip: &Path) -> Result<PackFormat, Error>;   // modrinth.index.json → Mrpack; manifest.json with manifestType minecraftModpack → CurseForge; else UnknownFormat
pub struct PackPlan { pub name: String, pub version: String, pub minecraft: String, pub loader: Loader, pub loader_version: String, pub files: Vec<PackFile>, pub overrides: Vec<String> /* zip dir prefixes in apply order */ }
pub struct PackFile { pub path: Option<String> /* mrpack: relative under .minecraft; CF: None (resolved by class) */, pub url: Option<String>, pub sha1: Option<String>, pub size: Option<u64>, pub source: Option<(SourceId, String /* project */, String /* file/version id */)>, pub required: bool }
// mrpack.rs: parse modrinth.index.json → PackPlan (dependencies map → loader: fabric-loader|quilt-loader|forge|neoforge; files: skip env.client == unsupported; downloads[0] must be on the host allowlist else Error::DisallowedHost; path must pass safe_join (no ..); overrides ["overrides/", "client-overrides/"])
// curseforge.rs: parse manifest.json → PackPlan (modLoaders primary id "<loader>-<version>"; files → source Some((CurseForge, projectID, fileID)); overrides [manifest.overrides + "/"])
pub struct ImportRequest { pub zip: PathBuf, pub name: Option<String>, pub keep_partial: bool, pub pack_source: Option<PackSource> }
pub struct ImportOutcome { pub instance: Instance, pub installed: usize, pub manual: Vec<ManualDownload> }
pub async fn import(ctx: &ContentCtx<'_>, instances: &Instances, loader_ctx: &LoaderCtx<'_>, ep: &LoaderEndpoints, game_defaults: &BTreeMap<String,String>, req: ImportRequest) -> Result<ImportOutcome, Error>;
// 1. detect+parse (spawn_blocking); 2. instances.create(name or pack name, minecraft, loader, Some(loader_version), game_defaults) with config.pack = req.pack_source; 3. loaders::install; 4. files: mrpack → DownloadSpec per file (sha1 known) → download_all → link into game_dir/<path> (safe_join) and record ContentEntry (kind from the path's first segment: mods/ resourcepacks/ shaderpacks/ else Other? — record only known kinds; unknown paths are still placed but not recorded); CF → files_batch(ids) → for each Version: primary file → download (url None → manual list) → place_file by kind (class → kind); 5. overrides: extract listed prefixes into game_dir in order (safe_join; later wins); 6. on error after step 2: delete the instance unless keep_partial; 7. events per phase.
pub async fn fetch_pack(ctx: &ContentCtx<'_>, source: SourceId, project: &str, version: Option<&str>) -> Result<(PathBuf, PackSource), Error>; // pick the pack version (kind Modpack: Modrinth project_type modpack; CF classId modpacks) → download the primary file into cache/objects → return path
```
Errors: `UnknownFormat`, `Parse { what, detail }`, `DisallowedHost(String)`, `UnsafePath(String)`, `Content(#[from])`, `Loaders(#[from])`, `Instance(#[from])`, `Download(#[from])`, `Sources(#[from])`, `Zip`, `Io`.

- [ ] **Step 1: Tests**: `detect` on synthetic zips; mrpack parse over a hand-written index (fixture-free) with one unsupported-env file skipped and a disallowed host rejected and a `..` path rejected; CF manifest parse over `tests/fixtures/curseforge/manifest.json`; `import` of a synthetic mrpack over wiremock (Fabric fixtures for the loader, a synthetic vanilla via `tests/common`, two files served) → instance exists with loader set, files placed, overrides applied (a file `overrides/config/x.toml` lands), `config.pack` recorded; failure mid-way (one file 404) → instance removed unless `keep_partial`.
- [ ] **Step 2: Implement, add `Modpacks(#[from])`, commit** `feat(core): modpack import (mrpack, curseforge)`

---

### Task 8: Launcher wiring

**Files:** modify `crates/gcl-core/src/launcher/mod.rs`

```rust
impl Launcher {
    pub fn sources(&self) -> Vec<BoxSource>;              // Modrinth always; CurseForge when config.curseforge_api_key() is Some; base URLs from Endpoints (add modrinth/curseforge fields + GCL_MODRINTH_BASE_URL / GCL_CURSEFORGE_BASE_URL)
    pub fn source(&self, id: SourceId) -> Result<BoxSource, crate::Error>;   // SourceUnavailable
    pub fn search(&self, id: SourceId, q: &SearchQuery) -> Result<SearchPage, crate::Error>;
    pub fn add_content(&self, slug: &str, req: AddRequest) -> Result<AddOutcome, crate::Error>;
    pub fn list_content(&self, slug: &str) -> Result<Vec<ContentEntry>, crate::Error>;
    pub fn remove_content(&self, slug: &str, project_id: &str) -> Result<(), crate::Error>;
    pub fn set_content_enabled(&self, slug: &str, project_id: &str, enabled: bool) -> Result<PathBuf, crate::Error>;
    pub fn check_updates(&self, slug: &str) -> Result<Vec<UpdateCandidate>, crate::Error>;
    pub fn apply_updates(&self, slug: &str, candidates: &[UpdateCandidate]) -> Result<Vec<AddOutcome>, crate::Error>;
    pub fn import_manual_file(&self, slug: &str, pending: &ManualDownload, file: &Path, kind: ContentKind) -> Result<ContentEntry, crate::Error>;
    pub fn import_modpack_file(&self, zip: &Path, name: Option<String>) -> Result<ImportOutcome, crate::Error>;
    pub fn import_modpack(&self, source: SourceId, project: &str, version: Option<&str>, name: Option<String>) -> Result<ImportOutcome, crate::Error>;
    pub fn pending_manual(&self, slug: &str) -> Result<Vec<ManualDownload>, crate::Error>; // persisted under instances/<slug>/pending-manual.json by add/import; import_manual_file removes the entry
}
```

- [ ] **Step 1: Tests** in `tests/launcher_flow.rs`: `add_content` over wiremock Modrinth fixtures (URL-rewritten to serve the jar bytes from the mock) into a Fabric instance; `sources()` without a key has one entry, with a key (set via `config_mut().keys.curseforge_api_key`) two; `pending_manual` round-trip.
- [ ] **Step 2: Implement, commit** `feat(core): launcher content and modpack orchestration`

---

### Task 9: CLI and e2e

**Files:** `crates/gcl-cli/src/commands/{content,modpack}.rs`, modify `debug.rs`, `main.rs`, `scripts/e2e.sh`, tests

```
gcl content search <text> [--source modrinth|curseforge] [--type mod|resourcepack|shader|datapack|world] [--mc <v>] [--loader <l>] [--limit n] [--offset n]
gcl content add <slug> --source <s> --project <id|slug> [--version <id|number>] [--world <name>]
gcl content list <slug>
gcl content remove <slug> <project-id>
gcl content enable|disable <slug> <project-id>
gcl content update <slug> [--apply]
gcl content pending <slug>                           -> manual downloads still needed (page urls)
gcl content import-file <slug> <path> --kind <k> [--project <id>]   -> satisfies a pending manual download (matched by file name or --project)
gcl modpack install --source <s> --project <id|slug> [--version <id>] [--name <n>]
gcl modpack install-file <path> [--name <n>]
gcl debug verify-source modrinth|curseforge          -> live: search "sodium" (mod), project, versions for 1.20.1 fabric, hash lookup; curseforge: class ids, search, files (SKIPPED when no key, exit 0 with a SKIP line)
```
Text output: `search` table (source, id, slug, title, downloads); `add` prints each installed entry and each manual download with its page URL and exits 3 when manual downloads remain (0 otherwise); `list` table (project, version, file, kind, enabled). e2e: the content step is now unconditional (`content add e2e --source modrinth --project sodium` then `content list e2e --json` must contain `sodium`). Add a second script `scripts/e2e-modpack.sh` (recipe `just e2e-modpack`): temp root, `modpack install --source modrinth --project fabulously-optimized --name e2epack` (or any small popular Fabric pack; pick one whose latest version targets a recent MC), then `launch e2epack --offline-user t --dry-run` and the classpath check.

- [ ] **Step 1: CLI tests** with wiremock Modrinth fixtures via `GCL_MODRINTH_BASE_URL`: `content search sodium --json` has a hit; `content add demo --source modrinth --project sodium` (jar served by mock) then `content list demo --json` shows it enabled; `content disable` then `list` shows `enabled: false`; `content add ... --source curseforge` without a key exits 1 with `disabled`.
- [ ] **Step 2: Implement, `just check`, then live: `just e2e` (content step real) and `just e2e-modpack`.** Fix real bugs in core if either fails; paste tails in the report.
- [ ] **Step 3: Commit** `feat(cli): content and modpack commands; e2e content step`

---

### Task 10: Docs

- [ ] Update `ARCHITECTURE.md` (`sources`, `content`, `modpacks` rows real; `instances::content`; event flow for manual downloads), `.claude/skills/mod-sources/SKILL.md` and `modpack-formats/SKILL.md` to the implemented API (class ids as fetched at runtime; the `search_types.json` answer on `world`; the CurseForge fixtures being synthetic), `testing` skill (FakeSource pattern, `GCL_MODRINTH_BASE_URL`/`GCL_CURSEFORGE_BASE_URL`, e2e-modpack), `docs/SPEC.md` (R7.2 note on which kinds each source supports as verified), README "Try it" (`content search`, `content add`, `modpack install-file`). Run `just check && just deny && just lint-claude && just verify-api modrinth`. Commit `docs: sources, content, modpacks`.
