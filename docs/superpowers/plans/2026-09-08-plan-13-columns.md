# Plan 13: browser columns and list polish

Spec: `docs/superpowers/specs/2026-09-08-browser-columns-design.md`. Status: in progress.

## Global constraints

No `unwrap`/`expect` outside tests; no network in tests; element ids `<thing>_<kind>`; sample
data only in `Preview*`; `just check` per task; commits end with
`Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Tasks

| # | Owner | Files | Work |
|---|---|---|---|
| 1 | coder | `sources/{types,modrinth,curseforge}.rs` (+tests, fixtures) | `SearchHit.updated` from `date_modified`/`dateModified`; every `SearchHit {` literal gains it |
| 2 | slint-designer | `ui/theme.slint`, `ui/screens/{browser,instance}.slint`, `ui/components/list-row.slint`, `src/models/mod.rs`, `src/screens/browser.rs`, ids doc | header row, columns with theme widths, date `YYYY-MM-DD`, state line under the button, no title hover, alternating rows in both lists, in-place row patching (no `set_rows` after paint, no clearing on target change) |
| 3 | coder + slint-designer | `tests/flow_content.rs`, `tests/support/mod.rs`, fixes | header present, state line texts, model identity + icon survive an install, no hover element on the title; real-input screenshot; docs/CHANGELOG/SPEC row |

Order: 1, then 2, then 3. api-verifier after Task 1 (`sources/` changed): confirm `date_modified` on a live Modrinth hit.
