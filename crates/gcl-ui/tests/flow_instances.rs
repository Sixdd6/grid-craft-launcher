//! The instances screen, driven end to end through the Slint testing backend.
//!
//! One process may hold one Slint backend, so every flow runs inside a single `#[test]`, in
//! order, over one window: an empty list, a vanilla create, a Fabric create, a launch and a
//! stop, a launch that fails and shows the error dialog, then a rename and a delete.

#![cfg(unix)]

mod support;

use std::rc::Rc;
use std::time::Duration;

use gcl_ui::{App, AppWindow, InstanceState, InstancesState, Screen};
use slint::{ComponentHandle, Model};
use support::TestApp;

/// How long a flow waits for a job that only touches the mock host or the disk.
const QUICK: Duration = Duration::from_secs(20);

/// How long a flow waits for the stand-in java to start or to stop.
const LAUNCH: Duration = Duration::from_secs(30);

#[test]
fn the_instances_screen_creates_launches_and_removes_an_instance() {
    support::init_backend();
    let app = Rc::new(TestApp::new());
    let driver = Rc::clone(&app);
    support::run(async move {
        let app = &driver;
        an_empty_root_shows_the_empty_state(app).await;
        creating_a_vanilla_instance_adds_a_row(app).await;
        a_never_launched_instance_has_an_empty_log(app).await;
        creating_a_fabric_instance_installs_the_loader(app).await;
        launching_a_row_runs_the_game_until_stop(app).await;
        a_launch_that_cannot_start_opens_the_error_dialog(app).await;
        the_click_after_escape_still_lands(app).await;
        renaming_and_deleting_an_instance(app).await;
    });
}

/// The names of the rows the list is showing.
fn row_names(window: &AppWindow) -> Vec<String> {
    window
        .global::<InstancesState>()
        .get_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect()
}

/// The row with this name, if the list has one.
fn row(window: &AppWindow, name: &str) -> Option<gcl_ui::InstanceRow> {
    window
        .global::<InstancesState>()
        .get_rows()
        .iter()
        .find(|row| row.name == name)
}

/// The position of the row with this name in the list.
fn row_index(window: &AppWindow, name: &str) -> usize {
    window
        .global::<InstancesState>()
        .get_rows()
        .iter()
        .position(|row| row.name == name)
        .unwrap_or_else(|| panic!("no row named `{name}` in {:?}", row_names(window)))
}

/// (a) A fresh root has no instances, and the screen says so.
async fn an_empty_root_shows_the_empty_state(app: &TestApp) {
    app.wait_until(
        "the first list read to finish",
        |window| !window.global::<InstancesState>().get_loading(),
        QUICK,
    )
    .await;
    assert!(
        row_names(&app.window).is_empty(),
        "a fresh root has no rows"
    );
    assert!(
        app.has("EmptyState::empty_title"),
        "the empty state is showing. Showing: {:?}",
        app.ids()
    );
    assert!(app.has("InstancesScreen::create_button"));
}

/// (b) Create with the loader left at "none", over the mock vanilla version.
async fn creating_a_vanilla_instance_adds_a_row(app: &TestApp) {
    app.click("InstancesScreen::create_button");
    app.wait_for("CreateInstanceDialog::name_field", QUICK)
        .await;
    app.wait_until(
        "the Minecraft version list to load",
        |window| {
            window
                .global::<InstancesState>()
                .get_version_labels()
                .row_count()
                > 0
        },
        QUICK,
    )
    .await;

    app.type_into("CreateInstanceDialog::name_field", "Vanilla");
    support::pump();
    assert_eq!(
        app.el("CreateInstanceDialog::version_combo")
            .accessible_value(),
        Some(support::MC.into()),
        "the newest release is preselected"
    );

    app.click("Dialog::confirm_button");
    app.wait_until(
        "the new row to appear",
        |window| row(window, "Vanilla").is_some(),
        QUICK,
    )
    .await;
    app.wait_gone("CreateInstanceDialog::name_field", QUICK)
        .await;
    assert!(
        app.launcher.root().instances_dir().join("vanilla").is_dir(),
        "the instance folder is on disk"
    );
}

/// (b2) The Logs tab of an instance nobody has launched is empty.
///
/// The game log is the output of a game that ran. A row of sample text here read as a game
/// that had started, on an instance whose folder holds no log file at all.
async fn a_never_launched_instance_has_an_empty_log(app: &TestApp) {
    let index = row_index(&app.window, "Vanilla");
    app.click_nth("InstancesScreen::row_open", index);
    app.wait_until(
        "the detail screen to show the instance",
        |window| {
            window.global::<App>().get_screen() == Screen::Instance
                && window.global::<InstanceState>().get_name() == "Vanilla"
        },
        QUICK,
    )
    .await;
    // Tab 3 is Logs.
    app.el_nth("TabBar::tab_entry", 3)
        .invoke_accessible_default_action();
    support::pump();

    let log: Vec<String> = app
        .window
        .global::<InstanceState>()
        .get_game_log()
        .iter()
        .map(|line| line.text.to_string())
        .collect();
    assert!(
        log.is_empty(),
        "this instance has never been launched, so it has written no log. It shows: {log:?}"
    );
    assert_eq!(
        app.window
            .global::<InstanceState>()
            .get_status_text()
            .to_string(),
        "",
        "and nothing has happened to it yet"
    );

    app.click("InstanceScreen::back_button");
    app.wait_until(
        "the list to come back",
        |window| window.global::<App>().get_screen() == Screen::Instances,
        QUICK,
    )
    .await;
}

/// (c) Create with Fabric: the build list loads, and the loader is installed after the create.
async fn creating_a_fabric_instance_installs_the_loader(app: &TestApp) {
    app.click("InstancesScreen::create_button");
    app.wait_for("CreateInstanceDialog::name_field", QUICK)
        .await;
    app.wait_until(
        "the Minecraft version list to load",
        |window| {
            window
                .global::<InstancesState>()
                .get_version_labels()
                .row_count()
                > 0
        },
        QUICK,
    )
    .await;
    app.type_into("CreateInstanceDialog::name_field", "Fabric");
    app.select_combo("CreateInstanceDialog::loader_combo", 1);
    assert_eq!(
        app.window.global::<InstancesState>().get_loader_index(),
        1,
        "the loader ComboBox picked Fabric"
    );

    app.wait_until(
        "the Fabric build list to load",
        |window| {
            let state = window.global::<InstancesState>();
            !state.get_loading_loader_versions()
                && state.get_loader_version_labels().row_count() > 0
        },
        QUICK,
    )
    .await;
    assert!(
        app.has("CreateInstanceDialog::loader_version_combo"),
        "the build ComboBox is showing. Showing: {:?}",
        app.ids()
    );

    app.click("Dialog::confirm_button");
    app.wait_until(
        "the Fabric row to appear",
        |window| row(window, "Fabric").is_some(),
        QUICK,
    )
    .await;
    app.wait_until(
        "the Fabric loader to be installed",
        |window| row(window, "Fabric").is_some_and(|row| row.installed),
        QUICK,
    )
    .await;
    let instance = app.launcher.instances().get("fabric").expect("read fabric");
    assert_eq!(
        instance.config.loader_version.as_deref(),
        Some(support::FABRIC)
    );
}

/// (d) Launch the vanilla row, then stop it from the detail screen.
async fn launching_a_row_runs_the_game_until_stop(app: &TestApp) {
    let index = row_index(&app.window, "Vanilla");
    app.click_nth("InstancesScreen::row_launch", index);

    app.wait_until(
        "the game to be running",
        |window| window.global::<InstanceState>().get_running(),
        LAUNCH,
    )
    .await;
    assert_eq!(
        app.window.global::<App>().get_screen(),
        Screen::Instance,
        "a launch opens the detail screen"
    );

    app.wait_until(
        "the stand-in java to record its arguments",
        |_| {
            let args = app.java_args();
            args.iter().any(|arg| arg == "--username")
        },
        LAUNCH,
    )
    .await;
    let args = app.java_args();
    let user = args
        .iter()
        .position(|arg| arg == "--username")
        .expect("a --username argument");
    assert_eq!(
        args.get(user + 1).map(String::as_str),
        Some(support::PLAYER),
        "the offline account named the player: {args:?}"
    );
    assert_eq!(app.launcher.running_slugs(), vec!["vanilla".to_string()]);

    app.click("InstanceScreen::stop_button");
    app.wait_until(
        "the game to stop",
        |window| !window.global::<InstanceState>().get_running(),
        LAUNCH,
    )
    .await;
    let status = app
        .window
        .global::<InstanceState>()
        .get_status_text()
        .to_string();
    assert_eq!(
        status, "Stopped",
        "a requested stop is reported as one, not as the 143 the signal produced"
    );
    let warned = app
        .window
        .global::<App>()
        .get_app_log()
        .iter()
        .any(|line| line.text.contains("Minecraft exited with code"));
    assert!(
        !warned,
        "a stop the user asked for raises no warning, so nothing reaches the app log"
    );
    assert!(app.launcher.running_slugs().is_empty());
}

/// (d2) A launch the launcher cannot start opens the shared error dialog, and Dismiss shuts it.
///
/// The stand-in java is made unreadable and unexecutable for the length of this sub-flow, so
/// the launch fails at the one step nothing can recover from. The dialog has to name the GUI
/// log, because that is where the whole error chain was written.
async fn a_launch_that_cannot_start_opens_the_error_dialog(app: &TestApp) {
    use std::os::unix::fs::PermissionsExt;

    let java = app.root().join("fake-java");
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o000))
        .expect("take the stand-in java away");

    app.click("InstanceScreen::launch_button");
    app.wait_until(
        "the error dialog to open",
        |window| window.global::<App>().get_error_open(),
        LAUNCH,
    )
    .await;

    let text = app.window.global::<App>().get_error_text().to_string();
    assert!(
        text.contains("gui.log"),
        "the dialog points at the GUI log. It says:
{text}"
    );
    assert!(
        app.has("AppWindow::error_dialog") && app.has("AppWindow::error_text"),
        "the dialog and its body are in the element tree. Showing: {:?}",
        app.ids()
    );
    assert_eq!(
        app.el("AppWindow::error_text").accessible_value(),
        Some(text.as_str().into()),
        "the body shows the same text the property holds"
    );

    app.click("Dialog::cancel_button");
    app.wait_until(
        "the error dialog to close",
        |window| !window.global::<App>().get_error_open(),
        QUICK,
    )
    .await;
    assert!(
        !app.has("AppWindow::error_text"),
        "a dismissed dialog leaves the element tree"
    );

    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755))
        .expect("give the stand-in java back");
}

/// (d3) A dialog closed with Escape does not eat the next click.
///
/// A dialog that stays mounted while closed leaves its overlay `TouchArea` in the tree, and
/// that area keeps the pointer grab it took when the dialog opened, so the first click after
/// an Escape is swallowed and the rail entry under it never navigates. The step opens the
/// rename prompt, presses Escape, and clicks the Accounts entry exactly once.
async fn the_click_after_escape_still_lands(app: &TestApp) {
    app.click("InstanceScreen::rename_button");
    app.wait_until(
        "the rename prompt to open",
        |window| window.global::<InstanceState>().get_prompt_open(),
        QUICK,
    )
    .await;

    app.press_key(slint::platform::Key::Escape);
    app.wait_until(
        "the rename prompt to close",
        |window| !window.global::<InstanceState>().get_prompt_open(),
        QUICK,
    )
    .await;

    app.click("Rail::rail_accounts");
    app.wait_until(
        "the accounts screen to open after one click",
        |window| window.global::<App>().get_screen() == Screen::Accounts,
        QUICK,
    )
    .await;

    // Back to the detail screen, which is where the next step starts.
    app.click("Rail::rail_instance");
    app.wait_until(
        "the detail screen to come back",
        |window| window.global::<App>().get_screen() == Screen::Instance,
        QUICK,
    )
    .await;
}

/// (e) Rename from the detail screen, then delete from the list.
async fn renaming_and_deleting_an_instance(app: &TestApp) {
    app.click("InstanceScreen::rename_button");
    app.wait_until(
        "the rename prompt to open",
        |window| window.global::<InstanceState>().get_prompt_open(),
        QUICK,
    )
    .await;
    app.type_into("PromptDialog::name_field", "Renamed");
    support::pump();
    app.click("Dialog::confirm_button");
    app.wait_until(
        "the new name to show",
        |window| window.global::<InstanceState>().get_name() == "Renamed",
        QUICK,
    )
    .await;

    app.click("InstanceScreen::back_button");
    app.wait_until(
        "the list to show the new name",
        |window| row(window, "Renamed").is_some(),
        QUICK,
    )
    .await;

    let index = row_index(&app.window, "Renamed");
    app.click_nth("InstancesScreen::row_delete", index);
    app.wait_until(
        "the delete confirmation to open",
        |window| window.global::<InstancesState>().get_confirm_open(),
        QUICK,
    )
    .await;
    app.click("Dialog::confirm_button");
    app.wait_until(
        "the row to go",
        |window| row(window, "Renamed").is_none(),
        QUICK,
    )
    .await;
    assert!(
        !app.launcher.root().instances_dir().join("vanilla").exists(),
        "the instance folder was removed"
    );
}
