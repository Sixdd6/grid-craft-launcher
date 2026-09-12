//! The instances screen, driven end to end through the Slint testing backend.
//!
//! One process may hold one Slint backend, so every flow runs inside a single `#[test]`, in
//! order, over one window: an empty list, a vanilla create, a Fabric create, a launch and a
//! stop, the JVM tab's garbage collector picker, a launch that fails and shows the error
//! dialog, a rename and a delete, then a second instance whose Java offers a shorter list.

#![cfg(unix)]

mod support;

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gcl_core::content::AddRequest;
use gcl_core::sources::SourceId;
use gcl_ui::{App, AppWindow, InstanceState, InstancesState, Screen};
use slint::{ComponentHandle, Model};
use support::{Mocks, TestApp};

/// How long a flow waits for a job that only touches the mock host or the disk.
const QUICK: Duration = Duration::from_secs(20);

/// How long a flow waits for the stand-in java to start or to stop.
const LAUNCH: Duration = Duration::from_secs(30);

/// An extra JVM argument the collector flow saves, to prove the preset's flags come first.
const EXTRA_ARG: &str = "-Dgcl.flow=1";

#[test]
fn the_instances_screen_creates_launches_and_removes_an_instance() {
    support::init_backend();
    // `Mocks::modrinth()` over `plain()`: the content search step installs two rows into
    // Fabric through the mock Modrinth host, on top of the offline account `plain()` gives.
    let app = Rc::new(TestApp::with(Mocks::modrinth()));
    let driver = Rc::clone(&app);
    support::run(async move {
        let app = &driver;
        an_empty_root_shows_the_empty_state(app).await;
        creating_a_vanilla_instance_adds_a_row(app).await;
        a_never_launched_instance_has_an_empty_log(app).await;
        creating_a_fabric_instance_installs_the_loader(app).await;
        typing_into_the_content_search_narrows_the_list(app).await;
        launching_a_row_runs_the_game_until_stop(app).await;
        the_jvm_tab_picks_a_garbage_collector(app).await;
        a_launch_that_cannot_start_opens_the_error_dialog(app).await;
        the_click_after_escape_still_lands(app).await;
        renaming_and_deleting_an_instance(app).await;
        another_java_offers_another_collector_list(app).await;
        a_saved_zgc_reads_as_the_generational_row_on_java_25(app).await;
        a_refused_pick_reads_the_collector_block_again(app).await;
        a_probe_that_fails_says_so_without_a_dialog(app).await;
        the_probe_only_ever_ran_the_stand_in_javas(app).await;
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

/// The text one label element is showing, whichever accessible field carries it.
fn text_of(app: &TestApp, id: &str) -> String {
    let element = app.el(id);
    element
        .accessible_label()
        .or_else(|| element.accessible_value())
        .map(|text| text.to_string())
        .unwrap_or_default()
}

/// The labels the garbage collector combo is offering, in menu order.
fn gc_labels(window: &AppWindow) -> Vec<String> {
    window
        .global::<InstanceState>()
        .get_gc_labels()
        .iter()
        .map(|label| label.to_string())
        .collect()
}

/// The position of one collector label in the combo.
fn gc_index(window: &AppWindow, label: &str) -> usize {
    gc_labels(window)
        .iter()
        .position(|row| row == label)
        .unwrap_or_else(|| panic!("no `{label}` entry in {:?}", gc_labels(window)))
}

/// The token behind one collector label, as the combo's own rows carry it.
fn gc_token(window: &AppWindow, label: &str) -> String {
    window
        .global::<InstanceState>()
        .get_gc_options()
        .iter()
        .find(|row| row.label == label)
        .map(|row| row.token.to_string())
        .unwrap_or_else(|| panic!("no `{label}` entry in {:?}", gc_labels(window)))
}

/// Picks one collector, the way the combo's `selected` handler does: one pick, one save.
///
/// Never [`TestApp::select_combo`] on this combo. It walks the popup a row at a time and the
/// combo saves every row it passes, each on its own background thread, so the last write to
/// land is not the last row the walk asked for and the reload it triggers moves the popup's
/// cursor under the walk. That race is what made this flow time out. One keypress at a time
/// is still covered, by [`TestApp::combo_step_down`].
fn pick_gc(app: &TestApp, label: &str) {
    let index = gc_index(&app.window, label);
    let token = gc_token(&app.window, label);
    let state = app.window.global::<InstanceState>();
    state.set_gc_selected_index(index as i32);
    state.invoke_gc_pick(token.into());
    support::pump();
}

/// Yields until this instance's `instance.toml` carries `line`, naming what it holds if not.
async fn wait_for_toml(app: &TestApp, slug: &str, line: &str) {
    app.wait_until(
        &format!("`{line}` to reach {slug}'s instance.toml"),
        |_| instance_toml(app, slug).contains(line),
        QUICK,
    )
    .await;
    assert!(instance_toml(app, slug).contains(line));
}

/// What one instance's `instance.toml` holds right now.
fn instance_toml(app: &TestApp, slug: &str) -> String {
    std::fs::read_to_string(
        app.launcher
            .root()
            .instances_dir()
            .join(slug)
            .join("instance.toml"),
    )
    .unwrap_or_default()
}

/// Opens the JVM tab of the detail screen that is already showing and waits for the probe.
///
/// The probe runs `<root>/fake-java -XX:+PrintFlagsFinal -version`, which the stand-in
/// answers with the recorded dump beside it, so the status line names that dump's version.
async fn open_jvm_tab(app: &TestApp, version: &str) {
    // Tab 2 is JVM.
    app.el_nth("TabBar::tab_entry", 2)
        .invoke_accessible_default_action();
    support::pump();
    app.wait_until(
        &format!("the collector probe to report {version}"),
        |_| text_of(app, "InstanceScreen::gc_status_text").contains(version),
        QUICK,
    )
    .await;
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

/// (c2) Typing into the Content tab's search box narrows the row list to what matches, and
/// clearing it shows everything installed again.
async fn typing_into_the_content_search_narrows_the_list(app: &TestApp) {
    app.launcher
        .add_content(
            "fabric",
            AddRequest {
                source: SourceId::Modrinth,
                project: support::MOD_PROJECT.to_string(),
                version: None,
                kind: None,
                world: None,
            },
        )
        .expect("install sodium into fabric");
    app.launcher
        .add_content(
            "fabric",
            AddRequest {
                source: SourceId::Modrinth,
                project: support::CURRENT_PROJECT.to_string(),
                version: Some(support::CURRENT_VERSION_ID.to_string()),
                kind: None,
                world: None,
            },
        )
        .expect("install reese's sodium options into fabric");

    let index = row_index(&app.window, "Fabric");
    app.click_nth("InstancesScreen::row_open", index);
    app.wait_until(
        "the detail screen to show Fabric",
        |window| {
            window.global::<App>().get_screen() == Screen::Instance
                && window.global::<InstanceState>().get_name() == "Fabric"
        },
        QUICK,
    )
    .await;
    app.wait_until(
        "both installed rows to load",
        |window| window.global::<InstanceState>().get_content().row_count() == 2,
        QUICK,
    )
    .await;

    app.type_into("SearchBox::search_field", "Reese");
    support::pump();
    app.wait_until(
        "the filter to narrow the list to the one match",
        |window| window.global::<InstanceState>().get_content().row_count() == 1,
        QUICK,
    )
    .await;

    app.type_into("SearchBox::search_field", "");
    support::pump();
    app.wait_until(
        "clearing the filter to show both rows again",
        |window| window.global::<InstanceState>().get_content().row_count() == 2,
        QUICK,
    )
    .await;

    app.click("InstanceScreen::back_button");
    app.wait_until(
        "the list to come back",
        |window| window.global::<App>().get_screen() == Screen::Instances,
        QUICK,
    )
    .await;
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

/// (d1b) The JVM tab offers the collectors the instance's Java carries, and a pick is launched.
///
/// The stand-in java answers the probe with a recorded Java 21 dump, which lists all six
/// collector flags, so every preset is on offer. Picking one saves it on its own, a heap save
/// afterwards must keep it, and the launch that follows must pass its flags before the
/// instance's own extra arguments.
///
/// One pick is made from the keyboard, one row down with the popup open, which is the whole
/// of the combo-to-`gc_pick` wiring and exactly one save. The rest go through [`pick_gc`].
async fn the_jvm_tab_picks_a_garbage_collector(app: &TestApp) {
    open_jvm_tab(app, "Java 21").await;
    assert_eq!(
        gc_labels(&app.window),
        [
            "Launcher default",
            "Serial",
            "Parallel",
            "G1",
            "ZGC",
            "ZGC (generational)",
            "Shenandoah",
        ],
        "the Java 21 dump lists every collector flag, so every preset is offered"
    );

    // A saved preset is what the combo is on, and nothing is saved yet, so it sits on the
    // first row. One Down inside the popup is one `selected`, and so one save.
    let state = app.window.global::<InstanceState>();
    assert_eq!(
        state.get_gc_selected_index(),
        0,
        "nothing is saved yet, so the combo is on `Launcher default`"
    );
    let stepped = gc_labels(&app.window)[1].clone();
    let stepped_token = gc_token(&app.window, &stepped);
    app.combo_step_down("InstanceScreen::gc_combo");
    wait_for_toml(app, "vanilla", &format!(r#"gc = "{stepped_token}""#)).await;
    assert_eq!(
        app.el("InstanceScreen::gc_combo").accessible_value(),
        Some(stepped.as_str().into()),
        "the keyboard pick shows on the combo itself, not only in the state behind it"
    );

    pick_gc(app, "ZGC (generational)");
    wait_for_toml(app, "vanilla", r#"gc = "zgc_generational""#).await;
    assert_eq!(
        app.el("InstanceScreen::gc_combo").accessible_value(),
        Some("ZGC (generational)".into()),
        "the combo itself shows what was picked, not only the state behind it"
    );

    // A heap save writes the other JVM fields. It must not wipe the preset beside them.
    app.type_into("InstanceScreen::memory_max_spin", "3072");
    app.type_into("InstanceScreen::jvm_args_field", EXTRA_ARG);
    support::pump();
    app.click("InstanceScreen::jvm_save_button");
    app.wait_until(
        "the heap change to reach instance.toml",
        |_| instance_toml(app, "vanilla").contains("max_mib = 3072"),
        QUICK,
    )
    .await;
    let toml = instance_toml(app, "vanilla");
    assert!(
        toml.contains(r#"gc = "zgc_generational""#),
        "a heap save keeps the collector preset. instance.toml holds:
{toml}"
    );

    app.click("InstanceScreen::launch_button");
    app.wait_until(
        "the game to be running",
        |window| window.global::<InstanceState>().get_running(),
        LAUNCH,
    )
    .await;
    app.wait_until(
        "the stand-in java to record the launch arguments",
        |_| app.java_args().iter().any(|arg| arg == EXTRA_ARG),
        LAUNCH,
    )
    .await;

    let args = app.java_args();
    let at = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("no `{flag}` in the launch arguments: {args:?}"))
    };
    let extra = at(EXTRA_ARG);
    assert!(
        at("-XX:+UseZGC") < extra,
        "the collector flags come before the instance's own arguments: {args:?}"
    );
    assert!(
        at("-XX:+ZGenerational") < extra,
        "Java 21 needs the generational switch as well: {args:?}"
    );

    app.click("InstanceScreen::stop_button");
    app.wait_until(
        "the game to stop",
        |window| !window.global::<InstanceState>().get_running(),
        LAUNCH,
    )
    .await;
    // Back to the Content tab, where the next step expects the screen.
    app.el_nth("TabBar::tab_entry", 0)
        .invoke_accessible_default_action();
    support::pump();
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

/// (f) A Java without a collector shortens the list, and a saved preset it lacks is named.
///
/// Two stand-in javas, both answering the probe from a recorded dump beside them: a real
/// Java 17 one, which has no `ZGenerational` flag and so no generational ZGC, and a Java 21
/// one with its `UseShenandoahGC` line taken out.
async fn another_java_offers_another_collector_list(app: &TestApp) {
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
    app.type_into("CreateInstanceDialog::name_field", "Jvm");
    support::pump();
    app.click("Dialog::confirm_button");
    app.wait_until(
        "the new row to appear",
        |window| row(window, "Jvm").is_some(),
        QUICK,
    )
    .await;

    let index = row_index(&app.window, "Jvm");
    app.click_nth("InstancesScreen::row_open", index);
    app.wait_until(
        "the detail screen to show the instance",
        |window| {
            window.global::<App>().get_screen() == Screen::Instance
                && window.global::<InstanceState>().get_name() == "Jvm"
        },
        QUICK,
    )
    .await;

    let java17 = support::write_java_stand_in(app.root(), "fake-java-17", support::PRINTFLAGS_17);
    point_java_at(app, &java17, "Java 17").await;
    let labels = gc_labels(&app.window);
    assert!(
        !labels.iter().any(|label| label == "ZGC (generational)"),
        "Java 17 has no generational ZGC flag, so it must not be offered: {labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "ZGC"),
        "it does carry plain ZGC: {labels:?}"
    );

    // Save Shenandoah while the Java has it, then take that Java away.
    let java21 = app.root().join("fake-java");
    point_java_at(app, &java21, "Java 21").await;
    pick_gc(app, "Shenandoah");
    wait_for_toml(app, "jvm", r#"gc = "shenandoah""#).await;

    let dump: String = support::PRINTFLAGS_21
        .lines()
        .filter(|line| !line.contains("UseShenandoahGC"))
        .collect::<Vec<_>>()
        .join("\n");
    let plain = support::write_java_stand_in(app.root(), "fake-java-no-shenandoah", &dump);
    point_java_at(app, &plain, "Java 21").await;
    app.wait_for("InstanceScreen::gc_unavailable_text", QUICK)
        .await;
    let warning = text_of(app, "InstanceScreen::gc_unavailable_text");
    assert!(
        warning.contains("Shenandoah"),
        "the saved preset this Java lacks is named. The line reads: {warning}"
    );
    assert!(
        !gc_labels(&app.window)
            .iter()
            .any(|label| label == "Shenandoah"),
        "and it is gone from the list"
    );
}

/// Points the instance on screen at `java` through the JVM tab, and waits for the reprobe.
async fn point_java_at(app: &TestApp, java: &Path, version: &str) {
    // Tab 2 is JVM. The tab is already showing after the first call, and clicking the entry
    // it is on changes nothing, so this is safe to repeat.
    app.el_nth("TabBar::tab_entry", 2)
        .invoke_accessible_default_action();
    support::pump();
    app.type_into("InstanceScreen::java_path_field", &java.to_string_lossy());
    support::pump();
    app.click("InstanceScreen::jvm_save_button");
    app.wait_until(
        &format!("the reprobe to report {version} from {}", java.display()),
        |_| {
            let status = text_of(app, "InstanceScreen::gc_status_text");
            status.contains(version)
                && instance_toml(app, "jvm").contains(&java.display().to_string())
        },
        QUICK,
    )
    .await;
}

/// (g) A plain ZGC saved on Java 21 is the generational row once the Java is 25.
///
/// Java 25 dropped `ZGenerational` because ZGC is generational there, so the picker offers one
/// ZGC row, named "ZGC (generational)". The saved token is still `zgc`, and the combo has to
/// land on that row rather than falling back to the first one and calling it unavailable.
async fn a_saved_zgc_reads_as_the_generational_row_on_java_25(app: &TestApp) {
    let java21 = app.root().join("fake-java");
    point_java_at(app, &java21, "Java 21").await;
    pick_gc(app, "ZGC");
    wait_for_toml(app, "jvm", r#"gc = "zgc""#).await;

    let java25 = support::write_java_stand_in(app.root(), "fake-java-25", support::PRINTFLAGS_25);
    point_java_at(app, &java25, "Java 25").await;
    assert_eq!(
        app.el("InstanceScreen::gc_combo").accessible_value(),
        Some("ZGC (generational)".into()),
        "the saved plain ZGC selects the one ZGC row Java 25 offers. The list is {:?}",
        gc_labels(&app.window)
    );
    assert!(
        !app.has("InstanceScreen::gc_unavailable_text"),
        "and nothing calls it unavailable. Showing: {:?}",
        app.ids()
    );
}

/// (h) A pick the Java refuses reads the collector block again, so nothing stale is left.
///
/// The stand-in java's dump loses its `UseSerialGC` line behind the screen's back, so the
/// block on screen still offers a Serial row the JVM no longer runs. Picking it is refused,
/// and the refused row must not be left selected: the block is read again from the Java that
/// refused it, which drops Serial from the list and puts the combo back on what is saved.
///
/// The pick goes through `InstanceState.gc_pick`, the way every pick in this flow does: see
/// [`pick_gc`]. Walking the popup would fire a pick for every row it passed, and a successful
/// one on the way would reload the block before the refused one ever ran.
async fn a_refused_pick_reads_the_collector_block_again(app: &TestApp) {
    let dump: String = support::PRINTFLAGS_25
        .lines()
        .filter(|line| !line.contains("UseSerialGC"))
        .collect::<Vec<_>>()
        .join("\n");
    support::write_java_stand_in(app.root(), "fake-java-25", &dump);

    pick_gc(app, "Serial");
    app.wait_until(
        "the refusal to open the error dialog",
        |window| window.global::<App>().get_error_open(),
        QUICK,
    )
    .await;
    app.click("Dialog::cancel_button");
    app.wait_until(
        "the collector block to be read again from the Java that refused the pick",
        |window| {
            !gc_labels(window).iter().any(|label| label == "Serial")
                && !window.global::<InstanceState>().get_gc_loading()
        },
        QUICK,
    )
    .await;
    assert_eq!(
        app.el("InstanceScreen::gc_combo").accessible_value(),
        Some("ZGC (generational)".into()),
        "the combo is back on the saved preset, not the refused row. The list is {:?}",
        gc_labels(&app.window)
    );
    let toml = instance_toml(app, "jvm");
    assert!(
        toml.contains(r#"gc = "zgc""#),
        "and the refused preset was not saved. instance.toml holds:
{toml}"
    );
}

/// (i) A Java that cannot answer the probe is a status line, not the error dialog.
///
/// The probe is background work nobody asked for: it runs on every load of the JVM tab. A
/// modal over the screen for it would interrupt whatever the user was doing.
async fn a_probe_that_fails_says_so_without_a_dialog(app: &TestApp) {
    let broken = support::write_failing_java(app.root(), "fake-java-broken");
    // Tab 2 is JVM, already showing from the step before.
    app.type_into("InstanceScreen::java_path_field", &broken.to_string_lossy());
    support::pump();
    app.click("InstanceScreen::jvm_save_button");
    app.wait_until(
        "the failed probe to reach the status line",
        |_| {
            app.has("InstanceScreen::gc_unavailable_text")
                && text_of(app, "InstanceScreen::gc_unavailable_text")
                    .contains("Could not check the garbage collector")
        },
        QUICK,
    )
    .await;
    assert!(
        !app.window.global::<App>().get_error_open(),
        "a failed probe opens no dialog. It says: {}",
        app.window.global::<App>().get_error_text()
    );
    assert!(
        !app.has("AppWindow::error_text"),
        "and none is in the element tree. Showing: {:?}",
        app.ids()
    );
}

/// (j) Every collector probe in this flow ran a stand-in java, and ran each one once.
///
/// Two rules in one place, both about the JVM this flow may start. Every path the probe ran
/// is inside the temp root, so no real `java` on this machine — a `PATH` one, a `JAVA_HOME`
/// one, or a downloaded Mojang runtime — ever decided what the picker offered. And each
/// stand-in was run once per version of itself, so `ProbeCache` answered every later load of
/// the JVM tab without spawning anything: a picker that reprobed per load would spend a real
/// process on every one of this flow's dozen instance loads.
async fn the_probe_only_ever_ran_the_stand_in_javas(app: &TestApp) {
    let runs = app.java_probes();
    assert!(!runs.is_empty(), "the collector probe ran at least once");
    let root = app.root().to_string_lossy().to_string();
    for run in &runs {
        assert!(
            run.starts_with(&root),
            "the probe ran `{run}`, which is not one of the stand-ins under {root}. It ran: \
             {runs:?}"
        );
    }
    assert_eq!(
        app.java_probe_count("fake-java"),
        1,
        "the Java 21 stand-in every instance load resolves to is probed once, then cached. \
         The probe ran: {runs:?}"
    );
    assert_eq!(
        app.java_probe_count("fake-java-17"),
        1,
        "so is the Java 17 one. The probe ran: {runs:?}"
    );
    assert_eq!(
        app.java_probe_count("fake-java-no-shenandoah"),
        1,
        "and the one with no Shenandoah. The probe ran: {runs:?}"
    );
    assert_eq!(
        app.java_probe_count("fake-java-25"),
        2,
        "the Java 25 one is written twice — the refused-pick flow rewrites its dump — and a \
         cache entry is keyed by modification time, so it is probed once per writing. The \
         probe ran: {runs:?}"
    );
}
