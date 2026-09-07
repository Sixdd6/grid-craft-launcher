//! Builds the window and wires the `App` global's callbacks to the bridge.

use std::sync::Arc;

use gcl_core::Launcher;
use gcl_core::events::Event;
use slint::ComponentHandle;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::bridge::Bridge;
use crate::events::start_forwarder;
use crate::state::RunState;
use crate::{App, AppWindow, Screen};

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
    crate::screens::instances::wire(&window, &bridge, &run);
    crate::screens::instance::wire(&window, &bridge, &run);
    crate::screens::browser::wire(&window, &bridge);
    crate::screens::accounts::wire(&window, &bridge);
    crate::screens::settings::wire(&window, &bridge);

    start_forwarder(rx, window.as_weak());
    Ok(window)
}
