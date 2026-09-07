//! The typed settings editor, driven end to end through the Slint testing backend.
//!
//! One process may hold one Slint backend, so the whole flow runs inside a single `#[test]`,
//! in order, over one window: the launcher defaults first, then the same keys on one
//! instance's own layer. Every assertion reads the file the launcher wrote.

#![cfg(unix)]

mod support;

use std::rc::Rc;
use std::time::Duration;

use gcl_core::instances::model::Loader;
use gcl_ui::{AppWindow, SettingRowModel, SettingsEditorState};
use slint::{ComponentHandle, Model};
use support::TestApp;

/// How long a flow waits for a job that only touches the disk.
const QUICK: Duration = Duration::from_secs(20);

/// The instance the second half of the flow edits.
const SLUG: &str = "settings";

#[test]
fn the_settings_editor_saves_defaults_and_instance_overrides() {
    support::init_backend();
    let app = Rc::new(TestApp::new());
    let driver = Rc::clone(&app);
    support::run(async move {
        let app = &driver;
        the_settings_screen_shows_the_catalog(app).await;
        the_fov_row_reads_in_degrees(app).await;
        a_slider_saves_a_launcher_default(app).await;
        a_switch_saves_a_launcher_default(app).await;
        a_raw_default_save_keeps_the_editor_search(app).await;
        an_instance_overrides_a_default_and_resets_it(app).await;
    });
}

/// The visible editor lines.
fn lines(window: &AppWindow) -> Vec<SettingRowModel> {
    window
        .global::<SettingsEditorState>()
        .get_rows()
        .iter()
        .collect()
}

/// The editor line for one key, if it is showing.
fn line(window: &AppWindow, key: &str) -> Option<SettingRowModel> {
    lines(window).into_iter().find(|line| line.key == key)
}

/// The keys the editor is showing, for a failure message.
fn keys(window: &AppWindow) -> Vec<String> {
    lines(window)
        .iter()
        .map(|line| line.key.to_string())
        .collect()
}

/// Waits until no read or save is running, so every control is live again.
async fn idle(app: &TestApp) {
    app.wait_until(
        "the editor to finish reading",
        |window| !window.global::<SettingsEditorState>().get_busy(),
        QUICK,
    )
    .await;
}

/// Types `query` into the search box and waits until only its group and hits are left.
async fn search(app: &TestApp, query: &str, key: &str) {
    idle(app).await;
    app.scroll_to("SettingsEditor::settings_search_field");
    app.type_into("SettingsEditor::settings_search_field", query);
    support::pump();
    app.wait_until(
        &format!("the editor to show `{key}` alone"),
        |window| {
            let showing = lines(window);
            // One header plus the one hit.
            showing.len() == 2 && showing[1].key == key
        },
        QUICK,
    )
    .await;
}

/// What `config.toml` holds for one game default, as the launcher has it in memory.
fn saved_default(app: &TestApp, key: &str) -> Option<String> {
    app.launcher.config().game_defaults.get(key).cloned()
}

/// `config.toml`, read off the disk.
///
/// `Launcher::config()` would answer from the copy the process already holds, which proves
/// only that the editor called the launcher. Reading the file proves the save landed.
fn config_file(app: &TestApp) -> String {
    let path = app.launcher.root().path().join("config.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// What `instance.toml` holds for one settings override.
fn saved_override(app: &TestApp, key: &str) -> Option<String> {
    app.launcher
        .instances()
        .get(SLUG)
        .expect("read the instance")
        .config
        .settings_overrides
        .get(key)
        .cloned()
}

/// (a) The settings screen opens the editor over the whole catalog, grouped.
async fn the_settings_screen_shows_the_catalog(app: &TestApp) {
    app.click("Rail::rail_settings");
    app.wait_until(
        "the editor to fill",
        |window| !lines(window).is_empty(),
        QUICK,
    )
    .await;

    let showing = lines(&app.window);
    let headers: Vec<String> = showing
        .iter()
        .filter(|line| line.control == "group")
        .map(|line| line.label.to_string())
        .collect();
    assert_eq!(
        headers,
        vec!["Video", "Controls", "Sound", "Chat", "Other"],
        "every catalog group has a header, and a fresh root has no unknown key"
    );
    let render_distance = line(&app.window, "renderDistance").expect("a renderDistance row");
    assert_eq!(render_distance.control, "slider");
    assert_eq!(render_distance.source, "default");
    assert!(
        !render_distance.resettable,
        "nothing is saved yet, so nothing offers Reset"
    );
    assert_eq!(
        line(&app.window, "graphicsMode").map(|row| row.control.to_string()),
        Some("choice".to_string())
    );

    // `fov` is stored as a float in [-1, 1] and set in degrees. The row shows the degrees.
    let fov = line(&app.window, "fov").expect("a fov row");
    assert_eq!(fov.control, "slider");
    assert_eq!(fov.value, "0.0", "what options.txt holds");
    assert_eq!(fov.number, 70.0, "and what the slider is on");
    assert_eq!((fov.minimum, fov.maximum), (30.0, 110.0));
}

/// (a2) The fov row's value label is the degrees, with the degree sign after them.
async fn the_fov_row_reads_in_degrees(app: &TestApp) {
    // The label, not the key: "fov" also matches `fovEffectScale`.
    search(app, "Field of View", "fov").await;
    app.scroll_to("SettingRow::setting_value");
    assert_eq!(
        app.el("SettingRow::setting_value").accessible_label(),
        Some("70\u{b0}".into()),
        "the label under the slider is the number a player sets, not the stored float"
    );
}

/// (b) Increment the render-distance slider: the row and `config.toml` both move.
async fn a_slider_saves_a_launcher_default(app: &TestApp) {
    search(app, "renderDistance", "renderDistance").await;
    let before = line(&app.window, "renderDistance").expect("the row").number;

    idle(app).await;
    app.scroll_to("SettingRow::setting_slider");
    // The press is a click on the track, not on the thumb: renderDistance starts at 12 of
    // 2..32, so the thumb sits near a third of the way across and 0.9 is well clear of it.
    // A press that lands on the thumb only picks it up, and the value never moves.
    app.drag_slider("SettingRow::setting_slider", 0.9);
    app.wait_until(
        "the new render distance to reach config.toml",
        |_| saved_default(app, "renderDistance").is_some(),
        QUICK,
    )
    .await;
    let saved = saved_default(app, "renderDistance").expect("a saved render distance");
    let number: f32 = saved.parse().expect("a number");
    assert!(
        number > before,
        "the drag moved the slider up from {before}, and {saved} is what was saved"
    );

    app.wait_until(
        "the row to show the saved value",
        |window| {
            line(window, "renderDistance")
                .is_some_and(|row| row.value == saved.as_str() && row.source == "preseed")
        },
        QUICK,
    )
    .await;
    let row = line(&app.window, "renderDistance").expect("the row");
    assert!(row.resettable, "a value this layer holds offers Reset");
    assert!(!row.inherited);

    let on_disk = config_file(app);
    let expected = format!("renderDistance = \"{saved}\"");
    assert!(
        on_disk.contains(&expected),
        "config.toml on disk holds `{expected}`. It holds:\n{on_disk}"
    );
}

/// (c) Flip the fullscreen switch: `true` is saved.
async fn a_switch_saves_a_launcher_default(app: &TestApp) {
    search(app, "fullscreen", "fullscreen").await;
    let row = line(&app.window, "fullscreen").expect("the fullscreen row");
    assert_eq!(row.control, "toggle");
    assert!(!row.checked, "Minecraft ships windowed");

    idle(app).await;
    app.scroll_to("SettingRow::setting_switch");
    app.click("SettingRow::setting_switch");
    app.wait_until(
        "fullscreen to reach config.toml",
        |_| saved_default(app, "fullscreen").is_some(),
        QUICK,
    )
    .await;
    assert_eq!(saved_default(app, "fullscreen"), Some("true".to_string()));
    app.wait_until(
        "the switch row to come back checked",
        |window| line(window, "fullscreen").is_some_and(|row| row.checked),
        QUICK,
    )
    .await;
}

/// (c2) A save from the settings screen itself refreshes the editor without retargeting it.
///
/// The screen's Add row writes a raw `options.txt` default, which reloads the whole screen.
/// That reload re-reads the editor's rows, but it must not reopen the editor: reopening
/// empties the search box under the user.
async fn a_raw_default_save_keeps_the_editor_search(app: &TestApp) {
    search(app, "gamma", "gamma").await;

    idle(app).await;
    app.scroll_to("SettingsScreen::default_key_field");
    app.type_into("SettingsScreen::default_key_field", "guiScale");
    app.type_into("SettingsScreen::default_value_field", "2");
    app.click("SettingsScreen::default_add_button");
    app.wait_until(
        "the raw default to reach config.toml",
        |_| saved_default(app, "guiScale") == Some("2".to_string()),
        QUICK,
    )
    .await;
    // The screen's own busy flag is not the editor's: clearing it is what marks the point
    // where the save's `done` ran and asked the editor to refresh. Only then is the editor's
    // flag worth waiting on.
    app.wait_until(
        "the settings screen to finish saving",
        |window| !window.global::<gcl_ui::SettingsState>().get_busy(),
        QUICK,
    )
    .await;
    idle(app).await;

    let state = app.window.global::<SettingsEditorState>();
    assert_eq!(
        state.get_search().to_string(),
        "gamma",
        "the save reloaded the rows, it did not reopen the editor"
    );
    assert_eq!(
        state.get_layer_name().to_string(),
        "default",
        "and the editor is still on the launcher-defaults layer"
    );
    assert_eq!(
        keys(&app.window),
        vec!["Video".to_string(), "gamma".to_string()],
        "so the filtered list is untouched"
    );
}

/// (d) The same key on an instance: the preseed is inherited, an override wins, Reset drops
/// it again.
async fn an_instance_overrides_a_default_and_resets_it(app: &TestApp) {
    // Setup only: the create dialog has its own flow test, so the instance is made directly.
    let defaults = app.launcher.config().game_defaults.clone();
    app.launcher
        .instances()
        .create("Settings", support::MC, Loader::None, None, &defaults)
        .expect("create the instance");

    app.click("Rail::rail_instances");
    support::pump();
    app.click("InstancesScreen::refresh_button");
    app.wait_until(
        "the instance row to show",
        |window| {
            window
                .global::<gcl_ui::InstancesState>()
                .get_rows()
                .iter()
                .any(|row| row.slug == SLUG)
        },
        QUICK,
    )
    .await;
    app.click("InstancesScreen::row_open");
    app.wait_until(
        "the detail screen to open",
        |window| window.global::<gcl_ui::InstanceState>().get_name() == "Settings",
        QUICK,
    )
    .await;
    // Tab 1 is Settings.
    app.el_nth("TabBar::tab_entry", 1)
        .invoke_accessible_default_action();
    support::pump();

    app.wait_until(
        "the instance editor to fill",
        |window| !lines(window).is_empty(),
        QUICK,
    )
    .await;
    search(app, "renderDistance", "renderDistance").await;
    let row = line(&app.window, "renderDistance").expect("the row");
    assert_eq!(
        row.source,
        "file",
        "creating the instance wrote the preseed into its options.txt. Showing: {:?}",
        keys(&app.window)
    );
    assert!(!row.resettable, "an inherited row offers no Reset");
    assert_eq!(
        Some(row.value.to_string()),
        saved_default(app, "renderDistance"),
        "and what it wrote is the launcher default"
    );

    idle(app).await;
    app.scroll_to("SettingRow::setting_slider");
    // The instance inherited the value the first drag saved, so the thumb is near 0.9 now.
    // 0.1 is the far end of the track, clear of it.
    app.drag_slider("SettingRow::setting_slider", 0.1);
    app.wait_until(
        "the override to reach instance.toml",
        |_| saved_override(app, "renderDistance").is_some(),
        QUICK,
    )
    .await;
    let overridden = saved_override(app, "renderDistance").expect("the override");
    assert_ne!(
        Some(overridden.clone()),
        saved_default(app, "renderDistance"),
        "the instance now holds a value of its own"
    );
    app.wait_until(
        "the row to say it is an override",
        |window| line(window, "renderDistance").is_some_and(|row| row.source == "override"),
        QUICK,
    )
    .await;
    // The save reloaded the instance, which reloads this screen. The editor has to stay on
    // the instance layer through that: a reopen here would point it back at the launcher
    // defaults and drop the search box with it.
    idle(app).await;
    let state = app.window.global::<SettingsEditorState>();
    assert_eq!(
        state.get_layer_name().to_string(),
        "override",
        "the editor is still editing this instance's layer"
    );
    assert_eq!(
        state.get_search().to_string(),
        "renderDistance",
        "and the search box survived the reload"
    );

    idle(app).await;
    app.scroll_to("SettingRow::setting_reset_button");
    app.click("SettingRow::setting_reset_button");
    app.wait_until(
        "the override to go from instance.toml",
        |_| saved_override(app, "renderDistance").is_none(),
        QUICK,
    )
    .await;
    app.wait_until(
        "the row to fall back to what options.txt holds",
        |window| line(window, "renderDistance").is_some_and(|row| row.source == "file"),
        QUICK,
    )
    .await;
    assert_eq!(
        saved_default(app, "renderDistance"),
        line(&app.window, "renderDistance").map(|row| row.value.to_string()),
        "the row is back on the value the instance was preseeded with"
    );
}
