---
name: slint-ui
description: Slint UI conventions for gcl-ui — file layout, theme tokens, models and callbacks, event forwarding from tokio, preview workflow, and visual direction. Read before touching any .slint file or gcl-ui Rust.
---

## Layout

```
crates/gcl-ui/
  build.rs                  slint_build::compile("ui/app.slint")
  ui/app.slint              AppWindow: imports screens, holds navigation state
  ui/theme.slint            global Theme: colors, radius, gap, pad, font sizes
  ui/components/            Button, Card, ListRow, ProgressBar, SearchBox, TabBar
  ui/screens/               instances.slint, instance-detail.slint, browser.slint, accounts.slint, settings.slint
  src/main.rs               builds the window, wires callbacks, starts core
  src/models/               one file per screen: converts core structs to Slint structs and VecModel
  src/events.rs             receives core events on a channel, forwards with invoke_from_event_loop
```

## Rules

- Every color, spacing, and font size comes from `Theme`. Add a token before you need a literal.
- Screens are pure layout. They expose `in` properties for data and `callback`s for actions. No logic.
- Data crosses the boundary as Slint `struct`s declared in `ui/types.slint` and exported. Rust builds `Rc<VecModel<T>>` in `src/models/`.
- Long work never runs on the UI thread. Callbacks call a `Launcher` handle method that spawns on the tokio runtime and returns at once. Results arrive as events.
- Event forwarding: the tokio task sends `Event` on an `mpsc` channel; a receiver task calls `slint::invoke_from_event_loop(move || { ... update models ... })`. Hold a `slint::Weak<AppWindow>` in the task, upgrade inside the closure.
- Progress: one `TaskRow { id, label, fraction, status }` per active task in a `VecModel`. Update by id.

## Preview

`just ui-preview screens/instances.slint` opens slint-viewer with live reload. Give screens
default property values so the preview shows realistic content. Describe what you saw in the report.

## Visual direction

- Dark first. Dense rows, 32 to 36 px tall. Left rail for navigation, content on the right.
- One accent color for primary actions and selection. Danger color for delete only.
- Text over icons. No gradients, shadows kept to one level, radius from `Theme.radius`.
- Show state in place: progress bars in the row that is installing, not in a modal.
- Keyboard: every list is arrow-navigable, Enter activates, Escape closes panels.

## Docs

Slint language and API: https://docs.slint.dev/latest/docs/slint/ (use context7 for lookups).
Cargo features in use: `std`, `backend-winit`, `renderer-femtovg`, `compat-1-2`.
