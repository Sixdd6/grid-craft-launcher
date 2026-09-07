//! Builds the window and wires the `App` global's callbacks to the bridge.

use std::sync::Arc;

use gcl_core::Launcher;
use gcl_core::events::Event;
use slint::ComponentHandle;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::bridge::Bridge;
use crate::events::start_forwarder;
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
            window.global::<App>().set_screen(screen);
        }
    });

    // Opening an instance reads its summary off the UI thread. The detail screen that uses
    // the summary lands in task 4; for now the shell only switches to it.
    app.on_open_instance(move |slug| {
        let slug = slug.to_string();
        bridge.run(
            "Open instance",
            move |launcher| launcher.instance_summary(&slug),
            |window, summary| {
                let app = window.global::<App>();
                app.set_current_slug(summary.instance.slug.as_str().into());
                app.set_screen(Screen::Instance);
                app.set_status_text(summary.instance.config.name.as_str().into());
            },
        );
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

    start_forwarder(rx, window.as_weak());
    Ok(window)
}
