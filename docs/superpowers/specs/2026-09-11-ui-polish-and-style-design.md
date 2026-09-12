# UI polish and a look-and-feel standard: design

Date: 2026-09-11. Status: written for the user's review; work started under it because the
session runs unattended.

## Goals

1. A written look-and-feel standard for `gcl-ui`, based loosely on GRID Launcher
   (`../grid-launcher`, read-only reference), expressed as `Theme` tokens and component rules.
2. The rail shows no shortcut digits. Digits 1 to 5 still work.
3. The instance screen's Content tab gets a search box and Home/End/PageUp/PageDown. The
   same four keys work on every other arrow-navigable list.
4. The browser searches on open with an empty query, and the Minecraft version filter is a
   dropdown.
5. The settings editor shows no layer word ("default", "preseed"). Reset shows only when the
   value is non-default. The raw key/value fields leave the Settings screen. The app root is a
   chooser: the current value plus a button that opens the OS folder picker.

Requirement ids: R13.1, R13.3, R13.4, R9.4.

## 1. Look-and-feel standard

The standard lives in `docs/design/ui-standard.md` and is summarised in the `slint-ui`
skill's "Visual direction" section, which points at the doc. `Theme` in `ui/theme.slint` is
the only place a value is written; the doc names each token.

### Palette (dark, the only theme)

| Token | Value | Reference |
|---|---|---|
| `bg` | `#07070f` | grid-launcher `--bg` dark |
| `surface` | `#14141f` | `--surface-2` (opaque panels: rail, cards, dialogs, fields) |
| `surface-raised` | `#1c1c2b` | one step up, for a dialog over a card |
| `surface-hover` | `#ffffff` at 7 % alpha | `--surface` (hover and pressed tint) |
| `surface-alt` | `#ffffff` at 3 % alpha | odd list rows |
| `border` | `#22223a` | `--border` |
| `text` | `#ffffff` | `--text` |
| `text-muted` | `#c8c8dc` | `--text-muted` |
| `accent` | `#8b74e8` | `--primary` |
| `accent-hover` | `#a18fff` | `--primary-hover` (new token) |
| `accent-pressed` | `#6043c8` | `--primary-pressed` (new token) |
| `accent-text` | `#ffffff` | white on the accent |
| `danger` | `#ff5050` | `--danger` |
| `success` | `#4ade80` | `--success` |
| `warning` | `#fbbf24` | `--warning` |
| `focus` | `accent-hover` | focus ring |

No gradients. One shadow level, `0 12px 32px` black at 35 %, on dialogs and toasts only.

### Shape and motion

| Token | Value | Use |
|---|---|---|
| `radius-control` | 4 px | buttons, fields, combo boxes |
| `radius-chip` | 6 px | chips, badges, icon buttons |
| `radius-row` | 8 px | list rows, rail entries, toasts |
| `radius-card` | 14 px | cards, dialog panels |
| `radius` | alias of `radius-control` | kept so existing files compile; new code names the specific one |
| `motion-fast` | 150 ms | hover and selection tints |
| `motion-base` | 220 ms | panel open and close, toast in |

Borders are 1 px `border`. Rail has a 1 px right border. Toolbars have a 1 px bottom border.

### Type

| Token | Value |
|---|---|
| `font-size` | 13 px |
| `font-size-small` | 11 px |
| `font-size-title` | 20 px, weight 600 (screen and dialog titles) |
| `font-size-h1` | 24 px (unused by the standard, kept) |
| `font-size-section` | 10 px, weight 700, uppercase, letter-spacing 0.1 em (group headings) |
| `font-weight-medium` | 600 (new; titles, active rail entry, toast text, chips) |

Font family is the system UI font. Numbers in a column use tabular figures where Slint
allows it. Line height stays Slint's default.

### Layout

- Rail: `rail-width` 220 px, `surface` background, right border. Entries are
  `row-height` (34 px) tall with `radius-row`, padding 10 px horizontal, label only.
  Active entry: `surface-hover` background and weight 600. Hover: `surface-hover`.
  A 3 px accent bar on the left edge of the active entry, `accent-stripe` wide.
- Screen content padding: `pad-screen` 24 px (new token). A screen starts with a title
  row (title on the left, primary actions on the right), then a toolbar row (search box,
  filters), then the list. Toolbar gap `gap-toolbar` 12 px (new).
- Rows keep `row-height`, stripe on odd rows, hover and selection tints win over the stripe.
  Selection is `accent` at 18 % alpha, plus the 3 px accent bar on the left.

### Controls

- Button variants: `primary` (accent background, white text, hover `accent-hover`, pressed
  `accent-pressed`), `secondary` (transparent, 1 px border, hover `surface-hover`),
  `danger` (transparent, border and text `danger`), `ghost` (no border, muted text, hover
  `surface-hover`; for row actions). Padding 8 px vertical, 16 px horizontal. Disabled is
  60 % opacity. A control that says "no" is gone, not disabled (existing rule).
- Fields and combo boxes: `surface` background, 1 px border, `radius-control`, padding
  6 px 10 px. Focus: border `focus`.
- Search box: a field with the placeholder "Search…" and a clear affordance on Escape,
  `min-width` 240 px, first in the toolbar.
- Chip: `radius-chip`, `surface-hover` background, 1 px border, `font-size-small` weight 600.
- Toast: `surface` background, 1 px border, `radius-row`, one shadow, `font-size` weight 600,
  the kind stripe on the left. Error toasts colour the text `danger`.
- Dialog: `surface` panel, 1 px border, `radius-card`, 24 px padding, one shadow, backdrop
  black at 55 %. Title 20 px weight 600.
- Chooser (new): a read-only value line in muted text plus a `secondary` "Choose…" button.
  Used for the app root. A typed path field stays where a picker cannot apply (an archive
  URL, a Java path stays typed for now).

### Keyboard

Every list scope handles Up, Down, Home, End, PageUp, PageDown, Enter, and Escape the
same way through `Shell.move_selection` and `Shell.jump`. Digits 1 to 5 switch screens and
are not shown in the rail. The doc lists this table.

### Words

Text over icons, as before. Labels are one or two words. A layer name never shows in the
settings editor. Status lines are one sentence.

## 2. Rail

`components/rail.slint`: `Entry` loses `index-text` and its `Text`. Padding and the active
style follow section 1. `keys::key_to_screen` is unchanged. The Entry ids stay.

## 3. Lists: search on the instance Content tab, and jump keys

### Search

- `InstanceState.content_filter: string` (starts `""`). `instance.slint` mounts
  `content_search_box := SearchBox` at the top of the Content tab, bound two-way to it, with
  `moved_down => content_keys.focus()`.
- Filtering is in Rust, as `instances.rs` does: `instance.rs` keeps the full row list in its
  `Shared` state and sets `InstanceState.content` to the rows whose title or file name
  contains the filter, case-insensitive. `content_filter` changes through a
  `changed content_filter` callback the screen forwards as
  `InstanceState.filter_changed()`. Every in-place row patch (toggle, remove, add refresh)
  patches the full list and re-applies the filter, so a filtered list never shows a stale row.
- `open()` resets `content_filter` to `""`.

### Jump keys

- `keys::jump(key: &str, current, page, len) -> Option<i32>`: `Home` gives `0`, `End` gives
  `len - 1`, `PageUp` gives `move_selection(current, -page, len)`, `PageDown` gives
  `move_selection(current, page, len)`, anything else `None`. Empty list gives `-1`. Unit
  tests in `keys.rs`.
- `Shell.jump(current, key, page, len) -> int` wraps it; `-1` also means "not a jump key",
  so the scope tests the key first: `if (event.text == Key.Home || … )`.
- `page` is the number of rows the list shows: `floor(list.visible-height / Theme.row-height)`,
  at least 1.
- Every list scope (instance content, instances, browser results, accounts) handles the four
  keys and, after any selection change, sets `viewport-y` so the selected row is inside the
  viewport (a one-line helper in each scope; Slint has no scroll-to-row).

## 4. Browser

### Search on open

`browser.rs::open()` runs the current kind's search (`search()` or `search_packs()`) once the
sources and targets have loaded, unless `returning` is set (Back from the project screen keeps
the rows). The query is whatever `BrowserState.query` holds, normally `""`. The existing
generation counters guard a stale answer.

### Minecraft version dropdown

- `BrowserState.minecraft_options: [string]` (starts `[]`), `minecraft_index: int` (starts
  `0`). Option 0 is `"any"`. Rust fills the rest in `open()` from `Launcher::list_versions()`:
  release versions newest first. When the target instance's version is a snapshot not in that
  list, it is inserted after `"any"`.
- `mc_combo := ComboBox` replaces `mc_field`. `selected` holds the pending index and restarts a
  named 500 ms timer, per the skill's debounce rule; the timer calls
  `BrowserState.set_minecraft(index)`, which sets `minecraft` (the string the search already
  reads; `"any"` maps to `""`) and runs the search.
- `open()` prefills `minecraft_index` from the target instance the way it prefills the string
  today. A target change re-prefills it.
- The manifest read is a `Bridge` job; until it answers the combo shows only `"any"` plus the
  prefilled version.

## 5. Settings

- `setting-row.slint`: the source `Text` goes. Reset stays gated on `entry.resettable`. The
  column width it freed goes to the control.
- `SettingRowModel.source` stays in `types.slint` (flow tests read it, and it is free), but
  nothing draws it.
- `settings.slint`: `default_key_field`, `default_value_field`, and `default_add_button` go,
  with `new_key` / `new_value`. `SettingsState.default_set` and its Rust handler go too. The
  instance screen's `override_*` fields are out of scope and stay.
- App root: `root_path_field` and `root_change_button` are replaced by a `Chooser`
  component (`components/chooser.slint`): value line `SettingsState.root_path`, button
  `root_choose_button` "Choose…", status line `root_change_status` as today. The button calls
  `SettingsState.choose_root()`.
- `choose_root` in `settings.rs` runs `rfd::FileDialog::new().set_directory(current).pick_folder()`
  on a `Bridge` thread. `Some(path)` goes through the existing `save_root`. `None` (cancelled,
  or no portal) does nothing. The dialog title is "Choose app root". A picker failure is not an
  error: nothing pops.
- `rfd` 0.16 joins the workspace with `default-features = false, features = ["xdg-portal",
  "tokio"]` so it reuses the tokio runtime and needs no GTK. `just deny` must pass.

## Testing

- `keys.rs`: unit tests for `jump` on each key, at both ends, on an empty list.
- `instance.rs` tests: the content filter matches title and file name, case-insensitive, and
  an in-place patch survives a filter.
- `browser.rs` tests: version options are releases newest first, `"any"` first, a snapshot
  target inserted; index and string stay in step.
- Flow tests: `flow_settings` stops reading `default_key_field`; its raw-default step uses the
  CLI-equivalent core call to seed the raw key and asserts the editor still shows it.
  `flow_content` and `flow_project` gain no browser step for the auto-search beyond an
  assertion that rows are non-empty after `wait_for_browser`. A new step drives `mc_combo`
  through `select_combo`. `flow_instances` types into `content_search_box` and asserts the
  row count.
- No test drives the folder picker.
- `just check`, `just deny`, `scripts/list-slint-ids.sh` to refresh the id table.

## Risks

- A theme change can widen text past a fixed width; check the browser header columns and the
  rail in a screenshot under `xvfb-run`.
- `rfd`'s portal path pulls `zbus`; if `just deny` flags a licence, fall back to `gtk3`
  feature or drop the chooser to a typed field with a note.
- The auto-search on open adds one network call per open; the mock in flow tests already
  answers an empty query.
