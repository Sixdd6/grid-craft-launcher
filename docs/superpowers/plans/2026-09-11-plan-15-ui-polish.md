# Plan 15: UI polish and a look-and-feel standard

Date: 2026-09-11. Status: done 2026-09-11 (1128 tests). Follow-up in the same commit: the browser combos bind two-way and the target follows `App.current_slug`.
Design: `docs/superpowers/specs/2026-09-11-ui-polish-and-style-design.md`.
Requirement ids: R13.1, R13.3, R13.4, R9.4.

## Summary

`gcl-ui` gets a written look-and-feel standard as `Theme` tokens plus `docs/design/ui-standard.md`,
a rail with no shortcut digits, Home/End/PageUp/PageDown on every arrow-navigable list, a browser
that searches on open with a Minecraft-version dropdown, and a settings editor with no layer word,
the raw key/value editor removed, and a native folder picker for the app root. All work is
`gcl-ui`-only: no `gcl-core` change, so neither `api-verifier` nor `e2e-runner` runs.

## Files

| File | Change | Reason |
|---|---|---|
| `crates/gcl-ui/ui/theme.slint` | new tokens: `radius-control/-chip/-row/-card`, `accent-hover/-pressed`, `motion-fast/-base`, `pad-screen`, `gap-toolbar`, `font-size-section`, `font-weight-medium`; palette values from the standard; `radius` stays as the control radius | §1 tokens |
| `docs/design/ui-standard.md` | new doc: palette, shape, type, layout, controls, keyboard table | §1 the written standard |
| `.claude/skills/slint-ui/SKILL.md` | "Visual direction" points at the doc; keyboard section gains jump; component list gains Chooser | keep the skill in step |
| `crates/gcl-ui/ui/components/rail.slint` | `Entry` loses `index-text`; active bar, weight 600 | §2 rail |
| `crates/gcl-ui/src/keys.rs` | `jump(key, current, page, len) -> Option<i32>` plus unit tests | §3 jump keys |
| `crates/gcl-ui/ui/state.slint` | `Shell.jump`; `InstanceState.content_filter` + `filter_changed`; drop `SettingsState.default_set`, `new_root`, `save_root(string)`→ keep; add `choose_root` | §3, §4, §6 |
| `crates/gcl-ui/src/app.rs` | wire `Shell.jump` | §3 |
| `crates/gcl-ui/ui/screens/instance.slint` | `content_search_box`, jump wiring on the Content scope | §3, §4 |
| `crates/gcl-ui/src/screens/instance.rs` | full row list in `Shared`, `filter_content_rows`, `filter_changed`, `open()` reset | §4 search |
| `crates/gcl-ui/ui/screens/instances.slint`, `accounts.slint`, `browser.slint` | jump wiring on their list scopes | §3 jump |
| `crates/gcl-ui/ui/screens/browser.slint` | `mc_combo` replaces `mc_field`, debounced | §5 dropdown |
| `crates/gcl-ui/src/screens/browser.rs` | `open()` searches; `minecraft_options`/`minecraft_index` fill; `set_minecraft` | §5 |
| `crates/gcl-ui/ui/components/setting-row.slint` | drop the source `Text` | §6 settings |
| `crates/gcl-ui/ui/screens/settings.slint` | drop raw key fields; root becomes `Chooser` | §6 settings |
| `crates/gcl-ui/ui/components/chooser.slint` | new component | §6 chooser |
| `crates/gcl-ui/src/screens/settings.rs` | drop `default_set`; add `choose_root` via rfd | §6 |
| `Cargo.toml`, `crates/gcl-ui/Cargo.toml` | `rfd 0.16`, `xdg-portal` + `tokio` features | §6 |
| `crates/gcl-ui/tests/flow_settings.rs` | raw-default step seeds through core | tests |
| `crates/gcl-ui/tests/flow_instances.rs` | content search step | tests |
| `crates/gcl-ui/tests/flow_content.rs` | rows non-empty after open; `mc_combo` step | tests |
| `docs/SPEC.md` | reword R9.4 | accuracy |
| `docs/research/2026-09-07-ui-element-ids.md` | refresh from `scripts/list-slint-ids.sh` | ids changed |

## Steps

All steps go to slint-designer. Steps that share a file run in order: 1 → 2/3/4 (one agent) →
5 and 6 in parallel → 7 → 8.

### 1. Theme tokens and the written standard
Files: `theme.slint`, `docs/design/ui-standard.md`, `.claude/skills/slint-ui/SKILL.md`.
Proof: `slint-viewer --check --style fluent crates/gcl-ui/ui/app.slint` clean; `cargo check -p gcl-ui`.

### 2. Rail: drop the digits
Files: `rail.slint`. Proof: `slint-viewer --check`, `just test flow_instances`.

### 3. Jump keys
Files: `keys.rs`, `state.slint`, `app.rs`, four list scopes. Proof: `cargo nextest run -p gcl-ui jump`.

### 4. Instance Content tab search
Files: `instance.slint`, `instance.rs`, `state.slint`, `flow_instances.rs`.
Proof: `cargo nextest run -p gcl-ui filter_content`, `just test flow_instances`.

### 5. Browser: search on open, version dropdown
Files: `browser.slint`, `browser.rs`, `flow_content.rs`.
Proof: `cargo nextest run -p gcl-ui minecraft_options`, `just test flow_content`.

### 6. Settings: no layer word, raw fields gone, root chooser
Files: `setting-row.slint`, `settings.slint`, `chooser.slint`, `settings.rs`, `state.slint`,
both `Cargo.toml`, `flow_settings.rs`. Proof: `just deny`, `just test flow_settings`.

### 7. Docs
Files: `docs/SPEC.md`, `docs/research/2026-09-07-ui-element-ids.md`.
Proof: `scripts/list-slint-ids.sh` output matches the doc.

### 8. Final verification
`just check`, `just deny`, `xvfb-run -a just run-ui -- --screenshot <png>` and view it.

## Verification

`just check`, `just deny`, screenshot. No api-verifier or e2e-runner: no gcl-core or gcl-cli change.

## Risks

- Wider control column or new fonts can overflow fixed widths: the screenshot checks it.
- rfd's portal path pulls zbus; if `just deny` fails, fall back to the `gtk3` feature.
- Auto-search adds one call per open; the flow mocks already answer an empty query.
- Step 6 and its flow test update land together, or `just check` fails in between.
