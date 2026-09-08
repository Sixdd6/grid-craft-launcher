# Browser columns and list polish: design

Date: 2026-09-08. Status: approved in chat.

## Goals

1. Browser rows align to columns: Name | Author | Downloads | Last updated | button.
2. The row title has no underline, hover cue or pointer change. The whole row opens details.
3. Rows alternate background in the browser and in the instance content list.
4. Install, Update and Add never blink the list; the one row updates in place.

## Design

### Core

`SearchHit.updated: String` from Modrinth `date_modified` and CurseForge `dateModified`
(RFC 3339, empty when absent). No other core change.

### UI

- A header row (`results_header`) above the results with the five column labels in
  `Theme.text-muted`, hidden while there are no rows. Column widths from theme tokens:
  Author `Theme.col-author` (140 px), Downloads `Theme.col-downloads` (100 px, right-aligned),
  Last updated `Theme.col-date` (110 px), button column `Theme.col-action` (170 px). Name fills
  the rest: icon, title (elided), then the muted one-line description (elided).
- The button column stacks the button and a small state line beneath it: "Latest 0.6.13",
  "Installed 0.6.0 ↑" when older, "Installed 0.6.13" when current, "No version" when none,
  "Checking…" while unresolved. The `↑` stays on the older state.
- `row_title` loses its hover rectangle, underline and pointer cursor; `row_open`'s click keeps
  opening details. The modpack rows get the same header and columns (Name | Author | Downloads
  | Last updated | Install).
- Date shown as `YYYY-MM-DD` from the RFC 3339 string in the Rust row builder (empty stays
  empty).
- Alternating rows: `Theme.surface-alt` on odd indices in the browser results, modpack results,
  and the instance content list (the `ListRow` gains an `alt: bool` input).
- Blink: the install and update paths, and the Add refresh, must patch the affected row through
  `set_row_data`; no `set_rows` with a fresh model after a search has painted, and no clearing of
  rows on a target change (states reset per row in place). A flow assertion pins the row count
  and the icon field across an install.

### Tests

Core: the date field parsed for both sources (fixtures). UI: `search_row` date formatting;
flow_content asserts the header is present, the state line under the button reads as
specified, the row model identity survives an install (same `ModelRc`, same icon), and the
title has no hover element. Real-input screenshot of the browser with results.

### Out of scope

Sorting by column; resizable columns.
