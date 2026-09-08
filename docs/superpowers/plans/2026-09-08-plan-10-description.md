# Plan 10: description rendering, changelog notes, browser rows

Date: 2026-09-08. Branch: `plan-10-description`. HEAD: f5a7d93. Status: not started.

**For agentic workers:** use superpowers:subagent-driven-development or
superpowers:executing-plans to run this plan task by task. Checkboxes track progress.

## Summary

Descriptions render their real block structure — tables, images, rules, quotes, nested lists,
`<details>`, and a bold-only line as a subheading — instead of dropping images and tables.
Each version row in the Versions tab gets a Notes button that opens that version's changelog in
a modal. Browser rows show the full title (wrapped, not elided), drop the URL line, size to
their content, and collapse line breaks in the short description. Spec:
`docs/superpowers/specs/2026-09-08-description-rendering-design.md` (approved). Research:
`docs/research/2026-09-08-description-markup.md`. The user chose "show all images" over an
https-only rule with no host allowlist for description images — icons keep their CDN allowlist;
description images do not.

## Global constraints

- No `unwrap`/`expect` outside tests and build scripts (`thiserror` in `gcl-core`, `anyhow`
  only in binaries, per `AGENTS.md`).
- Unit tests never touch the network; fixtures under `tests/fixtures/` with wiremock.
- Never log or print a CurseForge key or a token.
- Every new interactive Slint element carries a stable `<thing>_<kind>` id
  (`snake_case := `) with `accessible-role: button`, `accessible-label`, and
  `accessible-action-default` when it is a clickable non-`Button`/`ListRow` element; run
  `scripts/list-slint-ids.sh` and refresh `docs/research/2026-09-07-ui-element-ids.md` after
  adding one.
- Every new `*State` property starts empty (`""`, `0`, `false`, `[]`); the screen's `open`/`load`
  path sets every property it owns, including the empty case. Sample data lives only in that
  screen's `Preview*` component, never in a global default.
- Test hosts a description image or a changelog fixture points at are `.invalid` or the
  wiremock URI; never a real CDN, per the `testing` skill's "every endpoint is mocked or
  unreachable" rule.
- Description images are fetched and decoded off the UI thread (`Bridge::run`), same as icons
  today; only the final `slint::Image` / `SharedPixelBuffer` is built on the UI thread.
- No core call runs on the UI thread; every screen callback goes through `Bridge::run` or
  `Bridge::run_with_error`.
- `just check` (fmt, clippy, nextest) passes at the end of every task. Paste its last lines in
  that task's report.
- Every commit carries `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Design seams settled here

- **Slint `Block` table shape: flat `cells`/`columns`, not nested.** Slint structs in this
  codebase avoid nested arrays wherever a flat shape prints fine (`ContentVersionRow` joins
  `game_versions`/`loaders` into one string rather than passing arrays). A table is the one
  block that needs real 2-D data, so `Block` gains `columns: int` and `cells: [string]`,
  row-major, header included as row 0: `cells[row * columns + col]`. The renderer computes
  `rows = cells.length / columns` and paints row 0 bold. This avoids a `[[string]]` field
  (Slint supports nested array properties but nothing else in `types.slint` uses one, and nested
  models are awkward to build from a `VecModel` on the Rust side) and keeps `Block` a plain
  value type. `Block` therefore ends up with five fields total: `kind`, `text`, `depth` (bullet
  nesting, 0 elsewhere), `url` (image src, empty elsewhere), `columns`/`cells` (table, empty
  elsewhere) — the same "one struct, several kinds only some fields fill in" shape
  `SettingRowModel` already uses.
- **`image::Limits` for the 4096×4096 decode cap.** `image` 0.25's `ImageReader` takes a
  `Limits` value: `image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?` then
  `reader.limits(limits)` before `reader.decode()`, where `limits` is `image::Limits::default()`
  with `max_image_width = Some(4096)` and `max_image_height = Some(4096)` set on the struct's
  public fields (not a builder method — `Limits` is a plain struct in `image::io` re-exported as
  `image::Limits`). A description-image decode goes through this path; `decode_icon` in
  `crates/gcl-ui/src/models/mod.rs` (`image::load_from_memory`, no limits) stays as it is for
  icons, which are already capped at 2 MiB on the wire and never came from an arbitrary host.
  Pull the "bytes to RGBA + dimensions, with limits" step into a second pure function
  (`decode_description_image` or an optional `limits` parameter on a shared helper) next to
  `decode_icon`, so it is unit-tested the same way, per the `testing` skill's "pure helper
  first" rule.
- **Generalizing the icon cache into an image cache.** `crates/gcl-core/src/download/icons.rs`
  (`IconCache`, `ALLOWED_HOSTS`, `MAX_BYTES` = 2 MiB, `cache/icons/<sha1(url)>.<ext>`) stays
  exactly as it is — icons keep their CDN allowlist. A new sibling,
  `crates/gcl-core/src/download/images.rs`, holds an `ImageCache` (or a shared, policy-parameterized
  type both modules call into — coder's call, but keep `icons.rs`'s public shape unchanged either
  way) for description images: same single-flight-per-URL map and same
  stage-to-`.part`-then-`store_object` download path as `icons.rs`, but no host allowlist (`https://`
  only, checked the same way `icons.rs::scheme_allowed` does, with an `extra_hosts` test seam for
  the scheme exception only — there is no host list to extend) and a 5 MiB cap
  (`MAX_BYTES = 5 * 1024 * 1024`). Extract the shared "stream to `.part`, check the cap, move to
  `dest`" body into one function both modules call, rather than duplicating it, since the only
  difference between the two paths is which cap and which host rule applies. `Root` gains
  `images_dir()` next to `icons_dir()` in `crates/gcl-core/src/paths/mod.rs`
  (`self.cache_dir().join("images")`), and `Launcher` gains an `images: ImageCache` field beside
  `icons: IconCache`, following `fetch_icon`'s exact shape for `fetch_image`. A test-only
  `with_image_hosts` (mirroring `with_icon_hosts`) lets a wiremock host be plain HTTP for the
  scheme exception in tests; the shipped launcher passes no override, so a description image
  loads from any `https://` host and nothing else.
- **Where `Source::changelog` fetches from at Modrinth.** The version *list* request
  (`Modrinth::fetch_versions`) explicitly sends `include_changelog=false` to keep the list
  payload small — that stays as it is. `Source::changelog(project_id, version_id)` instead calls
  `GET /version/{version_id}` (the single-version endpoint `Modrinth::version` already uses),
  which is not filtered by that flag and, per the research doc, carries `changelog` on Sodium
  live. Add `changelog: Option<String>` to `RawVersion` (`#[serde(default)]`, since the *list*
  responses this same struct parses will carry `null` or omit the field) and to `Version` in
  `types.rs`; `map_version` copies it across. Factor `Modrinth::version`'s body into a small
  inherent helper (`version_raw` or similar) that both the trait's `version()` and the new
  `changelog()` call, so the GET happens once per code path rather than duplicated. CurseForge's
  `changelog()` is a new endpoint, `GET /v1/mods/{modId}/files/{fileId}/changelog`, parsed the
  same `Envelope<String>` shape `description()` already uses (`{"data": "<html>"}`) — VERIFY,
  same unverified-envelope caveat `description()` carries today, no `CURSEFORGE_API_KEY` on this
  machine.
- **Where the block renderer lives so both the Description tab and the Notes modal use it.**
  Pull the `for block in ... blocks` `VerticalLayout` out of `ProjectScreen`'s Description tab
  in `ui/screens/project.slint` into its own component (`BlockList` in a new
  `ui/components/block-list.slint`, taking `blocks: [Block]` as an `in` property). `ProjectScreen`
  and the new `NotesDialog` both instantiate it. This is the only way to add table/rule/image/
  quote rendering once instead of twice.
- **The Notes modal is a dialog, not a screen tab.** Per the `slint-ui` skill's dialog rules,
  every dialog lives in `app.slint` behind `if <State>.xxx_open:`, mounted only while open, so it
  takes the keyboard correctly and Escape closes it through `Dialog`'s own `FocusScope` — the
  same reason `CreateInstanceDialog` and the others are not inline in their screens.
  `NotesDialog` wraps `Dialog` (from `components/dialog.slint`) with a `ScrollView` of
  `BlockList` as its `@children`, title `"<number> notes"`, and only a Close button
  (`confirm-text` left empty). `ProjectState` gains `notes_open: bool`, `notes_title: string`,
  `notes_blocks: [Block]`, `notes_loading: bool`; a `version_notes_button` per version row (next
  to `version_install_button`) calls `ProjectState.open_notes(version_id, number)`; `Dialog`'s
  `close()` wires to `ProjectState.close_notes()`.
- **Image loading inside a description uses a per-open generation counter, the same pattern
  `browser.rs::fetch_icons`/`icon_generation` already uses for search-row icons.** `ProjectState`
  (or `screens/project.rs`'s private `Shared`) gets an `AtomicU64` bumped once per `open()` (and
  once per `open_notes()`, separately, since the two lists load independently); the image-fetch
  job started after blocks land checks the counter before writing pixels back, so navigating away
  mid-fetch (open another project, or close the Notes modal and open a different version's) never
  paints a stale image over a different project's blocks. Cap at 20 images per open, per the
  design doc, by filtering `Block::Image` entries to the first 20 before dispatching the fetch job
  — the rest keep the `[Image: alt]` failure/placeholder text forever, which is an acceptable
  degrade for a pathological description, not a bug to chase further.

## Files

| File | Change | Task |
|---|---|---|
| `crates/gcl-core/src/sources/richtext.rs` | `Block` gains `Table`, `Rule`, `Image`, `Quote`, `Bullet { depth, text }`; markdown/HTML converters for each; limits updated (50 rows/8 cols/200 chars per cell) | 1 |
| `crates/gcl-core/src/sources/richtext/tests.rs` (or `mod tests` inline) | converter cases: pipe table (ImmediatelyFast), badge drop (existing, re-verify), `<details>`/`<summary>`, bold-only heading, rule, quote, nested bullet depth, image | 1 |
| `crates/gcl-core/tests/fixtures/modrinth/*.json` | any new fixture text needed for the above (or inline strings in unit tests, matching the existing `richtext` test style) | 1 |
| `crates/gcl-core/src/paths/mod.rs` | `Root::images_dir()` | 2 |
| `crates/gcl-core/src/download/images.rs` (new) | `ImageCache`: any-https, 5 MiB cap, `cache/images/<sha1(url)>.<ext>`, single-flight, `with_image_hosts`-style test seam | 2 |
| `crates/gcl-core/src/download/icons.rs` | factor the shared stream/cap/move body out so `images.rs` reuses it, if that is the chosen shape | 2 |
| `crates/gcl-core/src/download/mod.rs` | register the `images` module | 2 |
| `crates/gcl-core/src/sources/types.rs` | `Version.changelog: Option<String>` | 2 |
| `crates/gcl-core/src/sources/mod.rs` | `Source::changelog(project_id, version_id) -> Result<String, Error>`, no default body | 2 |
| `crates/gcl-core/src/sources/modrinth.rs` | `RawVersion.changelog`, `map_version` copies it, `changelog()` trait impl via the single-version GET | 2 |
| `crates/gcl-core/src/sources/curseforge.rs` | `changelog()` trait impl: `GET /v1/mods/{id}/files/{fileId}/changelog`, `Envelope<String>` | 2 |
| `crates/gcl-core/tests/fixtures/curseforge/get_file_changelog.json` (new, synthetic) | changelog fixture, per the `mod-sources` skill's synthetic-fixture convention | 2 |
| `crates/gcl-core/src/launcher/mod.rs` | `fetch_image(url) -> PathBuf`, `with_image_hosts`, `version_notes(source, project_id, version_id) -> Vec<Block>` | 2 |
| `crates/gcl-core/tests/*` or a `FakeSource` used by `content`/`modpacks` tests | add a `changelog` stub to any test `Source` impl the trait change breaks | 2 |
| `crates/gcl-ui/ui/types.slint` | `Block` gains `depth: int`, `url: string`, `columns: int`, `cells: [string]` | 3 |
| `crates/gcl-ui/ui/components/block-list.slint` (new) | `BlockList` component: table (bold header row, horizontally scrollable), rule, quote (left bar), bullets by `depth`, image (placeholder/loading/`[Image: alt]` failure), reused by both callers | 3 |
| `crates/gcl-ui/ui/screens/project.slint` | Description tab uses `BlockList`; `version_notes_button` per version row; `ProjectState` gains `notes_open`/`notes_title`/`notes_blocks`/`notes_loading` and `open_notes`/`close_notes` callbacks | 3, 4 |
| `crates/gcl-ui/ui/app.slint` | mount `if ProjectState.notes_open: NotesDialog { }` | 4 |
| `crates/gcl-ui/ui/components/dialog.slint` or a new `ui/components/notes-dialog.slint` | `NotesDialog` wrapping `Dialog` with a `ScrollView` of `BlockList` | 4 |
| `crates/gcl-ui/src/models/mod.rs` | `block_row` handles the five new kinds; `decode_icon`/a sibling `decode_description_image` applies `image::Limits`; `search_row` collapses `\n`/`\r` in the description | 3, 5 |
| `crates/gcl-ui/src/screens/project.rs` | `block_row` mapping extended; `open_notes`/`close_notes` wiring, `version_notes` job, image-fetch job with its own generation counter, per-open image cap of 20 | 3, 4 |
| `crates/gcl-ui/ui/screens/browser.slint` | row title `wrap: word-wrap`, no elide, no width cap; URL `Text` removed; row height follows content | 5 |
| `crates/gcl-ui/Cargo.toml`, root `Cargo.toml` | no change expected — `image` is already a dependency with the needed features from plan 9 | 5 |
| `crates/gcl-ui/tests/flow_project.rs` | table block assertion, an image block served by a mock PNG, Notes modal open/close against a changelog fixture | 6 |
| `crates/gcl-ui/tests/flow_content.rs` | a long title is not elided; a description with a newline renders as one wrapped paragraph | 6 |
| `crates/gcl-ui/tests/support/mod.rs` | fixture/mocks updates the above two flows need (a table/image in the recorded project body, a changelog endpoint mock, `.invalid`-safe image host) | 6 |
| `docs/SPEC.md`, `ARCHITECTURE.md`, `.claude/skills/{mod-sources,download-cache,slint-ui,testing}/SKILL.md`, `CHANGELOG.md`, `docs/research/2026-09-07-ui-element-ids.md` | docs | 7 |
| `scripts/ui-xtest.py`, `just ui-xtest` recipe | a step opening a details page with a table, real X input | 7 |

## Tasks

### Task 1 — richtext: table, rule, image, quote, bullet depth (coder)

Files: `crates/gcl-core/src/sources/richtext.rs`, its test module, any new fixture text.

- `Block` enum gains: `Table { header: Vec<String>, rows: Vec<Vec<String>> }`, `Rule`,
  `Image { url: String, alt: String }`, `Quote(String)`; `Bullet(String)` becomes
  `Bullet { depth: u8, text: String }`. Update `Block::text()`/`text_mut()` for the new
  variants (a `Table`, `Rule`, or `Image` has no single "text" the way the others do — decide in
  this task how `text()` answers for them, e.g. `Rule` and `Table` return `""`, `Image` returns
  `alt`, and document the choice on the method).
- Markdown: pipe tables (a `|---|---|` alignment row marks the header/body split — the row
  `rule_line` already treats as a dropped separator becomes the *signal* that the row above it
  was the header, not just noise to drop); `---`/`***` on their own line (already partly handled
  by `rule_line`, which currently treats a bare `---` as "drop it" — now it becomes `Block::Rule`
  instead of being dropped, except still after a text line, which is a heading, per the existing
  setext rule); `> ` quotes, one level (currently `unquote` strips every `>` — narrow that so a
  clean `> text` line with no other structure becomes `Block::Quote`, but keep stripping `>` from
  everything else so a nested or mid-paragraph `>` does not regress the existing paragraph tests);
  list depth from two leading spaces (or one tab) per level, carried as `Bullet.depth`; `![alt](url)`
  becomes `Block::Image` instead of being dropped by `inline()`; `<img>` in HTML-in-markdown becomes
  `Block::Image` the same way, instead of being stripped by `strip_tags`; a badge
  (`[![alt](image)](url)`) is still dropped whole, unchanged; `<details>`/`<summary>` become
  `Heading(3, summary_text)` followed by the section's own blocks (not flattened into one
  paragraph); a paragraph that is entirely one `**bold**` run (no other text) becomes
  `Heading(4, text)` instead of a stripped-emphasis `Paragraph`.
- HTML: `<table>` (was `SKIPPED` and dropped whole) now builds a `Block::Table` — first `<tr>`
  with `<th>` cells, or the first `<tr>` if none has `<th>`, is the header; `<hr>` becomes
  `Block::Rule`; `<blockquote>` (was flushed as a paragraph like `<p>`) becomes `Block::Quote`;
  `<ul>`/`<ol>` nesting depth feeds `Bullet.depth` (currently every `<li>` is depth-less); `<img
  src alt>` (was dropped) becomes `Block::Image`.
- Limits: 2 000 blocks and 8 KiB per block stay; add 50 rows and 8 columns per table (extra rows/
  columns dropped, not erroring), and cells cut at 200 characters (with the same `…` marker
  `truncate_text` uses, reused rather than duplicated).
- Update the `mod-sources` skill's "Rich text" section (deferred to Task 7, but note here that
  the "images, tables ... are dropped with their contents" line in both `ARCHITECTURE.md` and
  the skill becomes false and needs updating there, not here).
- Tests: one markdown case and one HTML case per feature (table incl. the ImmediatelyFast
  fixture from the research doc, rule, quote, nested bullet two levels deep, `<details>`, a
  bold-only paragraph, an image with and without alt text); confirm the existing badge-drop test
  still passes unchanged; a table over 50 rows or 8 columns is truncated, not rejected; a cell
  over 200 characters ends in `…`.
- Command: `just test 'package(gcl-core) and test(richtext)'` then `just check`.
- Commit: `feat(core): tables, images, rules, quotes, and nested bullets in descriptions`.

### Task 2 — core: image cache, changelog field, `Source::changelog`, `Launcher::fetch_image`/`version_notes` (coder)

Files: `crates/gcl-core/src/paths/mod.rs`, `crates/gcl-core/src/download/images.rs` (new),
`crates/gcl-core/src/download/icons.rs`, `crates/gcl-core/src/download/mod.rs`,
`crates/gcl-core/src/sources/types.rs`, `crates/gcl-core/src/sources/mod.rs`,
`crates/gcl-core/src/sources/modrinth.rs`, `crates/gcl-core/src/sources/curseforge.rs`,
`crates/gcl-core/tests/fixtures/curseforge/get_file_changelog.json` (new),
`crates/gcl-core/src/launcher/mod.rs`, any test `Source` impl the trait change breaks.

- `Root::images_dir()` next to `icons_dir()`.
- `download::images::ImageCache`: same single-flight-per-URL shape as `IconCache::fetch`, no
  host allowlist (any `https://` host, per the approved spec choice — "show all images"), 5 MiB
  cap (`MAX_BYTES = 5 * 1024 * 1024`), `cache/images/<sha1(url)>.<ext>` via the same
  `cache_file_name`/`extension_of` helpers `icons.rs` already has (reuse them — they take no
  host-specific behavior — rather than copy them verbatim; make them `pub(crate)` in `icons.rs`
  if they need to cross the module boundary, or lift them into a small shared file both modules
  import). An `extra_hosts`-style test seam covers only the scheme exception (plain HTTP for a
  wiremock host in tests), since there is no host list to extend.
- `Source::changelog(&self, project_id: &str, version_id: &str) -> Result<String, Error>` on the
  trait, no default body.
- `Version.changelog: Option<String>` in `types.rs`. `RawVersion.changelog: Option<String>`
  (`#[serde(default)]`) in `modrinth.rs`; `map_version` copies it across. Factor `Modrinth::version`'s
  GET into an inherent helper both `version()` and the new `changelog()` call, so `changelog()`
  does not duplicate the request-and-parse logic; `changelog()` returns
  `raw.changelog.unwrap_or_default()`.
- `CurseForge::changelog`: `GET /v1/mods/{project_id}/files/{version_id}/changelog`,
  `Envelope<String>` (the same wrapper `description()` uses), same "VERIFY: no
  `CURSEFORGE_API_KEY` on this machine" caveat and synthetic fixture as `description()`'s.
- `Launcher::fetch_image(&self, url: &str) -> Result<PathBuf, crate::Error>`: mirrors
  `fetch_icon` exactly (`self.images.fetch(&http, &root, url, &hosts)` under `block_on`).
  `with_image_hosts(mut self, hosts: Vec<String>) -> Self`: test seam, mirrors
  `with_icon_hosts`.
- `Launcher::version_notes(&self, source: SourceId, project_id: &str, version_id: &str) ->
  Result<Vec<Block>, crate::Error>`: calls `source.changelog(project_id, version_id)`, then
  `richtext::from_markdown` (Modrinth) or `from_html` (CurseForge) — the same source-id dispatch
  `project_details` already does.
- Any test `Source` implementation (a `FakeSource` in `content`/`modpacks` tests) gets a
  `changelog` stub, per the `testing` skill's `FakeSource` pattern; grep for `impl Source for`
  to find every implementor the trait change breaks, including any test-only ones.
- Tests: `fetch_image_allows_any_https_host` (a wiremock host the icon allowlist would refuse,
  proving no allowlist applies here), `fetch_image_refuses_a_non_https_url` (same shape as
  `icons.rs`'s scheme test, minus the host-allowlist test that has no image equivalent),
  `fetch_image_caps_at_five_mib`, `fetch_image_is_single_flight`, `fetch_image_reuses_the_cached_file`;
  `modrinth_changelog_returns_the_stored_string` (wiremock `GET /version/{id}` fixture carrying
  `changelog`), `curseforge_changelog_returns_the_html_from_the_data_envelope`; a
  `version_notes` test per source confirming it dispatches to the right converter.
- Command: `just check`.
- Commit: `feat(core): image cache, version changelogs, and version notes`.

### Task 3 — UI: `Block` shape, `BlockList` renderer, description tab wiring (slint-designer)

Files: `crates/gcl-ui/ui/types.slint`, `crates/gcl-ui/ui/components/block-list.slint` (new),
`crates/gcl-ui/ui/screens/project.slint`, `crates/gcl-ui/src/models/mod.rs`,
`crates/gcl-ui/src/screens/project.rs`.

- `types.slint`: `Block` gains `depth: int` (bullet nesting, 0 elsewhere), `url: string` (image
  source, empty elsewhere), `columns: int` and `cells: [string]` (table, empty elsewhere — see
  "Design seams" above for the flat, row-major shape and why). Document each field's meaning per
  `kind` in the struct's doc comment, the way `SettingRowModel`'s comment already explains which
  fields a given `control` value uses.
- `ui/components/block-list.slint`: `BlockList` component, `in property <[Block]> blocks`.
  Renders: `heading1`..`heading6` and `paragraph`/`bullet` (indented by `depth * Theme.gap` or
  similar, bulleted with `•`) as today; `code` as today; `table` as a bold header row (row 0 of
  `cells`) over `columns` body rows on a `Theme.surface-raised` tile, horizontally scrollable
  (a `ScrollView` or a `Flickable` wrapping a fixed-width `GridLayout`) when the content is wider
  than the column; `rule` as a 1px `Theme.border`-colored line; `quote` as an indented
  `Theme.text-muted` paragraph with a left bar (a thin `Theme.accent`- or `Theme.border`-colored
  `Rectangle`); `image` as an `Image` element sized to the column width with `block.url`'s alt
  text below it, a `Theme.surface-raised` placeholder tile while `row.icon`-equivalent state says
  "loading" (see Task 4's per-block image state), and literal text `"[Image: " + block.text +
  "]"` when the fetch or decode failed. Since `BlockList` itself has no Rust-side state to track
  per-image load status, decide in this task how a "loading" vs "failed" vs "loaded" image is
  told apart on screen — the simplest shape is `Block.url` carrying the *decoded* image only once
  it is ready (a per-project map of `url -> slint::Image` built in Task 4, looked up by
  `screens/project.rs` when it builds the `Vec<Block>` for display, not by `BlockList` itself) so
  `BlockList` stays pure layout with no knowledge of fetch state; document that choice here so
  Task 4 does not re-decide it.
- `ui/screens/project.slint`: the Description tab's inline block-rendering `VerticalLayout` is
  replaced with one `BlockList { blocks: ProjectState.blocks; }`.
- `src/models/mod.rs`: `block_row` (or wherever the `CoreBlock -> Block` mapping lives) handles
  `Table`, `Rule`, `Image`, `Quote`, and `Bullet { depth, text }`, filling the new fields and
  leaving the others at their type default. Add a pure decode helper for a description image
  under the 4096×4096 `image::Limits` cap (see "Design seams" above for the exact API shape),
  next to `decode_icon`, tested the same way with known PNG bytes plus a check that an
  oversized image (larger than the limit) is refused rather than decoded.
- `search_row` (also in `models/mod.rs`, unrelated to blocks but touched again in Task 5) is left
  alone here; Task 5 owns it.
- Tests: `block_row` maps every new `CoreBlock` variant correctly (a table's flattened
  `cells`/`columns`, a bullet's `depth`, an image's `url`/`text`); the new decode helper accepts
  a normal PNG and refuses one wider than 4096px (construct a small oversized synthetic image in
  the test, or assert on the `image::error::LimitError` kind rather than decoding a real
  multi-megabyte fixture).
- Command: `just check`; `slint-viewer --check --style fluent
  ui/components/block-list.slint` and `ui/screens/project.slint`; a manual `just ui-preview
  screens/project.slint` look with a table and an image added to `PreviewProjectScreen`'s sample
  blocks, described in the report.
- Commit: `feat(ui): table, image, rule, and quote rendering in project descriptions`.

### Task 4 — UI: description images off the UI thread, Notes modal (slint-designer)

Files: `crates/gcl-ui/ui/screens/project.slint`, `crates/gcl-ui/ui/app.slint`,
`crates/gcl-ui/ui/components/notes-dialog.slint` (new, or added to `dialog.slint`),
`crates/gcl-ui/src/screens/project.rs`.

- `screens/project.rs`: after a project's blocks land (the existing `apply` function, once
  `state.set_blocks(...)` has run), collect up to 20 `Block::Image` URLs from
  `opened.details.blocks`, bump a per-open image-fetch generation counter (an `AtomicU64` beside
  the existing pattern in `browser.rs`'s `Shared`/`icon_generation` — add an equivalent field to
  whatever shared state this module already threads through `Bridge`, or a new small `Shared`
  struct if `project.rs` has none yet), and start one `Bridge::run` job that fetches
  (`Launcher::fetch_image`) and decodes (Task 3's new helper) each URL in turn, the same
  sequential-in-one-job shape `browser.rs::fetch_icons` uses. The `done` closure checks the
  generation counter before writing anything back (a stale fetch from a project the user has
  since navigated away from is dropped, not painted), then rebuilds `ProjectState.blocks` with
  each matching `Image` block's decoded pixels attached — per Task 3's design note, this can be a
  side map (`url -> slint::Image`) merged into the `Block` list on the UI thread, or the `Block`
  list rebuilt with `url` replaced by a data-URL-free marker plus a lookup; pick whichever is
  less code and document the choice in a comment, since Task 3 deliberately left this open for
  the implementer closest to the fetch code.
- `ProjectState` (in `project.slint`): `notes_open: bool`, `notes_title: string`, `notes_blocks:
  [Block]`, `notes_loading: bool`; callbacks `open_notes(string /* version_id */, string /*
  number */)` and `close_notes()`.
- `version_notes_button` added to each version row in the Versions tab, next to
  `version_install_button`, calling `ProjectState.open_notes(row.id, row.number)`.
- `ui/components/notes-dialog.slint` (or inline in `dialog.slint` next to the other named
  dialogs): `NotesDialog` wraps `Dialog` (`open: ProjectState.notes_open`, `title: "" +
  ProjectState.notes_title + " notes"`, `close-text: "Close"`, no `confirm-text`) with a
  `ScrollView` containing `BlockList { blocks: ProjectState.notes_blocks; }` as its `@children`,
  plus a loading line the same shape as `ProjectScreen`'s own `ProjectState.loading` text. `Dialog`'s
  `close()` wires to `ProjectState.close_notes()`.
- `app.slint`: `if ProjectState.notes_open: NotesDialog { }`, mounted alongside the app's other
  dialogs, per the "a dialog is mounted only while open" rule.
- `src/screens/project.rs`: `on_open_notes` sets `notes_open = true`, `notes_loading = true`,
  `notes_title = number`, `notes_blocks = []`, runs `Launcher::version_notes(source, project_id,
  version_id)` through `Bridge::run_with_error`, and on success maps the blocks the same way
  `apply` does for the description tab (reusing `block_row`), including the same up-to-20-images
  fetch-and-generation pattern as the main description — a changelog can carry images too, and
  nothing in the design excludes that. `on_close_notes` sets `notes_open = false`.
- Tests: none new here beyond compiling; real assertions land in Task 6's flow test. Manual
  `just ui-preview screens/project.slint` check with the Notes dialog opened via
  `PreviewProjectScreen`'s `init`, described in the report.
- Command: `just check`.
- Commit: `feat(ui): version notes modal with off-thread image loading`.

### Task 5 — UI: browser row polish (slint-designer)

Files: `crates/gcl-ui/ui/screens/browser.slint`, `crates/gcl-ui/src/models/mod.rs`.

- `browser.slint`: the search-row title (`row_title`/`title_text`) drops its
  `width: min(title_text.preferred-width, Theme.label-width)` cap and its `overflow: elide`,
  gaining `wrap: word-wrap` instead, so a long title wraps rather than truncating. The `row.page_url`
  `Text` at the bottom of each content-search row is removed. `row_open`'s fixed
  `height: 3 * Theme.font-size + 3 * Theme.gap + Theme.pad` becomes the row's own
  `preferred-height` (or an equivalent content-driven height — `ListRow`/`Rectangle` sizing may
  need `height: layout.preferred-height` where `layout` is the row's inner `VerticalLayout`,
  named for the purpose), so a wrapped multi-line title does not clip. Apply the same three
  changes (title wrap, URL removal, content height) to the modpack results rows
  (`pack_row`/`ListRow` for-loop) too, for consistency, unless a modpack row's fixed three-line
  layout is deliberately left alone — note whichever call is made and why in the commit.
- `src/models/mod.rs`: `search_row` replaces every `\n` and `\r` in `h.description` with a space
  before building `SearchRow.description`, so a short description with embedded line breaks
  renders as one line the `Text`'s own `word-wrap` reflows, rather than as literal blank lines
  Slint's `Text` would otherwise show.
- Tests: `search_row_collapses_newlines_in_the_description` (a description fixture with `\n` and
  `\r\n` in it, asserting neither survives in the built row).
- Command: `just check`.
- Commit: `feat(ui): browser rows wrap the full title and drop the url line`.

### Task 6 — GUI flow tests and fixes (coder + slint-designer, then api-verifier)

Files: `crates/gcl-ui/tests/flow_project.rs`, `crates/gcl-ui/tests/flow_content.rs`,
`crates/gcl-ui/tests/support/mod.rs`, any fixes the flows find in
`crates/gcl-ui/src/screens/{project,browser}.rs` or `crates/gcl-core/src/{sources,download,launcher}/*`.

- `support/mod.rs`: extend the recorded Modrinth project body (`MOD_BODY` or a new constant) used
  by `Mocks::project()` with a small pipe table and an `![alt](url)` pointed at the mock's own
  icon-serving path (or a second small PNG endpoint), so `flow_project` can assert a real table
  block and a real decoded image block land on screen. Add a changelog mock (`GET
  /version/{id}` already exists for the single-version path if `version_notes` reuses it, or a
  dedicated endpoint if Task 2's Modrinth `changelog()` fetches separately) serving a short
  changelog string with at least one heading and one bullet, for the Notes modal assertion.
- `flow_project.rs`: after the existing description assertion (`a_row_title_opens_the_description`),
  assert the blocks list contains a `table` block with the right `columns`/`cells`, and an
  `image` block whose decoded pixel width is nonzero once the async fetch lands (`wait_until`,
  same pattern as `first_icon_width`). Add a new sub-flow: open the Versions tab, click a version
  row's `version_notes_button`, wait for `ProjectState.notes_open` and `!notes_loading`, assert
  `notes_blocks` carries the mocked changelog's heading and bullet, click `Dialog::cancel_button`
  (or whatever id `NotesDialog`'s Close button carries — confirm against
  `docs/research/2026-09-07-ui-element-ids.md` once Task 4 lands), assert `notes_open` is false
  again.
- `flow_content.rs`: assert a long title (add one to a fixture, or reuse an existing long-titled
  project if one already exists in a recorded fixture) is not elided in the installed content
  list or the browser row — read the row's rendered height or its full text via the element
  tree, whichever the harness can observe without pixel comparison — and a description carrying
  `\n` renders with no literal blank line in the row (assert on `SearchRow.description` having no
  `\n` left, via whatever the flow can read off the built model, similar to how other flows
  assert on state rather than pixels).
- Fix whatever these flows find broken in Tasks 1–5 before moving on, the same pattern earlier
  plans used (build the feature, then let the flow test find the wiring bugs).
- Run `just check`.
- **api-verifier**: Task 2 changed `crates/gcl-core/src/sources/*` and `download/*`. Run `just
  verify-api modrinth` live for the version-list and single-version endpoints (confirming
  `changelog` really comes back on `GET /version/{id}` the way the research doc's live check on
  Sodium found) and report PASS/FAIL. CurseForge's `changelog` endpoint stays an unverified
  envelope shape on this machine (no `CURSEFORGE_API_KEY`) — note that explicitly, the same way
  Plan 9 carried its `description` VERIFY forward rather than letting it go quiet.
- A FAIL from api-verifier goes back to coder with the report, then api-verifier runs again, per
  `CLAUDE.md`'s delegation rules.
- Commit: `test(ui): description rendering, notes modal, and browser row flows; fixes found along the way`.

### Task 7 — docs, ids, ui-xtest, live verification (doc-research drafts, coder/slint-designer edits, then a live run)

Files: `docs/SPEC.md`, `ARCHITECTURE.md`, `.claude/skills/mod-sources/SKILL.md`,
`.claude/skills/download-cache/SKILL.md`, `.claude/skills/slint-ui/SKILL.md`,
`.claude/skills/testing/SKILL.md`, `CHANGELOG.md`, `docs/research/2026-09-07-ui-element-ids.md`,
`scripts/ui-xtest.py`.

- `docs/SPEC.md`: status rows for the richer description rendering, the changelog/Notes modal,
  and the browser row changes, under the nearest existing requirement or as a short addendum,
  following the file's existing status-table format.
- `ARCHITECTURE.md`: update the `sources` row (richtext's new block kinds, `Source::changelog`),
  the `download` row (the new `download::images` cache alongside `download::icons`, no host
  allowlist, 5 MiB cap), and the `launcher` row (`fetch_image`, `version_notes`) — the current
  text explicitly says "images and tables dropped" for `richtext` and lists only `fetch_icon`/
  `project_details`/`project_versions` for `launcher`; both need correcting, not just appending
  to. Update the `gcl-ui` structure block for `ui/components/block-list.slint` and the notes
  dialog file.
- `.claude/skills/mod-sources/SKILL.md`: the "Rich text (`sources::richtext`)" section's opening
  line ("images, tables ... are dropped with their contents") is now false — rewrite it to
  describe the new block kinds, and add the `Source::changelog` endpoint shapes (including the
  CurseForge envelope VERIFY note) next to the existing `description` documentation.
- `.claude/skills/download-cache/SKILL.md`: add a "Description images (`download::images`)"
  section mirroring the existing "Icons" section, calling out the one real difference (no host
  allowlist, 5 MiB not 2 MiB) so a future reader does not assume the two caches share a policy.
- `.claude/skills/slint-ui/SKILL.md`: document `BlockList` as the shared renderer two callers use
  (Description tab, Notes modal), and the per-open image-generation-counter pattern as the
  second worked example next to `browser.rs`'s icon one.
- `.claude/skills/testing/SKILL.md`: a one-line pointer to the extended `flow_project`/
  `flow_content` coverage.
- `CHANGELOG.md`: an `Unreleased` bullet for table/image/rule/quote rendering, the Notes modal,
  and the browser row changes.
- `scripts/list-slint-ids.sh` re-run, `docs/research/2026-09-07-ui-element-ids.md` refreshed with
  `version_notes_button` and whatever id `NotesDialog`'s close button carries.
- `just ui-xtest`: extend the existing browse leg (or add a short third leg) with a step that
  opens a details page whose description carries a table, so this feature gets the same
  real-pointer check earlier plans added for the rest of the app; confirm the app's GUI log
  proves the step happened, per the existing pattern (grep the quoted job label).
- Live run: `just verify-api modrinth` (repeated here if Task 6's run is stale), then `just check
  && just deny && just lint-claude && just ui-xtest`.
- Commit: `docs: description rendering, changelog notes, and browser row polish`.

## Verification

- api-verifier: required after Task 2, since `sources/` and `download/` changed. `just
  verify-api modrinth` for the single-version `changelog` field; CurseForge's `changelog`
  endpoint stays a documented VERIFY gap, the same as its `description` endpoint today.
- e2e-runner: not required — this plan touches no `instances/`, `launch/`, `modpacks/`,
  `settings/`, or `gcl-cli` behavior. Run `just e2e` once anyway at the end of Task 7 as a cheap
  regression check, since it is fast and already in the standard command set.

## Risks

- **CurseForge's `changelog` response envelope is unverified**, the same gap `description`
  already carries. Mitigation: Task 2's fixture is explicitly synthetic per the `mod-sources`
  skill's convention, and Task 7 carries the VERIFY note into the skill and `docs/SPEC.md` rather
  than letting it go silent, exactly as Plan 9 did for `description`.
- **No host allowlist for description images is a deliberate, user-approved widening of what
  this launcher fetches automatically.** A description can now cause an https request to any
  host a project's author put an `<img>`/`![]()` at. Mitigation: the spec is explicit that this
  was chosen over an allowlist; the 5 MiB cap and https-only rule are the only guards, both
  already in Task 2, and this risk is called out again in the skill docs Task 7 updates so a
  future reader does not assume the icon allowlist's protection extends here.
- **A table wider than the screen needs horizontal scrolling inside a renderer (`BlockList`)
  that also has to sit inside the Description tab's own vertical `ScrollView`.** Nested
  scrollables are a known Slint rough edge. Mitigation: Task 3 scopes the horizontal scroll to
  the table's own tile (a `Flickable`/`ScrollView` sized to the tile, not the whole page), and
  the manual `ui-preview` check in that task's report specifically looks at the ImmediatelyFast
  table example from the research doc, which is exactly this shape.
- **Per-open image-fetch generation counters add a second piece of "is this fetch still wanted"
  state (Description tab, Notes modal) on top of `browser.rs`'s existing one for search-row
  icons.** Mitigation: Task 4 copies the same pattern deliberately rather than inventing a new
  one, and the flow test in Task 6 exercises opening one project, then another, while an image
  fetch is plausibly still in flight (the mock's PNG response can carry a short `set_delay` the
  same way `IconCache`'s single-flight test does) to catch a stale write.
- **`Block`'s five-field-for-all-kinds Slint shape (most fields empty for most kinds) could grow
  awkward if a future block kind needs yet another field.** Mitigation: this is the same shape
  `SettingRowModel` already uses successfully for a similarly varied set of row kinds, so it is a
  known-acceptable trade-off in this codebase rather than a new risk being introduced.

## Open questions

1. **Where a stale-vs-loading-vs-failed description image's state lives** — a per-project map
   built in `screens/project.rs` and merged into `Block.url` on the UI thread, vs. the `Block`
   list itself being rebuilt with decoded pixels attached some other way — is deliberately left
   for Task 4's implementer to pick, per the note in "Design seams" and Task 3's description.
   Whichever is chosen should be written once as a code comment so Task 6's flow test and any
   later reader are not left to reverse-engineer it.
2. **Whether the icon cache and the new image cache should share one generic type
   (policy-parameterized) or stay two small, near-duplicate modules** is left to Task 2's coder.
   The plan asks only that the shared "stream to `.part`, cap-check, move to `dest`" body not be
   copy-pasted twice; the public shape of `IconCache` must not change, since `browser.rs`'s
   existing icon flow and its flow-test assertions depend on it exactly as it is today.
3. **CurseForge `GET /mods/{id}/files/{fileId}/changelog` response envelope** is assumed to match
   every other CurseForge endpoint this codebase parses (`{"data": "<html>"}`), the same
   assumption `description()` already carries and has not yet had confirmed live. Task 2 ships
   the fixture as synthetic and flags it; Task 7 carries the note forward. A human with a
   `CURSEFORGE_API_KEY` running `just record-fixture curseforge get_file_changelog '<url>'`
   against a real file id is the only way to close this out, same as the existing `description`
   gap.
