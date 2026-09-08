# Mod details and browser polish: design

Date: 2026-09-07. Status: approved in chat.

## Goals

1. A mod details screen with a Description tab and a Versions tab, reached from a browser
   search row and from the installed content list.
2. Install a specific version from the Versions tab. Installing over an installed mod removes
   the old file and places the chosen one.
3. The installed list shows two rows per entry: the mod name on top, the file name below,
   sorted by name. The source badge is a button that opens the details screen.
4. Search rows show the project's icon.
5. Search still runs on Enter or the Search button. Nothing changes there.

## Design

### Core

- `Source::description(project_id) -> Result<String>`: Modrinth returns the project `body`
  (markdown); CurseForge calls `GET /mods/{id}/description` (HTML). Fixtures under
  `tests/fixtures/{modrinth,curseforge}/`.
- `sources::richtext`: converts markdown or HTML to `Vec<Block>` where
  `Block::{Heading(level, text), Paragraph(text), Bullet(text), Code(text)}`. Links keep their
  text and append the URL in parentheses. Images and tables are skipped. No new dependency for
  markdown; HTML uses a small tag stripper with paragraph and list awareness. Tests cover both
  inputs.
- `ContentEntry.title: Option<String>` stored on install from `Project.title`. Old entries get
  it from the source during Check updates. Display falls back to the file stem.
- `Launcher::project_details(source, project_id) -> ProjectDetails { project, blocks }` and
  `Launcher::project_versions(source, project_id, filter) -> Vec<Version>`. Installing a chosen
  version reuses `content::add` with `version: Some(id)` at depth 0, which replaces the file.
- Icons: `Launcher::fetch_icon(url) -> PathBuf` downloads into `cache/icons/<sha1(url)>.<ext>`
  through the download cache, off the UI thread, at most one request per URL at a time. The UI
  decodes with the `image` crate (PNG, WebP, GIF, JPEG) into `slint::Image`.

### UI

- New `Screen.project`. `ProjectState` global: source, project id, title, author, kind,
  downloads, page url, target slug, tab (0 description, 1 versions), `blocks`, `versions`,
  `loading`, `status`, `show_all_versions`.
- Reached from `BrowserScreen` (click the row title) with the browser's target instance, and
  from `InstanceScreen` (click the source button) with that instance. Back returns to the
  screen it came from.
- Versions tab: rows with number, release kind, date, game versions, loaders, and an Install
  button. Versions match the target instance's Minecraft version and loader by default; a
  checkbox shows all. The installed version shows "Installed" instead of Install. Install runs
  through `Bridge`, reloads the instance content, and returns a status line.
- Description tab: a scrolling column of `Text` blocks styled by kind.
- Installed list: two-line `ListRow` with name and file name; sorted by name, case-insensitive,
  in Rust. The source badge becomes a `Button` (`row_source_button`) that opens details.
- Search rows: a 48 px icon at the left, blank until the icon is loaded; a placeholder tile
  when the project has no icon or the download fails.

### Tests

Core: richtext converter (markdown and HTML fixtures), description endpoints, title stored on
install and backfilled on update check, replace-on-install keeps one file, icon cache path and
single-flight. GUI: `flow_project.rs` opens details from a search row, switches to Versions,
installs a version, opens details from the installed list, installs another version, and
asserts the old file is gone and the new one present; the installed list shows the title on
top and the file name below in name order; a search row carries an icon after the mock serves
one.

### Out of scope

Rendering images inside descriptions; CurseForge live verification without a key.
