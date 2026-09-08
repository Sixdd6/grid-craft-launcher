---
name: mod-sources
description: Modrinth and CurseForge API usage — endpoints, headers, rate limits, content types, install targets, null download URLs, fingerprints, and the Source trait. Read before touching gcl-core sources/.
---

Full research: `docs/research/2026-09-06-modrinth-and-curseforge-apis.md`. Both VERIFY items
about live behavior are resolved below. The CurseForge shader and data pack class ids stay
unverified: this machine has no `CURSEFORGE_API_KEY`.

## Source trait

```rust
#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;                                   // Modrinth | CurseForge
    fn supported_kinds(&self) -> &[ContentKind];                // Mod, ResourcePack, Shader, DataPack, World
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, Error>;
    async fn search_packs(&self, q: &SearchQuery) -> Result<SearchPage, Error>;  // modpacks; default: UnsupportedPacks
    async fn project(&self, id_or_slug: &str) -> Result<Project, Error>;
    async fn description(&self, project_id: &str) -> Result<String, Error>;  // no default body
    async fn versions(&self, project_id: &str, f: &VersionFilter) -> Result<Vec<Version>, Error>;
    async fn version(&self, version_id: &str) -> Result<Version, Error>;
    async fn resolve_by_hash(&self, sha1: &[String]) -> Result<Vec<Version>, Error>;
    async fn resolve_by_fingerprint(&self, fps: &[u32]) -> Result<Vec<Version>, Error>;
    fn as_modrinth(&self) -> Option<&modrinth::Modrinth> { None }
    fn as_curseforge(&self) -> Option<&curseforge::CurseForge> { None }
}
pub type BoxSource = std::sync::Arc<dyn Source>;
```

`search_packs` is the modpack search: a modpack is not a `ContentKind`, so it has its own
call. Modrinth sends the facet `[["project_type:modpack"]]` plus `[["versions:<mc>"]]` when
the query names a Minecraft version; CurseForge sends `classId=<modpacks>`. Neither sends a
loader filter: a pack states its loader in its index, not in its categories. Every hit has
`SearchHit.is_pack = true`, reports `ContentKind::Mod` as a placeholder kind, and links
through `pack_page_url`. Hits from `search` always have `is_pack = false`.

Modrinth answers `resolve_by_fingerprint` with `Ok(vec![])` (no fingerprint endpoint);
CurseForge answers `resolve_by_hash` the same way (no hash endpoint). `as_modrinth` and
`as_curseforge` exist because a modpack is not a `ContentKind`: `modpacks` needs each
client's pack-only endpoints, which are not on the trait.

`Version` carries `files: Vec<VersionFile> { url: Option<String>, file_name, size: Option<u64>,
sha1: Option<String>, sha512: Option<String>, fingerprint: Option<u32>, primary }` and
`dependencies: Vec<Dependency> { project_id: Option<String>, version_id: Option<String>, kind:
Required | Optional | Incompatible | Embedded }`.

`SearchHit.icon_url` is the project's icon, `None` when the source serves none. It is a plain
CDN URL, not a hash-addressed file, so `download::icons` caches it by URL rather than through the
object store; see the `download-cache` skill. Never point a test fixture's `icon_url` at a real
CDN: the icon allowlist accepts `cdn.modrinth.com` and the two `forgecdn.net` hosts, so a flow
test would reach the live internet and swallow the outcome.

`Modrinth::project_with_body(id_or_slug) -> (Project, String)` is an inherent method, not a
trait one: Modrinth's `GET /project/{id}` already carries the description in `body`, so
`Source::project` and `Source::description` are the same request twice. Both now call it and
throw half the answer away. A caller that wants both — `Launcher::project_details` — reaches
it through `as_modrinth()` and pays for one request per details open. CurseForge keeps the
description behind `GET /mods/{id}/description`, so it still costs two there.

Constructors: `Modrinth::new(http)` / `Modrinth::with_base_url(http, base)`,
`CurseForge::new(http, api_key)` / `CurseForge::with_base_url(http, api_key, base)`.
`Launcher::sources()` uses `new`/`with_base_url` against `Endpoints`, always with Modrinth and
with CurseForge only when a key was found; the list is cached on the `Launcher`, so a later
`config_mut()` change to the key does nothing until a new `Launcher` is opened.

## Modrinth

- Base `https://api.modrinth.com/v2` (`modrinth::BASE`). User-Agent is the shared `HttpClient`'s.
  300 requests per minute; honor `X-Ratelimit-Remaining` and `X-Ratelimit-Reset`.
- Search: `GET /search?query=&facets=<json>&index=relevance&offset=&limit=`. Facets: outer array
  AND, inner array OR.
- Versions: `GET /project/{id}/version?loaders=["fabric"]&game_versions=["1.20.1"]&include_changelog=false`.
- Hash lookup: `POST /version_files { hashes, algorithm: "sha1" }`.
- Description: no endpoint of its own. `Source::description` re-fetches `GET /project/{id}` and
  returns its `body` (markdown), which `sources::richtext::from_markdown` turns into blocks. A
  project with no body answers an empty string, not an error.
- Files have both `sha1` and `sha512`; the object store only checks sha1.
- **`world` is not a Modrinth project type.** `KINDS` in `modrinth.rs` lists `Mod`,
  `ResourcePack`, `Shader`, `DataPack` only; `ContentKind::World` is left out and `search`
  rejects it with `Error::UnsupportedKind`. Confirmed live: a `project_type:world` facet
  returns zero hits (`tests/fixtures/modrinth/search_types.json`).
- **A `project_type:datapack` search hit still reports `project_type: "mod"`.** The rule this
  launcher applies: an explicit `kind` in the `SearchQuery` always wins over the hit's own
  `project_type`; only a kindless search falls back to parsing `project_type`, and a type that
  does not map to a `ContentKind` (a modpack) drops that hit from the page.
- `Modrinth::pack_versions(project_id, minecraft)` lists a modpack's versions — same request as
  `versions`, without a loader filter, since a pack states its loader in the pack index, not in
  `loaders`.

## CurseForge

- Base `https://api.curseforge.com` (`curseforge::BASE`). Header `x-api-key`. Game id `432`
  (`curseforge::GAME_ID`). Page size max 50 (`PAGE_SIZE`), same size for the `POST /v1/mods` and
  `POST /v1/mods/files` batch endpoints.
- **Class ids are fetched at runtime, once per client, and cached in a `tokio::sync::OnceCell`**
  (`CurseForge::class_ids`, `GET /v1/categories?gameId=432&classesOnly=true`). There is no disk
  cache: a new `CurseForge` client fetches again. `mods`, `modpacks`, `resource_packs`, and
  `worlds` are required — a response missing one of the four is `Error::BadResponse`. `shaders`
  and `data_packs` are `Option<u32>`: a response without that class leaves the field `None`, and
  searching for that kind then fails with `Error::UnsupportedKind`, whatever `supported_kinds`
  lists. **This machine has no `CURSEFORGE_API_KEY`, so the shader and data pack class ids are
  unverified.** `tests/fixtures/curseforge/README.md` says why the fixtures are synthetic.
- Description: `GET /v1/mods/{id}/description`, HTML, turned into blocks by
  `sources::richtext::from_html`. **VERIFY: the response envelope is assumed to be
  `{"data": "<html string>"}`**, matching every other CurseForge endpoint this codebase parses.
  No `CURSEFORGE_API_KEY` on this machine, so `tests/fixtures/curseforge/get_mod_description.json`
  is synthetic and the shape is unconfirmed against a live response. A non-numeric project id is
  `Error::NotFound`, as with `versions`.
- Loader ids (`modLoaderType`): Forge 1, Fabric 4, Quilt 5, NeoForge 6.
- Files: `GET /v1/mods/{id}/files?gameVersion=&modLoaderType=&pageSize=50&index=0`.
  `downloadUrl` may be null. `CurseForge::mod_files` and `CurseForge::pack_files` both call
  this; `pack_files` skips the loader filter, since a pack states its loader in
  `manifest.json`, not in a file's `gameVersions`.
- Batch: `POST /v1/mods/files { fileIds }` (`CurseForge::files_batch`) and
  `POST /v1/mods { modIds }` (`CurseForge::mods_batch`), both chunked by `PAGE_SIZE`. An id the
  server does not know is dropped, not an error.
- Fingerprint: `POST /v1/fingerprints { fingerprints }` (`CurseForge::resolve_by_fingerprint`),
  chunked the same way. Only `exactMatches` count; a partial or unmatched fingerprint is
  dropped.
- `CurseForge::resolve_pack_id(id_or_slug)` resolves a modpack's id or slug to its numeric mod
  id, for `modpacks::fetch_pack` — `Source::project` refuses a modpack outright, since a modpack
  is not a `ContentKind`.

## Rich text (`sources::richtext`)

`Block::{Heading(level, text), Paragraph(text), Bullet(text), Code(text)}`, built by
`from_markdown` (Modrinth) or `from_html` (CurseForge). Both are display converters, not
parsers: headings, paragraphs, list items, code blocks, and links (`text (url)`) survive;
images, tables, `<script>`, and `<style>` are dropped with their contents; every other tag or
marker is stripped and its text kept. No dependency — the markdown side is a line scanner, the
HTML side a tag-aware stripper.

A real Modrinth `body` is markdown with HTML in it: `<center><img>`, `<details>`, `<summary>`,
badge tables, `<br>`. The rules, all covered by tests over the recorded bodies of Sodium,
Create, and JEI (`tests/fixtures/modrinth/project_body_*.json`):

- **Tags in markdown are stripped** with the same tokenizer `from_html` uses, outside code
  fences only, so a fenced `<config>` survives. `<img>` goes, a `<script>`, `<style>`, or
  `<table>` subtree goes whole, a block-level tag becomes a line break, `<li>` becomes `- `,
  and entities are decoded.
- **A body that is block-level HTML** (`<p>`, `<ul>`, `<h1..6>` in it) **with no markdown
  structure at all** — no `#` heading, list marker, or fence — is handed to `from_html` whole.
  A mixed body stays on the markdown path.
- **Ordered lists** (`1. `, `1) `) become `Bullet`, since a `Block` carries no numbering.
- **Setext headings**: a `===` or `---` underline after a text line is a `Heading` (1 or 2).
  A `---` on its own is a rule and is dropped, as is a table separator row (only `|`, `-`, and
  `:` in it). A leading `> ` is dropped.
- **Two `<br>` in a row end the paragraph**; one is a space.
- **A badge** — `[![alt](image)](url)` — is dropped whole, like an `<a>` with no text.
- **Entities**: numeric (`&#8217;`, `&#x2019;`) plus `nbsp lt gt quot apos amp mdash ndash
  hellip rsquo lsquo ldquo rdquo middot copy trade reg`. Anything else stays as written.
- **`</a>` appends ` (href)` only when the anchor produced text**, so an image-only link keeps
  no URL. `attr()` reads name/value pairs with quotes respected, so a `title="href=nope"` is
  not mistaken for the `href`.
- **A self-closing `<script/>`, `<style/>`, or `<table/>` opens no subtree**: `parse_tag`
  reports `self_closing`, so it cannot swallow the rest of the document.
- **Bounded work and bounded output**: a link label is looked for within 512 characters and no
  scan starts past the last `]`, so a line of unmatched `[` stays linear. At most 2 000 blocks
  and 8 KiB per block; a longer block ends in `…`, and a longer body ends with one
  `Paragraph("…")`.

## Fingerprint (`sources::fingerprint`)

MurmurHash2, 32-bit, the original `MurmurHash2` (not `MurmurHash2A`), seed `1`, over the file
bytes after every byte equal to 9 (tab), 10 (LF), 13 (CR), or 32 (space) is removed.
`curseforge_fingerprint(bytes)` and `fingerprint_file(path)` (blocking; async callers use
`spawn_blocking`) implement it over the `murmur2` crate. Verified against an independent
implementation — see `crates/gcl-core/src/sources/fingerprint.rs`'s pinned-vector test.

## Null download URL

A null `downloadUrl` (CurseForge) or a missing `url` (never happens on Modrinth, which always
publishes one) means the author opted out of third-party distribution. This surfaces as
`content::ManualDownload { source, project_id, version_id, file_name, page_url, fingerprint,
sha1 }`, never as a failed `add` or import. `curseforge::file_page_url(project_page_url,
file_id)` builds the per-file page; Modrinth manual downloads point at the project page, since
there is no per-file page there. The CLI and UI show the page and accept a dropped file, then
`content::import_manual` verifies it by fingerprint (CurseForge) or sha1 (Modrinth) before
storing it.

## One file per project

`content::add` installs a project at most once per instance. A dependency (`depth > 0`) whose
pin names another version than the one `instances::content::installed` finds is never
installed: the installed version stays, the walk goes on through
`queue_installed_dependencies`, and the pin lands in `AddOutcome.conflicts` as a
`DependencyConflict { project_id, title, installed_version_id, wanted_version_id, wanted_by }`.
Two jars for one project break NeoForge and Forge mod loading, which is the whole reason.

A pack file that no source id could be found for is recorded under the source `file` with
its sha1 for a project id, so `installed` cannot match it. `add` therefore checks the
picked *file* as well (`entry_holding_file`): an entry whose sha1 is the picked file's
sha1, or a `file`-source entry with the picked file's name, counts as installed. That
project is skipped, its dependencies are still walked, and a pin of another version lands
in `conflicts` the same way.
The CLI prints one `warning: kept ...` line per conflict and puts them in `--json`; the GUI
browser raises one toast (`screens::browser::conflict_note`).

Only the top-level request (`depth == 0`) may change the installed version: a user pin of
another version replaces the entry in place, and `place_file` deletes the old file and its
`.disabled` twin before linking the new one.

## Install targets

| ContentKind | Directory under `.minecraft/` |
|---|---|
| Mod | `mods/` |
| ResourcePack | `resourcepacks/` |
| Shader | `shaderpacks/` |
| DataPack | `saves/<world>/datapacks/` (caller supplies the world) |
| World | `saves/` (zip extracted, one folder) |

A modpack is not a `ContentKind`; it becomes a new instance through `modpacks`, not through
this table.

## Testing against a fake source

Unit tests for `content` and `modpacks` do not need wiremock: a `FakeSource` implementing
`Source` in the test module is enough. See the `testing` skill for the pattern.

## Re-recording CurseForge fixtures

`tests/fixtures/curseforge/*.json` are synthetic, written by hand from the API docs, because
this machine has no `CURSEFORGE_API_KEY`. To replace one with a real response, put a key in
`.env` and run `just record-fixture curseforge <name> '<url>'`. Never commit a `.env` or a
fixture that still carries a live key in its URL or headers.

## Do not

- Do not build a second `reqwest::Client`. Use `HttpClient`.
- Do not send the CurseForge key anywhere but `api.curseforge.com`.
- Do not hardcode class ids as the only source; the runtime fetch wins.
- Do not construct a CDN URL to work around a null `downloadUrl`; send the user to the page.
