# Description rendering, changelog notes, browser rows: design

Date: 2026-09-08. Status: approved in chat. Research: `docs/research/2026-09-08-description-markup.md`.

## Goals

1. Descriptions render their block structure: tables, images, rules, quotes, nested lists,
   collapsible sections, bold-only lines as subheadings.
2. Each version row has a Notes button that opens the version's changelog in a modal.
3. Browser rows show the full title, drop the URL, size to their content, and collapse line
   breaks inside the short description.

## Design

### Core: `sources::richtext`

`Block` gains `Table { header: Vec<String>, rows: Vec<Vec<String>> }`, `Rule`,
`Image { url: String, alt: String }`, `Quote(String)`, and `Bullet { depth: u8, text }` replaces
`Bullet(String)`. Markdown: pipe tables with an alignment row; `---`/`***` rules; `> ` quotes
(one level); list depth from leading spaces (two per level) or tabs; `![alt](url)` and `<img>`
become `Image`; a badge (image inside a link) is dropped; `<details>`/`<summary>` become a
`Heading(3, summary)` followed by the content; a paragraph that is entirely `**bold**` becomes
`Heading(4, text)`; inline code loses its backticks; strikethrough markers are stripped. HTML:
`<table>` rows and cells (first row or `<th>` cells are the header), `<hr>`, `<blockquote>`,
`<ul>`/`<ol>` nesting depth, `<img src alt>`. Limits: 2 000 blocks, 8 KiB per block, 50 table
rows and 8 columns per table, cells cut at 200 chars. The markdown scanner stays hand-rolled.

### Core: images and changelogs

- `Launcher::fetch_image(url) -> PathBuf`: like `fetch_icon` but any `https://` host, a 5 MiB cap,
  the same single-flight map, stored under `cache/images/<sha1(url)>.<ext>`. The UI asks for at
  most 20 images per description and decodes with `image` under a 4096×4096 pixel limit
  (`image::Limits`), off the UI thread.
- `Version.changelog: Option<String>` (Modrinth: the `changelog` field; CurseForge: `None`).
  `Source::changelog(project_id, version_id) -> Result<String>`: Modrinth returns the stored
  string; CurseForge calls `GET /v1/mods/{id}/files/{fileId}/changelog` (`{"data": html}`,
  VERIFY). `Launcher::version_notes(source, project_id, version_id) -> Vec<Block>` converts with
  the right converter.

### UI

- `Block` in Slint gains `kind` values `table`, `rule`, `image`, `quote`, plus `depth` and
  `cells: [string]`/`columns: int` for tables. Rendering: table as a `GridLayout` of `Text`
  cells with a bold header row on a surface tile, horizontally scrollable when wider than the
  column; rule as a 1 px line; quote as an indented muted paragraph with a left bar; bullets
  indented by depth; image as an `Image` sized to the column width with the alt text below,
  a placeholder tile while loading, `[Image: alt]` when the fetch fails.
- Versions tab: `version_notes_button` per row opens `NotesDialog` (a `Dialog` with a
  `ScrollView` of blocks, title "<number> notes", Close, Escape). `ProjectState.notes_open`,
  `notes_title`, `notes_blocks`, `notes_loading`. Mounted only while open.
- Browser rows: title `Text` with `wrap: word-wrap`, no elide; the URL `Text` removed; the row
  height is the content's preferred height; the short description has `\n` and `\r` replaced
  by spaces in the Rust row builder.

### Tests

Core: converter cases per feature (markdown and HTML), the ImmediatelyFast table, badge drop,
details, bold-only heading, limits; `fetch_image` cap and https rule; changelog endpoints with
fixtures; `version_notes` conversion. GUI: `flow_project` gains a table block assertion, an
image block with a mock-served PNG, the Notes modal open/close with a changelog fixture;
`flow_content` asserts a long title is not elided and a description with a newline renders on
one wrapped paragraph. Real-input smoke: open a details page with a table.

### Out of scope

Inline bold/italic inside a paragraph (Slint `Text` has no spans); rendering `<iframe>` embeds.
