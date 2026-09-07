# Usable app: design

Date: 2026-09-07. Status: approved in chat.

## Problem

The MVP GUI compiles, renders, and passes `--smoke`, but on the user's machine the create flow
produced no instance and several buttons appear dead. Root causes: the GUI has no logging, errors
surface only through one dialog, every screen was verified by preview and compile, and nobody has
driven the real window in a test. The settings editor is a raw key/value list where the user wants
typed controls like the CurseForge app.

## Goals

1. Every visible control does what its label says, and a headless test proves it.
2. A user can create an instance, install a loader, add a mod, edit settings, and launch, from the
   GUI alone, on Linux, with an offline account.
3. Game settings preseed and per-instance overrides use typed widgets for known `options.txt`
   keys (sliders, toggles, choices) and raw rows for unknown keys.
4. A running game can be stopped from the GUI.
5. Standard widgets render dark.
6. Modpacks can be searched, not only installed by id.

## Design

### GUI flow tests

`gcl-ui` becomes a library crate plus a thin binary. Integration tests under `crates/gcl-ui/tests/`
use `i-slint-backend-testing` (`init_integration_test_with_mock_time`), build the real
`AppWindow` over a `Launcher` on a temp root with wiremock endpoints, run the Slint event loop, and
drive the UI from a `slint::spawn_local` task through `ElementHandle::find_by_element_id`,
`invoke_accessible_default_action`, `set_accessible_value`, and the increment/expand actions.
Every interactive element gets a stable element name (`create_button`, `name_field`, ...). A
`wait_until(predicate, timeout)` helper polls with a Slint `Timer` so worker-thread results posted
by `Bridge` are observed. Flows: create → loader install → launch (a shell stand-in for Java) →
content add → settings edit → account add → rename/delete → modpack install by id and by file →
error dialog on failure. The tests replace the "verified by compile" claims.

### Diagnostics

`gcl-ui` initializes `tracing` to `<root>/logs/gui.log` (daily rotation) and to stderr when
`GCL_LOG` is set. Every `Bridge` job logs start, duration, and outcome. The error dialog shows the
error chain and the log path.

### Settings editor

`gcl-core::settings::catalog` describes known keys: `Setting { key, label, group, control:
Slider { min, max, step, format } | Toggle | Choice { values: [(stored, label)] } | Text, default,
since }`. A `SettingsDoc` merges three layers for display: Minecraft default → launcher preseed
(`config.game_defaults`) → instance override (`settings_overrides`), and reports the source of each
value. The GUI renders groups (Video, Controls, Sound, Chat, Other) with the control per kind and
an "inherited" marker; changing a control writes to the layer the screen edits (defaults in
Settings, overrides in the instance). Unknown keys from the instance's `options.txt` show as raw
rows. Values are stored exactly as Minecraft writes them (booleans `true`/`false`, floats with a
decimal point, enums by their stored token).

### Stop

`launch::RunningGame` exposes the child; `Launcher::launch_instance_async` keeps an
`Arc<Mutex<Option<Child>>>` per running slug in a registry and `Launcher::stop_instance(slug)`
sends SIGTERM (unix) / `kill()` (windows), waits up to 10 s, then SIGKILL. The wait task observes
the exit as usual. The GUI Stop button and `gcl stop <slug>` call it.

### Dark widgets

`AppWindow` sets `Palette.color-scheme: ColorScheme.dark`; `Theme` colors stay the source of
truth for custom components.

### Modpack search

`Source::search_packs(query) -> SearchPage` (Modrinth `project_type:modpack`; CurseForge class
modpacks); `Launcher::search_packs`; the browser's modpack kind lists results with "Install".

### Out of scope

Windows/macOS hands-on testing; Microsoft login live verification (no client id); CurseForge live
verification (no key).
