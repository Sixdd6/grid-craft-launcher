# gcl-ui look-and-feel standard

Date: 2026-09-11. Based loosely on GRID Launcher (`../grid-launcher`, read-only reference).

`Theme` in `crates/gcl-ui/ui/theme.slint` is the only place a value is written. This doc names
each token and the rule it serves. A screen or component reads a token; it never writes a
literal color, spacing, radius, duration, or font size.

## Palette (dark, the only theme)

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
| `accent-hover` | `#a18fff` | `--primary-hover` |
| `accent-pressed` | `#6043c8` | `--primary-pressed` |
| `accent-text` | `#ffffff` | white on the accent |
| `danger` | `#ff5050` | `--danger` |
| `success` | `#4ade80` | `--success` |
| `warning` | `#fbbf24` | `--warning` |
| `focus` | `accent-hover` | focus ring |

No gradients. One shadow level, `0 12px 32px` black at 35 %, on dialogs and toasts only.

## Shape and motion

| Token | Value | Use |
|---|---|---|
| `radius` | 4 px | buttons, fields, combo boxes (the control radius) |
| `radius-chip` | 6 px | chips, badges, icon buttons |
| `radius-row` | 8 px | list rows, rail entries, toasts |
| `radius-card` | 14 px | cards, dialog panels |
| `motion-fast` | 150 ms | hover and selection tints |
| `motion-base` | 220 ms | panel open and close, toast in |

`radius` is the token name in `theme.slint`; there is no separate `radius-control` token. It is
kept named `radius`, not renamed to match `radius-chip`/`radius-row`/`radius-card`, so existing
files compile. New code names the specific radius it wants (`radius-chip`, `radius-row`,
`radius-card`), or `radius` for a control.

Borders are 1 px `border`. Rail has a 1 px right border. Toolbars have a 1 px bottom border.

## Type

| Token | Value |
|---|---|
| `font-size` | 13 px |
| `font-size-small` | 11 px |
| `font-size-title` | 20 px, weight 600 (screen and dialog titles) |
| `font-size-h1` | 24 px (unused by the standard, kept) |
| `font-size-section` | 10 px, weight 700, uppercase, letter-spacing 0.1 em (group headings) |
| `font-weight-medium` | 600 (titles, active rail entry, toast text, chips) |

Font family is the system UI font. Numbers in a column use tabular figures where Slint
allows it. Line height stays Slint's default.

## Layout

- Rail: `rail-width` 220 px, `surface` background, right border. Entries are `row-height`
  (34 px) tall with `radius-row`, `pad-entry` (10 px) horizontal padding, label only — no
  shortcut digit.
  Active entry: `surface-hover` background and weight 600. Hover: `surface-hover`.
  A 3 px accent bar (`accent-stripe`) on the left edge of the active entry.
- Screen content padding: `pad-screen` 24 px. A screen starts with a title row (title on
  the left, primary actions on the right), then a toolbar row (search box, filters), then
  the list. Toolbar gap `gap-toolbar` 12 px.
- Rows keep `row-height`, stripe on odd rows, hover and selection tints win over the
  stripe. Selection is `accent` at 18 % alpha, plus the 3 px accent bar on the left.

## Controls

- Button variants:
  - `primary`: accent background, white text, hover `accent-hover`, pressed `accent-pressed`.
  - `secondary`: transparent, 1 px border, hover `surface-hover`.
  - `danger`: transparent, border and text `danger`.
  - `ghost`: no border, muted text, hover `surface-hover`; for row actions.
  - Padding 8 px vertical, 16 px horizontal. Disabled is 60 % opacity. A control that says
    "no" is gone, not disabled.
- Fields and combo boxes: `surface` background, 1 px border, `radius`, padding
  6 px 10 px. Focus: border `focus`.
- Search box: a field with the placeholder "Search…" and a clear affordance on Escape,
  `min-width` 240 px, first in the toolbar.
- Chip: `radius-chip`, `surface-hover` background, 1 px border, `font-size-small` weight 600.
- Toast: `surface` background, 1 px border, `radius-row`, one shadow, `font-size` weight 600,
  the kind stripe on the left. Error toasts colour the text `danger`.
- Dialog: `surface` panel, 1 px border, `radius-card`, 24 px padding, one shadow, backdrop
  black at 55 %. Title 20 px weight 600.
- Chooser: a read-only value line in muted text plus a `secondary` "Choose…" button. Used
  for the app root. A typed path field stays where a picker cannot apply (an archive URL,
  a Java path stays typed for now).

## Keyboard

Every list scope handles the same eight keys the same way, through `Shell.move_selection`
and `Shell.jump`:

| Key | Effect |
|---|---|
| Up | move selection up one row |
| Down | move selection down one row |
| Home | select the first row |
| End | select the last row |
| PageUp | move selection up one page |
| PageDown | move selection down one page |
| Enter | activate the selected row |
| Escape | close the open dialog, or clear a search box |

Digits 1 to 5 switch screens and are not shown in the rail.

## Words

Text over icons, as before. Labels are one or two words. A layer name (`"default"`,
`"preseed"`) never shows in the settings editor. Status lines are one sentence.
