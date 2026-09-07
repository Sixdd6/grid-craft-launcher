---
name: slint-ui
description: Slint UI conventions for gcl-ui — file layout, theme tokens, models and callbacks, event forwarding from tokio, preview workflow, and visual direction. Read before touching any .slint file or gcl-ui Rust.
---

## Layout

```
crates/gcl-ui/
  build.rs                  slint_build::compile_with_config("ui/app.slint", fluent style, embeds resources)
  ui/app.slint               AppWindow: rail, one mounted screen, every dialog, the toast host
  ui/theme.slint              global Theme: colors, radius, gap, pad, font sizes, row-height
  ui/types.slint               exported structs crossing the Rust boundary
  ui/state.slint               Shell global plus the per-screen *State globals
  ui/components/               Button, Card, ListRow, ProgressBar, ProgressPanel, SearchBox,
                                TabBar, Rail, ToastHost, Dialog and its Confirm/Prompt/Choice/
                                DeviceCode/CreateInstance variants
  ui/screens/                  instances.slint, instance.slint, browser.slint, accounts.slint,
                                settings.slint
  src/lib.rs                   `slint::include_modules!()` plus every module, so tests can
                                build the real AppWindow
  src/main.rs                  args (--smoke, --screenshot), logging::init, app::build, run
  src/logging.rs               tracing to <root>/logs/gui.log daily, stderr when GCL_LOG is set
  src/app.rs                   builds AppWindow, wires App/Shell callbacks, starts the forwarder
  src/bridge.rs                Bridge: runs a Launcher call off the UI thread
  src/events.rs                forwarder thread: batches core Events onto the UI thread
  src/launch_flow.rs           the one launch path: running flag, log tail, offline prompt
  src/state.rs                 RunState: which slugs have a game running
  src/keys.rs                  pure keyboard rules
  src/toasts.rs                the toast queue
  src/models/                  pure converters from gcl-core structs to Slint structs
  src/screens/*.rs              one module per screen, each exposing `wire(&window, &bridge, ...)`
```

## Rules

- Every color, spacing, and font size comes from `Theme`. Add a token before you need a literal.
- Screens are pure layout. They read a `*State` global (see below) and expose no logic beyond
  binding properties and forwarding callbacks.
- Data crosses the boundary as Slint `struct`s declared in `ui/types.slint` and exported. Rust
  builds them with the pure converters in `src/models/` and wraps a list in `ModelRc::new(VecModel::from(vec))`.
- No core call ever runs on the UI thread. A callback hands work to `Bridge::run` or
  `Bridge::run_with_error`, which spawns a plain thread, calls `Launcher`, and posts the result
  back with `upgrade_in_event_loop`.
- Never hold a `Launcher::config()` guard (`ConfigRead`) across another `Launcher` call: a call
  that needs the write lock — `update_config` or most instance and content methods — would
  deadlock against a guard the caller still holds. Read what you need, drop the guard, then call.
- `slint::Weak<AppWindow>` is the only handle a spawned closure captures. Never capture the strong
  `AppWindow`/component handle inside its own callback: that is a reference cycle.
- `ModelRc<T>` and `slint::Image` are UI-thread only. Build them inside the
  `upgrade_in_event_loop` closure (or on the UI thread generally), never on a worker thread.
- Progress: one `TaskRow { id, label, fraction, status, detail }` per active task in `App.tasks`.
  Update by id; a finished or failed row is aged out later, not dropped by the batch that ended it.

## State globals

Each screen is mounted behind `if App.screen == Screen.x: XScreen { }` in `app.slint`, so Rust has
no handle into it once mounted. The fix is a global per screen — `InstancesState`, `InstanceState`,
`AccountsState`, `SettingsState`, declared in `ui/state.slint` or `ui/app.slint` — that both the
screen and `src/screens/x.rs` reach: the screen binds its layout to the global, and `x.rs` calls
`window.global::<XState>()` to read properties, set them, and answer callbacks. `Shell` (in
`state.slint`) is the one global every screen may import directly, for the two services every
screen needs: `Shell.toast(text, kind)` and `Shell.move_selection(current, delta, len)`. It cannot
live in `app.slint` because `app.slint` imports the screens, and a screen importing `app.slint`
back would cycle.

Give every `*State` property a realistic default. That default is what `just ui-preview
screens/x.slint` renders with no Rust running — it is the whole point of the preview workflow, so
an empty or placeholder default defeats it.

## Bridge

`Bridge::new(launcher: Arc<Launcher>, weak: Weak<AppWindow>)` is cheap to clone and shared across
screens. Two entry points:

- `Bridge::run(label, job, done)`: `job(&Launcher) -> Result<T, gcl_core::Error>` runs on a new
  thread; on `Ok`, `done(&AppWindow, T)` runs on the UI thread; on `Err`, the shared error dialog
  opens with `label` as its title and `done` never runs.
- `Bridge::run_with_error(label, job, done)`: the same threading, but `done(&AppWindow,
  Result<T, Error>)` always runs, after the error dialog opens on the `Err` path. Use this
  whenever a screen sets a "loading"/"busy" flag before the call — it needs to be cleared either
  way.

`bridge::error_chain(err)` joins an error and every `source()` under it with `": "`, the same
shape the CLI prints. `bridge::warn(window, text)` appends a warning to `App.app_log` and stacks a
toast — every warning a screen raises (a manual-download note, a launch that exited non-zero)
goes through this one function.

## Events and toasts

`events::start_forwarder(rx, weak)` owns the receiver end of the core event channel. It blocks for
the first event, sleeps 50 ms collecting the rest of the batch, then posts one closure that folds
the whole batch into `App.tasks` and `App.app_log` in a single redraw (`events::apply` is the pure
fold; test it directly, no window needed). A finished or failed row is stamped with its end time in
a `thread_local!` map and pruned by the one-second `Timer` in `src/app.rs` once its own life has
passed — `KEEP_DONE` (5 s) for a clean end, `FAILED_TTL` (30 s) for a failure — not by the batch
that ended it, so it stays visible. A failure is also said two other ways: `apply` appends an
`error` log line worded `"<label> failed: <detail>"` and reports it in `Applied::failures`, which
`push` shows as an `error` toast and uses to set `App.panel_expanded`, so the panel opens on the
row that carries it. `toasts.rs` follows the same push/prune split, `TTL` 6 s, `MAX` 3 live at
once. `RunState` (`src/state.rs`) is the one
piece of shared state that lives outside a `*State` global: an `Arc<Mutex<HashSet<String>>>` of
running slugs, so the instances list and the detail screen agree on "Running" regardless of which
one started the launch.

## Launching

`launch_flow::launch(bridge, run, slug, offline_user)` is the only way either screen starts a
game; neither keeps a copy. It marks the slug running in `RunState` and on both screens, calls
`launch_instance_async` on a thread, tails the game's log file into `InstanceState.game_log`
while the detail screen shows that slug, and at the end clears the running flag, posts the
outcome (a status line, plus a warning toast carrying the hint when the exit code is not zero),
refreshes `InstancesState.rows`, and reloads the detail screen. A launch that comes back
`auth::Error::NoAccount` is the one failure that does not open the error dialog: the shell
navigates to the detail screen and opens its prompt for an offline name, then calls back into
`launch` with it. The instances list opens that screen before launching for the same reason —
the prompt and the log view both belong to it.

`InstanceState`'s prompt is shared: `prompt_mode` is `"offline"` for that name and `"rename"`
for the detail header's Rename button, which prefills the current name and, on accept, calls
`instances().rename(slug, new_name)` in a job. The slug never changes, so only the header and
the list row have to be reloaded.

## Keyboard

A focused element sees a key first; the `FocusScope` in `app.slint` only gets what bubbles up, and
only calls `App.key_pressed` while `any_dialog_open` is false. `keys::key_to_screen` maps digits 1
to 5 to the five screens, matching the rail's own numbers. `keys::move_selection(current, delta,
len)` is the pure function behind every arrow-navigable list: clamps to `[0, len)` rather than
wrapping, so holding an arrow stops at an end; `-1` means no selection and a first press lands on
the near end. Both are ordinary Rust, unit-tested with no Slint instance. The `.slint` wiring that
calls them is checked by compiling the crate, not by a test that drives real key events.

No list scope ever calls `focus()` on `init`. A conditional element is rebuilt whenever its
condition changes, and an `init` handler that grabs focus would pull the keyboard out of a field
mid-typing. The keyboard reaches a list two ways instead: a click on a row, and `SearchBox.moved_down`,
which the field raises on Down so the owner can call `list.focus()`. That means a list scope has to
be mounted even while its list is empty — give it `vertical-stretch: 0` there and keep the `if` on
the `ListView` inside it. A `Dialog` takes focus when it opens, unless `focus-first-field` says the
caller focuses its own text field instead; Escape bubbles from that field to the dialog's scope
either way.

## Element ids

Every interactive element carries a `snake_case := ` name, because the Slint testing backend
addresses it as `<Component>::<name>`: `ElementHandle::find_by_element_id(&app,
"InstancesScreen::create_button")`. The component is the one whose *file* declares the name, so a
`Button` inside `Dialog` is `Dialog::confirm_button` no matter which screen opened the dialog.

- Screen controls: `<thing>_<kind>` — `create_button`, `refresh_button`, `search_box`,
  `java_path_field`, `loader_combo`.
- Dialog actions: `confirm_button` and `cancel_button` on `Dialog`, `name_field` for a prompt.
- Rows in a repeater: `row_open`, `row_launch`, `row_delete`, `row_select`, `row_refresh`,
  `row_toggle`, `row_install`. Every instance of a repeated element gets the same id, so a test
  takes the nth handle in list order.
- Rail entries: `rail_instances`, `rail_instance`, `rail_browser`, `rail_accounts`,
  `rail_settings`. Tab entries: `tab_entry` on `TabBar`, one per tab.

`Button`, `ListRow`, the rail entry, the tab entry, and the task panel's chevron each set
`accessible-role: button`, `accessible-label`, and `accessible-action-default`, so a test presses
them with `invoke_accessible_default_action`. A new clickable `Rectangle` needs the same three
lines; without them the element is found but cannot be pressed.

`scripts/list-slint-ids.sh` prints the whole table; `docs/research/2026-09-07-ui-element-ids.md`
holds its output. Re-run it after adding a control.

## Logging

`logging::init(root)` returns a `LogGuard` that `main` holds to the end: dropping it stops the
non-blocking writer's worker thread and loses the tail of the log. The file is
`<root>/logs/gui.log`, rotated daily, at `info`. Setting `GCL_LOG` adds a stderr layer with that
value as its `EnvFilter` directive, e.g. `GCL_LOG=debug`. `Bridge` writes one line per job —
`label`, `elapsed_ms`, and the error chain on failure — and the error dialog ends with
`Details: <root>/logs/gui.log.<date>`. `tracing_appender` appends the UTC date to the name, so
`logging::log_file(root)` rebuilds it; do not hard-code `gui.log`.

## Preview

`just ui-preview screens/instances.slint` opens slint-viewer with live reload against
`ui/screens/instances.slint`, rendered with each `*State` global's default property values. Every
screen file has one: `screens/instances.slint`, `screens/instance.slint`, `screens/browser.slint`,
`screens/accounts.slint`, `screens/settings.slint`. Give a changed screen realistic defaults before
previewing, and describe what you saw in your report — the tool has no snapshot output.

## Visual direction

- Dark first: `Theme.bg` is a dark background; the shell, rail, and screens follow it. Dense
  rows, sized from `Theme.row-height` (32 to 36 px). Left rail (`components/rail.slint`) for
  navigation, screen content on the right.
- One accent color for primary actions and selection. A separate danger color marks only
  destructive actions (remove, delete).
- Text over icons. No gradients; shadows kept to one level; corner radius from `Theme.radius`.
- Show state in place: a task's progress bar lives in `ProgressPanel`, in the row for that task,
  not in a modal. The instance list and detail screen mark a running game in place, through
  `RunState`, rather than a separate "now playing" panel.
- Every list is arrow-navigable through `Shell.move_selection`; Enter activates a selected row;
  Escape closes the open dialog. See "Keyboard" above.
- `std-widgets` controls follow the dark shell through `Palette.color-scheme` — see "Known
  limitations".

## Known limitations

- **Widget theme**: `std-widgets.slint` controls read their colors from the style's own
  `Palette`, which no `Theme` token reaches. `AppWindow`'s `init` sets
  `Palette.color-scheme = ColorScheme.dark`, which is the one switch that makes them match the
  dark shell. It has to be an assignment in `init`; `Palette.color-scheme: ...` in a component
  body is a parse error. `Theme` stays the source of truth for our own components.
- **No clipboard**: Slint 1.17 has no clipboard call reachable from a button here. Anywhere a user
  might want to copy text (the error dialog's body, for one) uses a read-only, selectable
  `TextEdit` instead — Ctrl+C on a selection is the whole copy story.
- **Modpack discovery is install-by-id or by path**: the browser installs a modpack once its
  source and project id are known, and its "From file" row imports a `.mrpack` or CurseForge
  zip already on disk (`import_modpack_file`). There is no in-app modpack *search* flow,
  because `gcl-core` has no modpack search endpoint, only project lookup and install. There is
  no file picker either: the path is typed into a `LineEdit`, the same way a hand-downloaded
  file is named on the detail screen.
- **`stop()` is disabled**: `InstanceState.stop` exists so the button has a place to grow into, but
  it is a no-op that only reports why — `gcl-core` has no way to kill a running launch.
- **Verified by compile, not by hand**: keyboard routing is exercised through unit tests on the
  pure functions and a passing build, not a live keyboard session. The CurseForge and Microsoft
  sign-in flows in the accounts and browser screens are unverified live on this machine — they
  compile and their pure logic is tested, but no one has clicked through a real CurseForge search
  or a real Microsoft device-code login in the built app here.

## Do not

- Do not call a `Launcher` method directly from a callback body. Go through `Bridge::run` or
  `Bridge::run_with_error`.
- Do not hold a `ConfigRead` guard while making another `Launcher` call.
- Do not build a `ModelRc` or `Image` outside the UI thread.
- Do not capture a strong `AppWindow`/component handle inside one of its own callbacks.
- Do not put logic in a `.slint` screen file. If it is not layout or a direct property/callback
  binding, it belongs in `src/screens/*.rs`.
- Do not add a color, spacing, or font literal outside `Theme`.
- Do not skip a realistic default on a `*State` property — it breaks the preview workflow for
  everyone after you.

## Docs

Slint language and API: https://docs.slint.dev/latest/docs/slint/ (use context7 for lookups).
Cargo features in use: `std`, `backend-winit`, `renderer-femtovg`, `compat-1-2`.
