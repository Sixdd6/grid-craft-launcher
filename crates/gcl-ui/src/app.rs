//! Builds the window and wires the `App` global's callbacks to the bridge.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gcl_core::Launcher;
use gcl_core::events::Event;
use slint::{ComponentHandle, Timer, TimerMode};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::bridge::Bridge;
use crate::events::start_forwarder;
use crate::keys::{jump, key_to_screen, move_selection};
use crate::state::RunState;
use crate::{App, AppWindow, Screen, Shell, events, toasts};

/// How often finished tasks and spent toasts are aged out.
const TICK: Duration = Duration::from_secs(1);

/// Creates the window, wires the shell callbacks, and starts the event forwarder.
///
/// Returns a `Result` because the generated `AppWindow::new` is fallible: it opens the
/// platform window.
pub fn build(
    launcher: Arc<Launcher>,
    rx: UnboundedReceiver<Event>,
) -> Result<AppWindow, slint::PlatformError> {
    let window = AppWindow::new()?;
    let bridge = Bridge::new(launcher, window.as_weak());
    let app = window.global::<App>();

    app.set_status_text(format!("GRID Craft Launcher {}", gcl_core::VERSION).into());

    let weak = window.as_weak();
    app.on_navigate(move |screen| {
        if let Some(window) = weak.upgrade() {
            // The slug is left alone: the rail's Instance entry goes back to whatever was
            // last open, and the browser says which target it means with
            // `BrowserState.fixed_target` instead.
            window.global::<App>().set_screen(screen);
        }
    });

    // Opening an instance only moves the shell: the detail screen loads itself when
    // `App.current_slug` changes, which `app.slint` turns into `InstanceState.load`.
    let weak = window.as_weak();
    app.on_open_instance(move |slug| {
        if let Some(window) = weak.upgrade() {
            let app = window.global::<App>();
            app.set_current_slug(slug.clone());
            app.set_screen(Screen::Instance);
        }
    });

    // Rail shortcuts. `app.slint` only sends keys nothing else took and no dialog wanted.
    let weak = window.as_weak();
    app.on_key_pressed(move |key| {
        let Some(screen) = key_to_screen(key.as_str()) else {
            return false;
        };
        let Some(window) = weak.upgrade() else {
            return false;
        };
        let app = window.global::<App>();
        // The browser reached from the keyboard names no instance, the same as the rail's own
        // entry, so it offers every instance as a target.
        if screen == Screen::Browser {
            window
                .global::<crate::BrowserState>()
                .set_fixed_target(false);
        }
        app.set_screen(screen);
        true
    });

    let shell = window.global::<Shell>();

    // The shell service a screen may reach. It lives on `Shell` rather than `App` because
    // `App` is declared in `app.slint`, which imports the screens and so cannot be imported
    // back by one.
    shell.on_move_selection(move_selection);
    shell.on_jump(|current, key, page, len| jump(&key, current, page, len).unwrap_or(-1));

    let weak = window.as_weak();
    app.on_dismiss_error(move || {
        if let Some(window) = weak.upgrade() {
            let app = window.global::<App>();
            app.set_error_open(false);
            app.set_error_title("".into());
            app.set_error_text("".into());
        }
    });

    let run = RunState::new();
    // The settings editor is wired first: both the settings screen and the instance detail
    // screen open the same one, so they need its handle.
    let editor = crate::screens::settings_editor::wire(&window, &bridge);
    crate::screens::instances::wire(&window, &bridge, &run);
    crate::screens::instance::wire(&window, &bridge, &run, &editor);
    crate::screens::browser::wire(&window, &bridge);
    crate::screens::project::wire(&window, &bridge);
    crate::screens::accounts::wire(&window, &bridge);
    crate::screens::settings::wire(&window, &bridge, &editor);

    start_forwarder(rx, window.as_weak());

    // One repeated timer ages both lists out: a finished task five seconds after its own end,
    // a toast six seconds after it was pushed. `Timer` stops when it is dropped, so it is
    // leaked on purpose: it has to outlive `build` and lives as long as the process does.
    let ticker: &'static Timer = Box::leak(Box::new(Timer::default()));
    let weak = window.as_weak();
    ticker.start(TimerMode::Repeated, TICK, move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let now = Instant::now();
        events::tick(&window, now);
        toasts::tick(&window, now);
    });

    Ok(window)
}
