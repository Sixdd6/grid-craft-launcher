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
    async fn changelog(&self, project_id: &str, version_id: &str) -> Result<String, Error>;  // no default body
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

`SearchHit.updated` is when the project last changed, RFC 3339 exactly as the source wrote it
(Modrinth `date_modified`, CurseForge `dateModified`), and `""` when the response carried no
date; nothing parses or normalises it in `gcl-core`.

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
- Release notes: `Source::changelog` GETs `/version/{version_id}` and returns its `changelog`
  (markdown), which `include_changelog=false` on the *list* endpoint does not filter.
  `Modrinth::version_raw` is the shared inherent helper behind both `version` and `changelog`,
  so one call is one GET. `Version.changelog: Option<String>` carries the same field whenever a
  response sent it; a listed version leaves it `None`. A version with no notes answers an empty
  string, not an error.
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
- Release notes: `GET /v1/mods/{modId}/files/{fileId}/changelog`, HTML, parsed from the same
  `Envelope<String>` `description` uses. **VERIFY: the envelope is assumed to be
  `{"data": "<html string>"}`** — no `CURSEFORGE_API_KEY` on this machine, so
  `tests/fixtures/curseforge/get_file_changelog.json` is synthetic and unconfirmed against a
  live response. A non-numeric mod id or file id is `Error::NotFound`. `Version.changelog` is
  always `None` here: CurseForge never sends notes with a file.
- Loader ids (`modLoaderType`): Forge 1, Fabric 4, Quilt 5, NeoForge 6.
- Files: `GET /v1/mods/{id}/files?gameVersion=&modLoaderType=&pageSize=50&index=0`.
  `downloadUrl` may be null. `CurseForge::mod_files` and `CurseForge::pack_files` both call
  this; `pack_files` skips the loader filter, since a pack states its loader in
  `manifest.json`, not in a file's `gameVersions`.
- Newest file per target: a search hit's `latestFilesIndexes` maps onto
  `SearchHit.latest_files: Vec<LatestFileIndex { game_version, loader: Option<u32>, file_id }>`,
  so a caller can name the newest file for a Minecraft version and loader without a second
  request. `loader` is the `modLoaderType` id (Forge 1, Fabric 4, Quilt 5, NeoForge 6), `None`
  on an entry with no `modLoader`. Modrinth has no comparable field, so a Modrinth hit's
  `latest_files` is always empty and the caller lists versions instead. **VERIFY: the entry
  shape (`gameVersion`, `modLoader`, `fileId`) is taken from the public docs; no
  `CURSEFORGE_API_KEY` on this machine, so the `latestFilesIndexes` array in
  `tests/fixtures/curseforge/search_mods.json` was written by hand and is unconfirmed against a
  live response.** An entry the index lacks means a fallback to the files call, never an error.
- Batch: `POST /v1/mods/files { fileIds }` (`CurseForge::files_batch`) and
  `POST /v1/mods { modIds }` (`CurseForge::mods_batch`), both chunked by `PAGE_SIZE`. An id the
  server does not know is dropped, not an error.
- Fingerprint: `POST /v1/fingerprints { fingerprints }` (`CurseForge::resolve_by_fingerprint`),
  chunked the same way. Only `exactMatches` count; a partial or unmatched fingerprint is
  dropped.
- `CurseForge::resolve_pack_id(id_or_slug)` resolves a modpack's id or slug to its numeric mod
  id, for `modpacks::fetch_pack` — `Source::project` refuses a modpack outright, since a modpack
  is not a `ContentKind`.

## Latest version and install state (`Launcher`)

`latest_versions(source, hits, target)` and `latest_versions_each(source, hits, target,
on_answer)` are the same lookup; the second calls `on_answer` as each hit resolves, under the
same four-in-flight semaphore and the same in-memory cache, and the first is a wrapper that
collects the answers back into hit order. The rules:

- **"Latest" is release-first**: `content::pick_latest` sorts by release channel and then by
  publish time, so a newer beta never outranks an older release. It is the version Add would
  install.
- **A mod on a vanilla target has no latest.** A `VersionTarget` with a Minecraft version and
  `Loader::None` answers `version: None` for a `ContentKind::Mod` hit and asks for nothing: a
  vanilla instance runs no mods. `index_file_id` follows the same rule and takes a
  loader-less `latestFilesIndexes` entry only.
- **The CurseForge index is a fast path, not an answer.** The file it names is fetched with
  one `POST /v1/mods/files` and cached under its own key (`index:<file_id>`), never under the
  listing's key, so a listing key always holds a whole listing. When `pick_latest` rejects
  that file — its `gameVersions` names no loader, say — the listing is fetched as usual.

`install_state(slug, source, project_id, target, latest)` reads those cached lists.
`Older` when `latest` was published after the installed version, `Installed` when the publish
times are equal. An installed CurseForge id the listing does not hold is resolved with one
`files_batch(&[id])` (cached under the same `index:` key), so `Older.installed_number` is a
version number and never a raw file id. A pack-imported entry, recorded under the source
`file`, is matched by `content::entry_holding_file` against the latest file and against every
version in the cached list, so a pack carrying an older build reads as `Older`. A failed
lookup answers `InstallState::Unknown` — the UI shows "Checking…" and then leaves the line
blank — while a source that cannot be built at all (no CurseForge key) is still an error.

## Rich text (`sources::richtext`)

`Block::{Heading(level, text), Paragraph(text), Bullet { depth, text }, Code(text),
Table { header, rows }, Rule, Image { url, alt }, Quote(text)}`, built by `from_markdown`
(Modrinth) or `from_html` (CurseForge). Both are display converters, not parsers: headings,
paragraphs, list items with their nesting depth, code blocks, tables, rules, quotes, images,
and links (`text (url)`) survive; `<script>` and `<style>` are dropped with their contents;
every other tag or marker is stripped and its text kept. No dependency — the markdown side is
a line scanner, the HTML side a tag-aware stripper.

The module is five files: `richtext/mod.rs` (`Block` and the output caps), `markdown.rs` (the
line scanner and the tag stripper that feeds it), `inline.rs` (links, images, emphasis),
`html.rs` (`HtmlState`, the block builder), and `tokenize.rs` (tags, attributes, entities).
`from_markdown` and `from_html` are re-exported from `mod.rs`, so every path a caller uses is
`sources::richtext::…` as before. Each file keeps its own tests beside it.

`Block::text()` answers the block's own text: `""` for `Rule` and `Table` (a table's strings
are its `header` and `rows`), the alt text for `Image`.

A real Modrinth `body` is markdown with HTML in it: `<center><img>`, `<details>`, `<summary>`,
badge tables, `<br>`. The rules, all covered by tests over the recorded bodies of Sodium,
Create, JEI, ImmediatelyFast (a pipe table), and Mod Menu (`<details>`, a blockquote)
(`tests/fixtures/modrinth/project_body_*.json`):

- **Tags in markdown are rewritten as markdown** with the same tokenizer `from_html` uses,
  outside code fences only, so a fenced `<config>` survives. `<img>` becomes `![alt](src)`,
  `<summary>` becomes a `###` heading line, `<hr>` becomes a `***` rule line, `<li>` becomes
  `- ` indented by its `<ul>`/`<ol>` nesting, a block-level tag becomes a line break, and
  entities are decoded. A `<script>`, `<style>`, or `<table>` subtree goes whole — a markdown
  body's HTML tables are badge layout, and its real tables are written with pipes.
- **A body that is block-level HTML** (`<p>`, `<ul>`, `<h1..6>` in it) **with no markdown
  structure at all** — no `#` heading, list marker, or fence — is handed to `from_html` whole.
  A mixed body stays on the markdown path.
- **Ordered lists** (`1. `, `1) `) become `Bullet`, since a `Block` carries no numbering.
  **Bullet depth** comes from a stack of the indent columns seen so far in the list, not from
  a fixed step: a deeper indent than the top of the stack pushes one level whatever its width,
  so a list written with four spaces per step nests one level per step and not two. A tab
  counts as two columns. Depth is capped at 6, and the stack is cleared by any block that ends
  the list.
- **Pipe tables**: a row whose next line is an alignment row (`|---|---|`) is the header, and
  body rows run until the first line that is not a pipe row. A row is a pipe row when it holds
  an unescaped `|`: neither outer pipes nor spaces around them are needed, so `a|b` is a row,
  and a `\|` stays text inside the cell it sits in. What keeps a sentence with a pipe in it
  from becoming a table is the alignment row underneath. An alignment row no table row precedes
  is dropped.
- **Setext headings**: a `===` or `---` underline after a text line is a `Heading` (1 or 2).
  On its own, `===`, `---`, `***`, or `___` is a `Rule`.
- **A one-level `> ` line** is a `Quote`, and a run of them joins into one block, which a blank
  line ends. An image on a quote line is kept and emitted after the quote, the way a paragraph's
  images are. A nested `>>` is not a quote: `unquote` strips its markers and the text joins the
  paragraph, as before.
- **A paragraph that is one `**bold**` run and nothing else** becomes `Heading(4, text)`.
- **Two `<br>` in a row end the paragraph**; one is a space.
- **A badge** — `[![alt](image)](url)` — is dropped whole, image and all, like an `<a>` with
  no text. A plain `![alt](url)` becomes an `Image` block emitted after the paragraph or
  bullet whose line carried it. An `<img src>` rewritten into markdown has its `(` and `)`
  percent-encoded, since either would end the link early when the scanner reads the line back.
- **Entities**: numeric (`&#8217;`, `&#x2019;`) plus `nbsp lt gt quot apos amp mdash ndash
  hellip rsquo lsquo ldquo rdquo middot copy trade reg`. Anything else stays as written.
- **`</a>` appends ` (href)` only when the anchor produced text**, so an image-only link keeps
  no URL. `attr()` reads name/value pairs with quotes respected, so a `title="href=nope"` is
  not mistaken for the `href`.
- **A self-closing `<script/>`, `<style/>`, or `<table/>` opens no subtree**: `parse_tag`
  reports `self_closing`, so it cannot swallow the rest of the document.
- **On the HTML path**, `<table>` builds a `Block::Table` — the first `<tr>` with `<th>` cells
  is the header, else the first row, and rows past the cap are dropped as each `<tr>` closes
  rather than at the end, so a document with a hundred thousand rows costs bounded memory —
  `<hr>` a `Rule`, `<blockquote>` a `Quote` (prose inside
  one flushes as a quote, however many `<p>` it holds), `<summary>` a level-3 heading, and
  `<img src alt>` an `Image`. `<ul>`/`<ol>` nesting sets each `<li>`'s depth. An `<img>` inside
  a table cell is ignored, so a badge grid does not spray images into the middle of a table.
- **Bounded work and bounded output**: a link label is looked for within 512 characters and no
  scan starts past the last `]`, so a line of unmatched `[` stays linear. At most 2 000 blocks
  and 8 KiB per block, the `…` spent out of that budget rather than added past it; a longer
  block ends in `…`, and a longer body ends with one
  `Paragraph("…")`. A table holds at most 50 body rows and 8 columns, extra ones dropped rather
  than refused, and a cell is cut at 200 characters with the same `…` marker.

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
