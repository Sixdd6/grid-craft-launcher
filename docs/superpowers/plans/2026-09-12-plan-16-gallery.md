# Plan 16: Project image gallery

Date: 2026-09-12. Status: done 2026-09-12 (1134 tests). api-verifier: Modrinth PASS live, CurseForge WARN (no key).
Requirement ids: R7.1, R13.1.

## Summary

A project's details screen gains a Gallery tab: a thumbnail grid of the images the source
publishes with the project (Modrinth's `gallery`, CurseForge's `screenshots`), and a full-size
viewer overlay with Prev/Next and Escape-to-close. `gcl-core`'s common `Project` gains a
`gallery: Vec<GalleryImage>` field that both source clients populate from data their fixtures
already carry (or, for CurseForge, already will once the fixture is extended). Everything reuses
the description-image fetch path (`Launcher::fetch_image`, `decode_description_image`) and the
generation-counter pattern `project.rs` already uses for the Description tab and the Notes modal.
No change touches `browser.slint`, `browser.rs`, or `flow_content.rs`, which another agent has in
flight.

## Facts already gathered (do not re-research)

- Modrinth `GET /v2/project/{id}` returns `gallery: [{url, raw_url, featured, title?,
  description?, created, ordering}]`. `tests/fixtures/modrinth/project_sodium.json` lines
  343-398 already has 6 entries. `RawProject` in `modrinth.rs:512-522` does not parse it today.
- CurseForge `GET /v1/mods/{id}` returns `screenshots: [{id, modId, title, description,
  thumbnailUrl, url}]` on `media.forgecdn.net`/`edge.forgecdn.net`. `tests/fixtures/curseforge/
  get_mod.json` lines 62-71 has 1 entry. `RawMod` in `curseforge.rs:822-843` does not parse it
  today.
- The common `Project` (`types.rs:138-155`) has no gallery field.
- Images already reach the screen through `Launcher::fetch_image(url)` (`download::images`, any
  `https://` host, 5 MiB cap, `cache/images`) and `decode_description_image`
  (`crates/gcl-ui/src/models/mod.rs:215-237`), which downscales to 1600 px on the long side and
  builds `slint::Image::from_rgba8`. `project.rs` already runs a generation counter for
  description images (`image_generation`) and for the Notes modal (`notes_generation`); the
  gallery copies that shape with its own counters rather than sharing either one.

## Files

| File | Change | Reason |
|---|---|---|
| `crates/gcl-core/src/sources/types.rs` | new `GalleryImage { url, thumbnail_url: Option<String>, title: Option<String>, description: Option<String>, featured: bool }`; `Project.gallery: Vec<GalleryImage>` | shared shape both sources map onto |
| `crates/gcl-core/src/sources/modrinth.rs` | `RawProject` gains `gallery: Vec<RawGalleryImage>` (`RawGalleryImage { url, title: Option<String>, description: Option<String>, featured: bool, ordering: i64 }`, `#[serde(default)]` on the array and on `title`/`description`/`featured`/`ordering`); map into `Project.gallery` sorted by `ordering`, `thumbnail_url: None` (Modrinth serves no separate thumbnail); apply in both `project()` and `project_with_body()`, since both build a `Project` from the same `RawProject` | R7.1: Modrinth gallery |
| `crates/gcl-core/src/sources/curseforge.rs` | `RawMod` gains `screenshots: Vec<RawScreenshot>` (`#[serde(rename_all = "camelCase")]`, `RawScreenshot { title: String, description: String, thumbnail_url: String, url: String }`, `#[serde(default)]` on the array and the two text fields); map into `Project.gallery` in source order, `thumbnail_url: Some(thumbnail_url)`, `title`/`description` as `None` when empty, `featured: false` always (CurseForge sends no such flag) | R7.1: CurseForge gallery |
| `tests/fixtures/curseforge/get_mod.json` | no change needed — the 1 existing `screenshots` entry (lines 62-71) is enough for a unit test; add a second entry only if the test needs to check ordering, which CurseForge does not publish, so one is enough | keep the fixture minimal |
| `docs/research/2026-09-06-modrinth-and-curseforge-apis.md` or `.claude/skills/mod-sources/SKILL.md` | one paragraph: `Project.gallery` maps Modrinth's `gallery` (sorted by `ordering`) and CurseForge's `screenshots` (no ordering, no featured flag); CurseForge's shape stays unverified live, same caveat `description`/`changelog` already carry | keep the skill in step with the code |
| `ARCHITECTURE.md` | `sources` module row: add `GalleryImage` to the list of shared types in `types.rs` | new type in the module's public interface |
| `crates/gcl-ui/ui/types.slint` | new `struct GalleryImageRow { url: string, title: string, description: string, image: image, image_state: string }` | thumbnail row shape crossing the boundary |
| `crates/gcl-ui/ui/screens/project.slint` | `TabBar.tabs` gains `"Gallery"` (index 2); a wrapping thumbnail grid tab body (fixed-height cards, `image-fit: cover`, title caption); an `EmptyState` "No images" when the list is empty; a full-window viewer overlay (`image-fit: contain`, title, description, Prev/Next buttons, Close button), mounted only while `ProjectState.viewer_open`; `keys`'s `key-pressed` grows an `Escape` branch that closes the viewer first when it is open, Back otherwise | R7.1, R13.1: the Gallery tab and viewer |
| `crates/gcl-ui/ui/screens/project.slint` (`PreviewProjectScreen`) | sample `gallery` rows and a `viewer_open` sample, so `just ui-preview screens/project.slint` shows the new tab and the overlay | preview parity, per the `slint-ui` skill |
| `crates/gcl-ui/src/screens/project.rs` | `Shared` gains `gallery_generation: Arc<AtomicU64>`; `open()` resets `ProjectState.gallery`/`viewer_*` the empty way every other property gets reset, bumps the new counter, and fetches gallery thumbnails (thumbnail_url if present else url) the same way `fetch_description_images` does, capped at 20; new `open_viewer(index)`/`close_viewer()`/`viewer_next()`/`viewer_prev()` callbacks: opening fetches the full-size image (uncapped by the 20-thumbnail limit, since only one full image is ever in flight) and guards it with a `viewer_generation` counter so a fast next/prev discards a slow fetch behind it, the same shape `notes_generation` already proves | wiring: fetch, decode, generation guard |
| `crates/gcl-ui/src/models/mod.rs` | `gallery_row(image: &GalleryImage) -> GalleryImageRow` (pure converter, no image/image_state filled — same split `block_row` uses with `prepare_images`) | keep `src/models/` the one place a core struct becomes a Slint struct |
| `crates/gcl-ui/tests/support/mod.rs` | `mod_project(base)` also rewrites each gallery entry's `url` (and `raw_url`, if the response test overwrite touches it) to the mock host, the way `icon_url` and `body` already are, so the flow test fetches no real CDN URL; new consts for the gallery image path(s) and title(s) the flow asserts on | keep the flow test off the real network |
| `crates/gcl-ui/tests/flow_project.rs` | one new sub-flow: open the Gallery tab, see the fixture's thumbnail count, open the viewer on the first thumbnail, step Next, close with Escape and confirm the tab is still Gallery (Escape closes the viewer, not the whole screen) | R13.5: this screen is flow-tested end to end |
| `docs/SPEC.md` | R7.1 sentence gains the gallery; the R7.1 status row gains one clause: Modrinth's gallery verified live (`just verify-api modrinth`), CurseForge's unverified (no key), same shape as the description/changelog caveats already there | accuracy, per the planning skill's "cite requirement ids" |
| `docs/research/2026-09-07-ui-element-ids.md` | refresh from `scripts/list-slint-ids.sh` | new element ids: `gallery_thumbnail`, `gallery_viewer_prev/next_button`, `gallery_viewer_close_button`, the `Gallery` `tab_entry` |

## Steps

Steps 1 touches `gcl-core`; steps 2-4 touch `gcl-ui` and run after it, since the UI needs
`GalleryImage` on `Project` to compile. Steps 3 and 4 share `project.rs`/`project.slint`/
`support/mod.rs`, so they run in order, one agent at a time. Step 5 (docs) touches files no other
step does and can start once its own subject (core or UI) has landed.

### 1. Core: `GalleryImage` and the two parsers
Agent: coder.
Files: `crates/gcl-core/src/sources/types.rs`, `crates/gcl-core/src/sources/modrinth.rs`,
`crates/gcl-core/src/sources/curseforge.rs`, `ARCHITECTURE.md` (the `sources` row only).
Tests: a Modrinth unit test over `tests/fixtures/modrinth/project_sodium.json` through wiremock
asserting 6 gallery entries in `ordering` order with the right `title`/`featured` on at least one;
a CurseForge unit test over `tests/fixtures/curseforge/get_mod.json` asserting 1 entry with
`thumbnail_url: Some(..)` and `featured: false`; a third test asserting a project fixture with no
`gallery`/`screenshots` key parses to an empty `Vec`, not an error.
Proof: `cargo nextest run -p gcl-core gallery`, then `just check`.

### 2. api-verifier
Runs after step 1, per the delegation rule for a `sources/` change.
Command: `just verify-api modrinth` (expect a `gallery` count printed or asserted for a known
project, e.g. sodium) and `just verify-api curseforge` (expect SKIP, no `CURSEFORGE_API_KEY` on
this machine — the CurseForge screenshot shape stays unverified, noted in the SPEC row step 5
writes).
A FAIL sends the report back to coder before step 3 starts.

### 3. UI types and the Gallery tab
Agent: slint-designer.
Files: `crates/gcl-ui/ui/types.slint`, `crates/gcl-ui/ui/screens/project.slint` (tab, grid, viewer
overlay, `PreviewProjectScreen` sample data, the `keys` Escape branch).
Proof: `slint-viewer --check --style fluent crates/gcl-ui/ui/app.slint` clean; `cargo check -p
gcl-ui`.

### 4. Wiring, thumbnails, and the viewer
Agent: slint-designer.
Files: `crates/gcl-ui/src/screens/project.rs`, `crates/gcl-ui/src/models/mod.rs`.
Test: a unit test on `gallery_row` (pure converter, no window needed).
Proof: `cargo nextest run -p gcl-ui gallery_row`, `cargo check -p gcl-ui`.

### 5. Flow test
Agent: slint-designer.
Files: `crates/gcl-ui/tests/support/mod.rs`, `crates/gcl-ui/tests/flow_project.rs`.
Test: the new Gallery sub-flow described in the files table.
Proof: `cargo nextest run -p gcl-ui --test flow_project` (one binary, one `#[test]`, per the
`.config/nextest.toml` 5×60s timeout the architecture doc names).

### 6. Docs
Agent: whichever of coder/slint-designer finishes last (coder for the `mod-sources`/research
line, slint-designer for the element-ids refresh; either may take `docs/SPEC.md`, since it names
both a core and a UI fact).
Files: `.claude/skills/mod-sources/SKILL.md` (or the research doc it points at), `docs/SPEC.md`,
`docs/research/2026-09-07-ui-element-ids.md`.
Proof: `scripts/list-slint-ids.sh` output matches the doc; `just lint-claude` if the skill file
changed.

### 7. Final verification
`just check`. No `e2e-runner`: nothing under `instances/`, `launch/`, `modpacks/`, `settings/`, or
`gcl-cli` changed. `api-verifier` already ran at step 2 and does not need a second pass unless
step 1 changed after its report.

## Verification

`api-verifier` (step 2, required: `sources/` changed). No `e2e-runner` (no `instances/`/`launch/`/
`modpacks/`/`settings/`/`gcl-cli` change). `just check` at the end.

## Risks

- **CurseForge's `screenshots` envelope is unverified live**, the same gap `description` and
  `changelog` already carry (no `CURSEFORGE_API_KEY` on this machine). The SPEC row and the skill
  note it rather than claim more than the fixture proves.
- **A gallery image is fetched from any `https://` host**, same as a description image — no new
  allowlist risk, but the flow test must rewrite every gallery URL to the mock host or it will
  try the real internet and hang until the mock's `with_image_hosts` seam refuses it.
- **Two independent fetch jobs (thumbnails, viewer full image) racing a fast tab switch or a fast
  Next/Prev** is the same class of bug the Notes modal already hit once; steps 3-4 must give the
  viewer its own generation counter rather than reusing `gallery_generation`, or a slow full-image
  fetch for image 1 can land after the user has already moved to image 3.
- **`browser.slint`/`browser.rs`/`flow_content.rs` are being edited by another agent right now.**
  No step in this plan touches them; if a merge conflict shows up anyway, that is a signal this
  plan drifted from that scope, not something to resolve by editing those files here.
- **20-thumbnail cap**: a project with a very large gallery shows the first 20 (Modrinth's
  `ordering`, CurseForge's response order) and nothing past that, matching the description-image
  cap's own documented degrade rather than inventing a new one.
