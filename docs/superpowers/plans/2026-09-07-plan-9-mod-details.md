# Plan 9: mod details and browser polish

Date: 2026-09-07. Branch: `plan-9-mod-details`. HEAD: fc4e38e. Status: proposed.

**For agentic workers:** use superpowers:subagent-driven-development or
superpowers:executing-plans to run this plan task by task. Checkboxes track progress.

## Summary

Add a mod details screen (Description and Versions tabs) reached from a browser search row
and from the installed content list, with the ability to install a specific version over an
installed one. Show the project's title (not just its file name) on installed rows, sorted by
name, and show an icon on search rows. Spec: `docs/superpowers/specs/2026-09-07-mod-details-design.md`
(approved). Touches `R7.1` (search filters), `R7.4`/`R7.7` (install and update), `R13.1`
(screens), `R13.5` (flow tests) in `docs/SPEC.md`; Task 7 adds the new rows this feature earns.

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
- `image` crate: add with only the `png`, `webp`, `gif`, and `jpeg` features (no default
  features) to `crates/gcl-ui/Cargo.toml` and the workspace `Cargo.toml`; run `just deny` after
  adding it and fix any flagged license before the task's commit.
- Icon bytes are fetched and decoded off the UI thread (`Bridge::run`); only the final
  `slint::Image` / `SharedPixelBuffer` is built on the UI thread, per the `slint-ui` skill's
  "do not build a `ModelRc` or `Image` outside the UI thread" rule.
- No core call runs on the UI thread; every screen callback goes through `Bridge::run` or
  `Bridge::run_with_error`.
- `just check` (fmt, clippy, nextest) passes at the end of every task. Paste its last lines in
  that task's report.
- Every commit carries `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Design seams settled here

- **Navigation back-stack.** There is no back stack today: `App.on_navigate` (`src/app.rs`)
  just sets `App.screen`. Adding a real stack is more than this feature needs. Smallest thing:
  a `return_to: Screen` property on the new `ProjectState` global, set by whoever opens the
  details screen (the browser row's title click sets it to `Screen.browser`; the instance
  content row's source button sets it to `Screen.instance`, leaving `App.current_slug`
  untouched so the detail screen still shows the right instance). `ProjectState.back()` sets
  `App.screen` to that value. No other screen needs a stack, so nothing else changes.
- **`ContentEntry.title`.** Add `pub title: Option<String>` to `ContentEntry`
  (`crates/gcl-core/src/instances/model.rs`). The struct already carries `#[serde(default)]`
  at the type level, so an old `instance.toml` with no `title` key parses to `None` with no
  extra attribute needed; confirm this with a test that parses a hand-written TOML string
  missing the key. `content::install_file` sets it from `Project.title` on every new install.
  `content::check_updates` backfills it for an old entry that has none, which means its
  signature changes from `&Instance` to `&mut Instance` so it can save the backfilled title;
  `Launcher::check_updates` and its one caller update to match. Display (`models::content_row`)
  falls back to the file stem when `title` is `None`.
- **Version row installed marker.** The Versions tab flags one row "Installed" instead of
  offering its Install button. `Launcher::project_versions` takes the instance's slug, reads
  `instances::content::installed` (or the equivalent scan over `instance.config.content`) for
  the entry matching `(source, project_id)`, and marks the row whose `id` equals that entry's
  `version_id`.

## Files

| File | Change | Task |
|---|---|---|
| `crates/gcl-core/src/sources/richtext.rs` (new) | markdown/HTML to `Vec<Block>` converter | 1 |
| `crates/gcl-core/src/sources/mod.rs` | `Source::description` (default `UnsupportedKind`... no: no default, both impls provide it) | 1 |
| `crates/gcl-core/src/sources/modrinth.rs` | `description` reads `body` from the project fetch | 1 |
| `crates/gcl-core/src/sources/curseforge.rs` | `description` calls `GET /mods/{id}/description` | 1 |
| `crates/gcl-core/tests/fixtures/modrinth/*.json`, `tests/fixtures/curseforge/*.json` | description fixtures | 1 |
| `crates/gcl-core/src/instances/model.rs` | `ContentEntry.title: Option<String>` | 2 |
| `crates/gcl-core/src/content/mod.rs` | set `title` on install; `check_updates` backfills it, takes `&mut Instance` | 2 |
| `crates/gcl-core/src/launcher/mod.rs` | `project_details`, `project_versions`; update `check_updates` call site | 2 |
| `crates/gcl-cli/src/*` | update the one `check_updates` call site for the new signature | 2 |
| `crates/gcl-core/src/launcher/mod.rs` | `fetch_icon(url) -> PathBuf`, per-URL single-flight map | 3 |
| `crates/gcl-core/src/download/mod.rs` | reuse `download_one`/object store for the icon path, or a small icon-specific helper if the cache key (URL, not sha1) does not fit `DownloadSpec` cleanly | 3 |
| `crates/gcl-ui/ui/types.slint` | `Block` struct (kind, text), `ContentVersionRow` struct | 4 |
| `crates/gcl-ui/ui/app.slint` | `Screen.project` variant, mount `ProjectScreen` | 4 |
| `crates/gcl-ui/ui/screens/project.slint` (new) | `ProjectState` global, `ProjectScreen` (Description/Versions tabs), `PreviewProjectScreen` | 4 |
| `crates/gcl-ui/src/screens/project.rs` (new) | binds `ProjectState`, `project_details`/`project_versions` calls, install-a-version job | 4 |
| `crates/gcl-ui/src/lib.rs` | register `screens::project` | 4 |
| `crates/gcl-ui/src/screens/browser.rs`, `ui/screens/browser.slint` | row title click opens details with `return_to = Screen.browser` | 4 |
| `crates/gcl-ui/src/screens/instance.rs`, `ui/screens/instance.slint` | source badge becomes `row_source_button`, opens details with `return_to = Screen.instance` | 5 |
| `crates/gcl-ui/src/models/mod.rs` | `content_row` sorts by title (case-insensitive) and shows title over file name; new `content_version_row` | 5 |
| `crates/gcl-ui/ui/screens/browser.slint`, `src/screens/browser.rs` | 48px icon on a search row, placeholder tile, `Launcher::fetch_icon` on a worker thread | 5 |
| `crates/gcl-ui/Cargo.toml`, root `Cargo.toml` | add `image` (no default features; `png`, `webp`, `gif`, `jpeg`) | 5 |
| `crates/gcl-ui/tests/flow_project.rs` (new) | GUI flow: open from search, open from installed list, install a version, replace check | 6 |
| `crates/gcl-core/src/content/tests.rs`, `crates/gcl-ui/src/models/tests.rs` | unit tests for the pieces above | 1–5 |
| `docs/SPEC.md`, `ARCHITECTURE.md`, `.claude/skills/{mod-sources,slint-ui,testing}/SKILL.md`, `CHANGELOG.md`, `docs/research/2026-09-07-ui-element-ids.md` | docs | 7 |
| `scripts/ui-xtest.py`, the `just ui-xtest` recipe | a step that opens a details page with real input | 7 |

## Tasks

### Task 1 — richtext converter and description endpoint (coder)

Files: `crates/gcl-core/src/sources/richtext.rs` (new), `crates/gcl-core/src/sources/mod.rs`,
`crates/gcl-core/src/sources/modrinth.rs`, `crates/gcl-core/src/sources/curseforge.rs`,
`crates/gcl-core/tests/fixtures/modrinth/*.json`, `crates/gcl-core/tests/fixtures/curseforge/*.json`.

- `richtext::Block` enum: `Heading(u8, String)`, `Paragraph(String)`, `Bullet(String)`,
  `Code(String)`. `richtext::from_markdown(text) -> Vec<Block>` and
  `richtext::from_html(text) -> Vec<Block>`. No new dependency: markdown parsing is a small
  hand-rolled line scanner (headings by leading `#`, bullets by leading `-`/`*`, fenced code
  blocks by ``` ```` ```, everything else a paragraph); HTML is a small tag-aware stripper that
  tracks `<p>`, `<li>`/`<ul>`/`<ol>`, `<h1>`–`<h6>`, `<pre>`/`<code>`, and `<a href="...">text</a>`
  (link becomes `text (href)`). `<img>` and `<table>` (and their contents) are dropped.
- `Source::description(&self, project_id: &str) -> Result<String, Error>` on the trait, no
  default body (both real sources answer it; a `FakeSource` in tests needs its own stub — check
  `mod-sources`/`testing` skill patterns for whether existing `FakeSource` test helpers need
  updating alongside this).
- `Modrinth::description`: reuses the project fetch (`GET /v2/project/{id}`), returns its
  `body` field.
- `CurseForge::description`: `GET /v1/mods/{id}/description`, returns the HTML string from
  whatever wrapper shape the endpoint uses (VERIFY: confirm the response envelope —
  `{"data": "<html>"}` vs a bare string — before trusting a hand-written fixture; there is no
  `CURSEFORGE_API_KEY` on this machine, so the fixture is synthetic per the `mod-sources` skill
  and needs an api-verifier pass later, or a `.env` key and `just record-fixture` by a human).
- Tests: markdown fixture covering a heading, a paragraph, a bulleted list, a fenced code
  block, and a link; HTML fixture covering the same shapes plus a skipped `<img>` and `<table>`;
  `Modrinth::description` and `CurseForge::description` against wiremock fixtures.
- Command: `just test 'package(gcl-core) and test(richtext)'` then `just check`.
- Commit: `feat(core): project description as rich-text blocks`.

### Task 2 — content title, project details, version listing (coder)

Files: `crates/gcl-core/src/instances/model.rs`, `crates/gcl-core/src/content/mod.rs`,
`crates/gcl-core/src/launcher/mod.rs`, the one `gcl-cli` call site touched by the
`check_updates` signature change.

- `ContentEntry.title: Option<String>`, `#[serde(default, skip_serializing_if = "Option::is_none")]`,
  after `file_name` in field order (on-disk order matters per the doc comment in `model.rs`).
- `content::install_file` sets `entry.title = Some(project.title.clone())`.
- `content::check_updates(ctx, instance: &mut Instance)`: for an entry with `title: None` whose
  source parses, fetch `source.project(&entry.project_id)` and set the title in place; save
  `instance.toml` once at the end if anything changed (do not save on every entry — one write
  per call). A source that errors on this lookup only skips the backfill for that entry
  (`warn`), the same tolerance `check_updates` already gives a dead source for versions.
  Update `Launcher::check_updates` (takes `&mut instance` now) and its `gcl-cli` call site.
- `Launcher::project_details(source: SourceId, project_id: &str) -> Result<ProjectDetails, Error>`
  where `ProjectDetails { project: Project, blocks: Vec<richtext::Block> }`. Calls
  `source.project` and `source.description`, converts with `richtext::from_markdown`
  (Modrinth) or `from_html` (CurseForge).
- `Launcher::project_versions(slug: &str, source: SourceId, project_id: &str, filter: &VersionFilter) -> Result<Vec<Version>, Error>`:
  lists versions and, separately, is paired on the UI side with the instance's installed
  `version_id` for that project (read via `instances().get(slug)` +
  `instances::content::installed`) — decide in this task whether the "installed" marker is
  computed in core (returning a richer row type) or left to `gcl-ui`'s `content_version_row`
  conversion; either is fine, but pick one and note it in the doc comment so Task 4 does not
  re-decide it.
- Installing a specific version reuses `content::add` with `AddRequest.version: Some(id)` at
  depth 0 — no new install path. Confirm with a test that pinning a version already installed
  from a *different* file name replaces the file in place (this exercises `place_file`'s
  existing replace-in-place behavior end to end through `add`, not just through
  `instances::content` directly).
- Tests: `content_entry_title_defaults_to_none_for_old_toml` (parse a hand-written TOML string
  with no `title` key), `title_stored_on_install` (via `FakeSource`), `title_backfilled_on_check_updates`,
  `check_updates_does_not_save_when_nothing_changed`, `pinning_a_version_replaces_the_old_file`
  (through `content::add`, asserting the old file name is gone).
- Command: `just check`.
- Commit: `feat(core): content title, project details, and version listing`.

### Task 3 — icon fetch cache (coder)

Files: `crates/gcl-core/src/launcher/mod.rs`, `crates/gcl-core/src/download/mod.rs` (only if a
shared helper is worth extracting).

- `Launcher::fetch_icon(&self, url: &str) -> Result<PathBuf, Error>`: destination
  `cache/icons/<sha1(url)>.<ext>` where `<ext>` comes from the URL's extension, falling back to
  a generic one (e.g. `bin`) when the URL carries none — the UI decoder in Task 5 sniffs the
  real format from bytes regardless, so the extension is a cache-file label, not a contract.
  Downloads through the existing `HttpClient`/download machinery (reuse `download_one`/
  `DownloadSpec` if the shape fits an unknown-sha1, size-only-verified download; a bespoke
  small function is fine if `DownloadSpec`'s object-store addressing does not fit a URL-keyed
  cache).
  Single-flight: a `Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>` (or a `dashmap`-free
  equivalent) on `Launcher` keyed by URL, so two callers asking for the same icon at once wait
  on one download rather than racing two.
  A file already at the destination is returned without a new request.
- Tests: `fetch_icon_writes_under_cache_icons_hashed_by_url`, `fetch_icon_is_single_flight`
  (wiremock `expect(1)` on the icon path while two calls run concurrently — see the
  `download-cache` skill's dedupe pattern), `fetch_icon_reuses_the_cached_file`.
- Command: `just check`.
- Commit: `feat(core): cached icon downloads`.

### Task 4 — project details screen: navigation, Description tab, Versions tab (slint-designer)

Files: `crates/gcl-ui/ui/types.slint`, `crates/gcl-ui/ui/app.slint`,
`crates/gcl-ui/ui/screens/project.slint` (new), `crates/gcl-ui/src/screens/project.rs` (new),
`crates/gcl-ui/src/lib.rs`, `crates/gcl-ui/ui/screens/browser.slint`,
`crates/gcl-ui/src/screens/browser.rs`.

- `types.slint`: `Block { kind: string, text: string }` (`kind` one of `heading1`..`heading6` or
  a level+text encoding of your choosing — document it in the doc comment; `paragraph`,
  `bullet`, `code`); `ContentVersionRow { id, number, kind, release_time, game_versions, loaders, installed: bool }`
  (`game_versions`/`loaders` pre-joined for display, matching how other rows in this file avoid
  passing arrays where a joined string prints fine).
- `app.slint`: add `Screen.project` to the `Screen` enum in `types.slint`, mount
  `if App.screen == Screen.project: ProjectScreen { }` next to the other screens.
- `ui/screens/project.slint`: `ProjectState` global — `source`, `project_id`, `title`, `author`,
  `kind`, `downloads`, `page_url`, `target_slug`, `tab` (0/1), `blocks: [Block]`,
  `versions: [ContentVersionRow]`, `loading`, `status`, `show_all_versions: bool`,
  `return_to: Screen`. Callbacks: `open(string source, string project_id, string target_slug, Screen return_to)`,
  `set_tab(int)`, `back()`, `toggle_show_all()`, `install(string version_id)`. `ProjectScreen`
  component: header (title, author, kind, downloads, a Back button wired to `back()`, an "Open
  page" link showing `page_url` as read-only selectable text per the "no clipboard" limitation
  in the `slint-ui` skill), a `TabBar` (Description/Versions), the Description tab as a
  scrolling `ListView`/`VerticalLayout` of `Text` styled per `Block.kind`, the Versions tab as
  rows (number, kind, release_time, game_versions, loaders, an `row_install` Button that reads
  "Installed" and is disabled when `row.installed` is true) plus a `show_all_versions_check`
  checkbox. `PreviewProjectScreen` at the end with sample blocks and versions.
- Wire every property in `src/screens/project.rs`'s `open`/`load` path (none left at its empty
  default), per the `slint-ui` skill's rule.
- `browser.rs`/`browser.slint`: clicking a search row's title text (a small `TouchArea` or
  promoting the row's title into its own clickable element — do not repurpose the whole
  `ListRow`'s `clicked`, which already selects the row for arrow-key navigation) calls
  `ProjectState.open(row.source, row.project_id, BrowserState.target_slug, Screen.browser)`
  then `App.navigate(Screen.project)`.
- Tests: none new here beyond compiling; `just ui-preview screens/project.slint --check` and a
  manual `just ui-preview screens/project.slint` look, described in the report. Real assertions
  land in Task 6's flow test.
- Command: `just check`.
- Commit: `feat(ui): project details screen with description and versions tabs`.

### Task 5 — installed list titles, source button, and search icons (slint-designer)

Files: `crates/gcl-ui/src/models/mod.rs`, `crates/gcl-ui/ui/screens/instance.slint`,
`crates/gcl-ui/src/screens/instance.rs`, `crates/gcl-ui/ui/screens/browser.slint`,
`crates/gcl-ui/src/screens/browser.rs`, `crates/gcl-ui/Cargo.toml`, root `Cargo.toml`.

- `models::content_row`: show `entry.title.clone().unwrap_or_else(|| file_stem(&entry.file_name).to_string())`
  as the row's name, and add a second field to `ContentRow` (`types.slint`) for the file name
  line if one is not already distinct from `name` (`ContentRow.file_name` already exists — use
  it for the second line, `name` for the first). `content_rows` (in `screens/instance.rs`) sorts
  by the same title, case-insensitively, before building the `Vec<ContentRow>`.
- `instance.slint`: the content row becomes two lines (title on top, `row.file_name` muted
  below) instead of the single `Text { text: row.name }`; the source `Text` becomes
  `row_source_button := Button` (or a small clickable element with the three accessible
  properties) that calls `ProjectState.open(row.source, row.project_id, InstanceState.slug_or_equivalent, Screen.instance)`
  then `App.navigate(Screen.project)` — check what identifies "this instance" on
  `InstanceState` already (likely `App.current_slug`, not a slug stored on `InstanceState`
  itself) and use that rather than adding a redundant property.
- `browser.slint`/`browser.rs`: add a 48px icon `Image` at the left of each search row, blank
  until loaded, a placeholder tile (from `Theme`) when the project has none or the fetch fails.
  `browser.rs`'s row-building path spawns `Bridge::run` per row needing an icon (or a single
  batched job — pick whichever avoids one thread per row on a 20-row page) calling
  `Launcher::fetch_icon`, decodes the returned file with the `image` crate off the UI thread
  (inside the same `Bridge::run` job body, before the result crosses back), and only builds the
  `slint::Image`/`SharedPixelBuffer` in the `done`/`upgrade_in_event_loop` closure.
- Add `image` to `crates/gcl-ui/Cargo.toml` (`[dependencies]`) and the workspace `Cargo.toml`
  (`[workspace.dependencies]`), no default features, `features = ["png", "webp", "gif", "jpeg"]`.
  Run `just deny` and resolve anything it flags before committing.
- Tests: `content_row_shows_title_over_file_stem`, `content_rows_sort_by_title_case_insensitive`,
  a decode-path unit test that feeds known PNG/WebP/GIF/JPEG bytes through the same conversion
  function the row-building path calls (pull the "bytes to `Vec<u8>` RGBA + dimensions" step
  into a pure function so it is testable without a window, per the `testing` skill's "pure
  helper first" rule).
- Command: `just check && just deny`.
- Commit: `feat(ui): installed list titles, source button, search row icons`.

### Task 6 — GUI flow test and unit-test sweep (coder + slint-designer, then api-verifier)

Files: `crates/gcl-ui/tests/flow_project.rs` (new), any fixes the flow finds in
`crates/gcl-ui/src/screens/{project,browser,instance}.rs` or `crates/gcl-core/src/content/mod.rs`.

- `flow_project.rs` (one `#[test]`, per the "one Slint backend per process" rule): opens the
  browser, searches (wiremock Modrinth fixture with two versions of one project), clicks a
  search row's title to open details, switches to the Versions tab, installs the older
  version, asserts the instance's content list shows it; reopens details from the installed
  list's source button, installs the newer version, asserts the old file is gone and the new
  one is present (mirrors the `place_file`-replaces-in-place unit test, but end to end through
  the UI); asserts the installed list shows the title on top and the file name below, in name
  order; asserts a search row carries a decoded icon after the mock serves one (assert on
  whatever observable state the row-building path exposes — e.g., a boolean or the image's
  size — since pixel-comparing a `slint::Image` is not practical in a flow test).
- Fix whatever this flow finds broken in the three tasks above before moving on — this is the
  same pattern plan 7/8 used (build the screen, then let the flow test find the wiring bugs).
- Run `just check`.
- **api-verifier**: since Task 1 changed `crates/gcl-core/src/sources/*`, run
  `just verify-api modrinth` live (Modrinth's `description`/`body` field) and report PASS/FAIL.
  CurseForge's `description` endpoint stays unverified on this machine (no
  `CURSEFORGE_API_KEY`) — note that explicitly rather than skipping the report.
- A FAIL from api-verifier goes back to coder with the report, then api-verifier runs again,
  per `CLAUDE.md`'s delegation rules.
- Commit: `test(ui): mod details flow; fixes found along the way`.

### Task 7 — docs, ids, ui-xtest, live verification (doc-research drafts, coder/slint-designer edits, then a live run)

Files: `docs/SPEC.md`, `ARCHITECTURE.md`, `.claude/skills/mod-sources/SKILL.md`,
`.claude/skills/slint-ui/SKILL.md`, `.claude/skills/testing/SKILL.md`, `CHANGELOG.md`,
`docs/research/2026-09-07-ui-element-ids.md`, `scripts/ui-xtest.py`.

- `docs/SPEC.md`: add status rows for the description endpoint, the details screen, the
  title/sort change, and the search icon, under the nearest existing requirement (`R7`/`R13`)
  or as a short new subsection if none fits cleanly — follow the existing status-table format
  in the file.
- `ARCHITECTURE.md`: update the `sources` and `content` module rows for `description`,
  `richtext`, `ContentEntry.title`, and `Launcher::project_details`/`project_versions`/
  `fetch_icon`; update the `gcl-ui` structure block for the new screen file and `Screen.project`.
- Skills: `mod-sources` gets the `description` endpoint shapes (and the CurseForge envelope
  VERIFY note); `slint-ui` gets the `ProjectState`/`return_to` pattern as the reference example
  for "a screen that needs to remember where it came from"; `testing` gets a one-line pointer
  to `flow_project.rs` in the flow-test list.
- `CHANGELOG.md`: an `Unreleased` bullet for the details screen, install-a-version, title
  display, and search icons.
- `scripts/list-slint-ids.sh` re-run, `docs/research/2026-09-07-ui-element-ids.md` refreshed
  with the new ids (`row_source_button`, `show_all_versions_check`, `row_install`'s reuse in
  the new screen if the id is shared, etc.).
- `just ui-xtest`: extend `scripts/ui-xtest.py`'s driven recipe (or add a second recipe) with
  steps that open a details page with real X input — click a search row's title, wait for the
  Description tab to render, switch to Versions, and take a `shot` — so this feature gets the
  same real-pointer/real-focus check plan 8 added for the rest of the app. Confirm the app's
  GUI log (`job ok label=...` lines) proves the steps happened, per the existing pattern.
- Live run: `just verify-api modrinth` (repeated here if Task 6's run is stale by the time this
  task lands), then `just check && just deny && just lint-claude && just ui-xtest`.
- Commit: `docs: mod details and browser polish`.

## Verification

- api-verifier: required after Task 1 (and again after any coder fix in Task 6), since
  `sources/` changed. `just verify-api modrinth` for the description endpoint; CurseForge stays
  a documented VERIFY gap.
- e2e-runner: not required — this plan touches no `instances/`, `launch/`, `modpacks/`,
  `settings/`, or `gcl-cli` behavior beyond the one `check_updates` call-site signature update,
  which is covered by existing CLI tests, not a behavior change `just e2e` would newly exercise.
  Run `just e2e` once anyway at the end of Task 7 as a cheap regression check, since it is fast
  and already in the standard command set.

## Risks

- **CurseForge `description` response shape is unverified.** No `CURSEFORGE_API_KEY` on this
  machine. Mitigation: Task 1's fixture is explicitly marked synthetic per the `mod-sources`
  skill's existing convention, and Task 7 carries the VERIFY note forward into the skill and
  `docs/SPEC.md` rather than letting it go silent.
- **`check_updates` signature change (`&Instance` → `&mut Instance`) ripples to every caller.**
  Mitigation: Task 2 is scoped to include the `gcl-cli` call site explicitly, and `just check`
  at the end of that task catches any other caller the grep missed.
- **Icon fetch on a search page could spawn many concurrent worker threads.** Mitigation:
  Task 5 calls out picking a bounded/batched approach instead of one thread per row, and the
  single-flight map from Task 3 caps duplicate downloads across screens.
- **A debounce or timing-sensitive interaction in the Versions tab (e.g., a slider-like show/hide
  for "all versions") cannot be driven by `TestApp::pump`.** Mitigation: Task 4 specs a plain
  checkbox (`show_all_versions_check`), not a debounced control, so Task 6's flow test can
  assert on it directly.

## Open questions

1. **CurseForge `GET /mods/{id}/description` response envelope** (bare HTML string vs.
   `{"data": "<html>"}`) is not confirmed anywhere in the current codebase or fixtures. Task 1
   should treat this as VERIFY and either get a live check from a human with a
   `CURSEFORGE_API_KEY`, or ship the more common CurseForge Core API shape (`{"data": ...}`,
   matching every other CurseForge endpoint this codebase already parses) and flag it loudly in
   the skill and `docs/SPEC.md` status table rather than asserting confidence the code does not
   have.
2. **Where the "installed version" marker for the Versions tab is computed** (in
   `Launcher::project_versions` returning a richer type, vs. in `gcl-ui`'s row conversion
   reading `InstanceState`/a passed-in installed-version-id) is left as a call for whoever
   implements Task 2 and Task 4 to agree on and document once, rather than a decision made
   here without seeing how `InstanceState` already exposes (or doesn't) the current instance's
   content list at the point the Versions tab needs it.
