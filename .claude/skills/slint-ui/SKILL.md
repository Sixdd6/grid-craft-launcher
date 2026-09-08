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
                                SettingRow, SettingsEditor, TabBar, Rail, ToastHost, Dialog and
                                its Confirm/Prompt/Choice/DeviceCode/CreateInstance variants,
                                BlockList (renders a `Block` list, shared by the Description tab
                                and NotesDialog), NotesDialog (a version's changelog in a modal
                                built on `Dialog`)
  ui/screens/                  instances.slint, instance.slint, browser.slint, accounts.slint,
                                settings.slint, project.slint
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
  src/screens/settings_editor.rs the typed game-settings editor, shared by the settings screen
                                (preseed layer) and the instance Settings tab (override layer)
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
`AccountsState`, `SettingsState`, and `SettingsEditorState` for the typed settings editor both the
settings screen and the instance Settings tab mount, declared in `ui/state.slint` or
`ui/app.slint` — that both the
screen and `src/screens/x.rs` reach: the screen binds its layout to the global, and `x.rs` calls
`window.global::<XState>()` to read properties, set them, and answer callbacks. `Shell` (in
`state.slint`) is the one global every screen may import directly, for the one service every
screen needs: `Shell.move_selection(current, delta, len)`. A toast is pushed from Rust through
`bridge::warn`, not from a `.slint` file. It cannot
live in `app.slint` because `app.slint` imports the screens, and a screen importing `app.slint`
back would cycle.

Every `*State` property starts empty: strings `""`, numbers `0`, bools `false`, arrays `[]`. What
the app shows must be what Rust put there, and a default carrying sample data is on screen until
the first read answers — that is how a never-launched instance came to show two lines of someone
else's game log. The exception is a value that is a real default rather than data: the labels
`prompt_title`, `prompt_label`, `prompt_accept`, `layer_name`, `kind_labels`, `loader_options`,
`page_size`, plus `autoscroll: true`, which is how the Logs tab starts, and
`BrowserState.target_index: -1`, which is "no target picked" and is not the same as row `0`.

The other half of that rule is `open`: every `open`/`load` in `src/screens/*.rs` sets every
property it owns, the empty case included, so a screen never shows the instance before it. After
adding a property, grep for its setter.

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

## A screen that has to come back: `ProjectState.return_to`

There is no navigation back stack. `App.navigate` sets `App.screen` and nothing else. Five of the
six screens are reached from the rail, so they need none. The project details screen is the
exception: it opens from a browser search row's title *and* from an installed content row's
source button, and Back has to go to whichever it was.

The pattern, and the one to copy if another screen ever needs the same:

- `ProjectState` carries `return_to: Screen`. Whoever opens the screen sets it —
  `Screen.browser` from the browser row, `Screen.instance` from the instance content row — as
  one more argument of `ProjectState.open(source, project_id, target_slug, return_to, ...)`.
- `ProjectState.back()` sets `App.screen` to `return_to`. The rail highlight reads `return_to`
  too, so the rail stays lit on the screen the user thinks they are in.
- Opening from the instance screen leaves `App.current_slug` alone, so the details screen still
  installs into the right instance and Back lands on the same instance.
- No stack, no history list, and no other screen changes. Add the property, not a mechanism.

`BrowserScreen` cannot call `ProjectState.open` itself: `app.slint` imports the screens, so the
reverse import would cycle. The screen declares an `open_project(...)` callback instead, and
`app.slint` wires it to `ProjectState.open` plus `App.navigate` — the same seam
`InstanceScreen.open_project` uses.

## BlockList and per-open image generation counters

`BlockList` (`ui/components/block-list.slint`) renders a `[Block]` — heading, paragraph, bullet
(indented by `depth`), code, table (`columns`/`cells`), rule, quote, and image — as pure layout,
with no fetch state of its own. `ProjectScreen`'s Description tab and `NotesDialog` both mount
one. A `Block.image_state` of `"loading"`/`"ready"`/`"failed"` and the decoded `Block.image` are
filled in by `src/screens/project.rs`, not by `BlockList`: the component only draws what it is
given.

An image inside a description or a changelog is fetched and decoded off the UI thread, one at a
time, capped at 20 per open. Two things can make a stale result show up: the user opens a
different project while a fetch is still running, or opens a different version's Notes modal
while its changelog is still loading. `project.rs` guards both with a generation counter, the
same pattern `browser.rs` already uses for search-row icons (`icon_generation`, bumped in
`load_hits`, checked in `fetch_icons`) and for each row's latest version and install state
(`latest_generation`, bumped in `search()` beside `icon_generation` and again on a target
change, checked in `fetch_latest`):

```rust
// project.rs: one AtomicU64 per thing that can go stale, held in Shared
image_generation: Arc<AtomicU64>,   // description images
notes_generation: Arc<AtomicU64>,   // the Notes modal's own open/close/changelog fetch
```

Opening a project (or a version's notes) bumps its counter and captures the new value as
`generation`; the fetch loop checks `counter.load(Ordering::SeqCst) != generation` both before
it starts the next image and again inside the `upgrade_in_event_loop` closure that paints one
image onto `ProjectState.blocks`/`notes_blocks`, so a closure that lands after the user has
navigated away is a no-op instead of a write to the wrong screen. Closing the modal also bumps
`notes_generation`, so a changelog fetch already in flight paints nothing once dismissed. Copy
this shape — bump on open/close, capture once, check on every posted closure — rather than
inventing a new "is this still wanted" flag when a third caller needs the same guarantee.

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

The detail screen's Stop button calls `Launcher::stop_instance(slug)` through
`Bridge::run_with_error`, so the busy flag clears on both paths. Core asks the game to exit,
waits `STOP_GRACE` (10 s), then kills it; the outcome comes back as `LaunchOutcome::Exited`
with `stopped` set, which the screen shows as "Stopped" and raises no warning toast for — a
requested exit is not a crash.

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

Two things keep the shell's shortcuts alive. `AppWindow` mirrors `App.screen` into its own
`shown_screen` property and calls `nav.focus()` from `changed shown_screen`, because a screen
change destroys whatever field held the keyboard and nothing else takes it back. And
`changed any_dialog_open` does the same when the last dialog closes. A dialog owns the keyboard
while it is up, so the screen handler leaves it alone.

## Dialogs

Every dialog in `app.slint` is mounted inside an `if <State>.xxx_open:`, so a closed dialog has
no element in the tree at all. A dialog that stayed mounted kept a full-window overlay
`TouchArea`, and that area holds the pointer grab it took when the dialog opened, so the first
click after an Escape landed on it instead of the screen. `visible: root.open` is not enough:
`Dialog` also gates both of its touch areas on `enabled: root.open`, for a caller that keeps the
component mounted, which is what the previews do.

The error dialog is mounted last in `app.slint`, after every other dialog. A dialog mounted
later draws over one mounted earlier, and each dialog's full-window overlay `TouchArea` catches
every click while it is up — so a dialog placed after the error dialog can swallow a click meant
for it, and the error dialog becomes undismissable while that other dialog is open. `NotesDialog`
learned this the hard way: appended after the error dialog, its own overlay ate the click on
Dismiss. Add a new dialog above the error dialog, never below it.

Being mounted only while open changes how a dialog takes the keyboard: it is created already
open, so `changed open` never fires. `Dialog`, `PromptDialog`, and `CreateInstanceDialog` each
call the same one-line `take-keyboard()` function from `init` and from `changed open`. Write a
new dialog the same way — an `init` handler alone breaks the preview, a `changed open` handler
alone breaks the app.

A control that says "no" must be gone, not dimmed: the Stop button on the instance screen is
`visible: InstanceState.running`. A disabled red button still reads as one to press.

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
lines; without them the element is found but cannot be pressed. A row that carries a value —
a settings row, for one — also sets `accessible-value`, so a test reads the row without knowing
where it sits in the list.

`scripts/list-slint-ids.sh` prints the whole table; `docs/research/2026-09-07-ui-element-ids.md`
holds its output. Re-run it after adding a control. It skips `:= Timer`: a timer never enters
the element tree, so `find_by_element_id` can never answer with one. Name your timers anyway —
a name is what a `restart()` call needs.

## Flow tests

`crates/gcl-ui/tests/support/mod.rs` builds the real `AppWindow` over a real `Launcher` on a
temp root and clicks through it. See the `testing` skill for the harness contract. What matters
while writing a screen:

- `app.click(id)` fails when the control is disabled, which is the point. `app.type_into(id,
  text)`, `app.drag_slider(id, fraction)`, `app.el(id)` / `el_nth(id, n)` for a repeater.
- `app.wait_until(what, pred, timeout)` yields to the event loop until `pred` holds, so a
  `Bridge` worker thread posts its result back exactly as it does in the app. Never assert
  straight after a click that starts a job.
- `app.select_combo(id, index)` opens the popup with `accessible-action-expand` and drives it
  with arrow keys, because a `ComboBox` exposes no accessible set-value action. It presses Up
  until `accessible_value` stops changing, then Down `index` times.
- Only what is drawn is in the element tree. A control below the fold of a `ScrollView` needs
  `app.scroll_to(id)` first: it scrolls toward the element until the whole rectangle is inside
  the viewport, and `click` refuses an element that is not.
- `crates/gcl-ui/build.rs` gates element names on `PROFILE == "debug"`, not `DEBUG`: `DEBUG`
  reports the debug-info level, so a release profile with debug info on would ship the names.
  `SLINT_EMIT_DEBUG_INFO=1` forces them.

## Debounce

A control that reports every intermediate step — a `Slider` under a screen reader's increment,
a `ComboBox` under arrow keys — must not save once per step: each save reloads the rows, which
tears the popup down under the user. Hold the pending value in a property and restart a named
`Timer`:

```slint
// in the control's own `changed` handler
root.pending_choice = self.current-index;
choice_commit.running = true;
choice_commit.restart();

choice_commit := Timer {
    interval: root.commit_delay;   // 500ms
    running: false;
    triggered => { self.running = false; root.chose(root.entry.key, root.pending_choice); }
}
```

`running = true` starts a stopped timer; `restart()` is what puts the delay back. Assigning
`running` the value it already holds is a no-op, so `running = false; running = true` does not
rearm anything — that bug shipped once and made the timer fire 500 ms after the *first* change
instead of the last. Always end with `restart()`.

**Not every `ComboBox` needs this.** The instance JVM tab's `gc_combo` saves straight through
`set_instance_gc` on its `selected` callback, no `Timer` and no Save button — one selection is
one write, unlike the min/max heap fields next to it, which only save when their own Save button
is pressed. Debounce a control that reports intermediate steps on the way to a value; a
`ComboBox` that reports the chosen value once has nothing to debounce.

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
`ui/screens/instances.slint`. The globals start empty, so the sample data lives in one
**`Preview<Screen>` component per screen file**, last in the file: slint-viewer with no
`--component` renders the last exported component, and that component's `init` fills the globals
with the values the screen used to declare as defaults. `PreviewInstancesScreen`,
`PreviewInstanceScreen`, `PreviewBrowserScreen`, `PreviewAccountsScreen`, `PreviewSettingsScreen`.
Nothing in the app instantiates one. Add a screen, add its preview; add a `*State` property that
shows anything, give the preview a value for it.

`slint-viewer --check --style fluent ui/screens/x.slint` compiles one file and prints its
diagnostics without opening a window — the fastest check after editing a `.slint` file.
`--screenshot out.png` renders one instead, which needs a display or `xvfb-run`. Describe what you
saw in your report.

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
- `std-widgets` controls follow the dark shell through `Palette.color-scheme =
  ColorScheme.dark`, set in `AppWindow`'s `init` — combo boxes, text fields and spin boxes
  render dark, not in the `fluent` style's light palette.

## Known limitations

- **Widget theme**: `std-widgets.slint` controls read their colors from the style's own
  `Palette`, which no `Theme` token reaches. `AppWindow`'s `init` sets
  `Palette.color-scheme = ColorScheme.dark`, which is the one switch that makes them match the
  dark shell, and they now render dark. It has to be an assignment in `init`;
  `Palette.color-scheme: ...` in a component body is a parse error. `Theme` stays the source of
  truth for our own components.
- **No clipboard**: Slint 1.17 has no clipboard call reachable from a button here. Anywhere a user
  might want to copy text (the error dialog's body, for one) uses a read-only, selectable
  `TextEdit` instead — Ctrl+C on a selection is the whole copy story.
- **No file picker**: a modpack archive on disk is named by typing its path into a `LineEdit`,
  the same way a hand-downloaded file is named on the detail screen. Modpack *search* works —
  the browser's modpack kind lists packs through `Launcher::search_packs`, with an Install
  button per row, next to the by-id and by-file panels.
- **A debounce timer cannot be driven from a flow test**: `TestApp::pump` hands the loop no
  time, and the system-time backend refuses `mock_elapsed_time` with a real duration. A flow
  test drags the slider instead (`TestApp::drag_slider`), which saves on release, and asserts
  the debounced path in a unit test on the converter.
- **Verified by compile, not by hand**: keyboard routing is exercised through unit tests on the
  pure functions and a passing build, not a live keyboard session. The CurseForge and Microsoft
  sign-in flows in the accounts and browser screens are unverified live on this machine — they
  compile, their pure logic is tested, and `flow_accounts` drives the device-code dialog against
  a mock, but no one has clicked through a real CurseForge search or a real Microsoft login here.
- **`--screenshot` needs a compositor that draws**: a Wayland session gives an unmapped or
  occluded window no frame callback, so the `AfterRendering` notifier never fires and the run
  hangs. Run it under `xvfb-run -a` to get a PNG every time.

## Do not

- Do not call a `Launcher` method directly from a callback body. Go through `Bridge::run` or
  `Bridge::run_with_error`.
- Do not hold a `ConfigRead` guard while making another `Launcher` call.
- Do not build a `ModelRc` or `Image` outside the UI thread.
- Do not capture a strong `AppWindow`/component handle inside one of its own callbacks.
- Do not put logic in a `.slint` screen file. If it is not layout or a direct property/callback
  binding, it belongs in `src/screens/*.rs`.
- Do not add a color, spacing, or font literal outside `Theme`.
- Do not give a `*State` property a non-empty default, outside the exception list above. Sample
  data goes in that screen's `Preview<Screen>` component.

## Docs

Slint language and API: https://docs.slint.dev/latest/docs/slint/ (use context7 for lookups).
Cargo features in use: `std`, `backend-winit`, `renderer-femtovg`, `compat-1-2`.
