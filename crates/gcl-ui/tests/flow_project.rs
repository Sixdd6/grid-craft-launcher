//! The project details screen, driven end to end through the Slint testing backend.
//!
//! One process may hold one Slint backend, so the whole flow runs inside a single `#[test]`,
//! in order, over one window: search, open a project's details from a search row, read its
//! description, go Back and reopen the same row, install an older version from the versions
//! tab, find it in the instance's content list, check updates and see a title an older writer
//! left out backfilled, reopen the details from the content list's source button, install the
//! newer version over it, go back, and reopen once more to check Escape does the same thing.
//!
//! Modrinth is a wiremock host over the recorded fixtures — the search page, the project, its
//! two versions, the jar each version points at, and the project's icon — so nothing here
//! touches the network.

#![cfg(unix)]

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gcl_core::instances::model::Loader;
use gcl_ui::{App, AppWindow, BrowserState, InstanceState, InstancesState, ProjectState, Screen};
use slint::{ComponentHandle, Model};
use support::{Mocks, TestApp};

/// How long a flow waits for a job that only touches the mock host or the disk.
const QUICK: Duration = Duration::from_secs(30);

/// The instance every sub-flow installs into.
const INSTANCE: &str = "Fabric Test";

/// Its directory name, which is what every launcher call takes.
const SLUG: &str = "fabric-test";

#[test]
fn the_details_screen_installs_a_chosen_version() {
    support::init_backend();
    let app = Rc::new(TestApp::with(Mocks::project()));
    // Setup only: creating an instance is `flow_instances`' subject, not this one's.
    app.launcher
        .instances()
        .create(
            INSTANCE,
            support::MC,
            Loader::Fabric,
            Some(support::FABRIC.to_string()),
            &BTreeMap::new(),
        )
        .expect("create the instance the flow installs into");

    let driver = Rc::clone(&app);
    support::run(async move {
        let app = &driver;
        searching_shows_a_decoded_icon(app).await;
        a_row_title_opens_the_description(app).await;
        back_keeps_the_search_results_and_reopens_the_same_row(app).await;
        the_versions_tab_installs_the_older_version(app).await;
        the_notes_button_shows_the_changelog(app).await;
        a_changelog_the_source_refuses_shows_the_error(app).await;
        the_content_list_shows_the_title_over_the_file_name(app).await;
        checking_updates_backfills_the_title_and_keeps_the_mark(app).await;
        the_source_button_reopens_the_details(app).await;
        installing_the_newer_version_replaces_the_file(app).await;
        back_returns_to_the_instance_screen(app).await;
        escape_does_what_back_does(app).await;
        opening_notes_again_before_a_slow_fetch_lands_keeps_the_new_one(app).await;
        opening_a_project_again_before_a_slow_load_lands_keeps_the_new_one(app).await;
    });
}

/// The names of the instance rows the list is showing.
fn instance_names(window: &AppWindow) -> Vec<String> {
    window
        .global::<InstancesState>()
        .get_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect()
}

/// The titles of the search rows the browser is showing.
fn row_titles(window: &AppWindow) -> Vec<String> {
    window
        .global::<BrowserState>()
        .get_rows()
        .iter()
        .map(|row| row.title.to_string())
        .collect()
}

/// The width in pixels of the first search row's decoded icon, `0` while it has none.
fn first_icon_width(window: &AppWindow) -> u32 {
    window
        .global::<BrowserState>()
        .get_rows()
        .iter()
        .next()
        .map(|row| row.icon.size().width)
        .unwrap_or_default()
}

/// The description blocks the details screen is showing, as `(kind, text)`.
fn blocks(window: &AppWindow) -> Vec<(String, String)> {
    window
        .global::<ProjectState>()
        .get_blocks()
        .iter()
        .map(|block| (block.kind.to_string(), block.text.to_string()))
        .collect()
}

/// The notes modal's blocks, as `(kind, text)`.
fn notes_blocks(window: &AppWindow) -> Vec<(String, String)> {
    window
        .global::<ProjectState>()
        .get_notes_blocks()
        .iter()
        .map(|block| (block.kind.to_string(), block.text.to_string()))
        .collect()
}

/// The first description block of `kind`, `None` when the description has none.
fn block_of_kind(window: &AppWindow, kind: &str) -> Option<gcl_ui::Block> {
    window
        .global::<ProjectState>()
        .get_blocks()
        .iter()
        .find(|block| block.kind == kind)
}

/// The version rows the details screen is showing, as `(number, installed)`.
fn versions(window: &AppWindow) -> Vec<(String, bool)> {
    window
        .global::<ProjectState>()
        .get_versions()
        .iter()
        .map(|row| (row.number.to_string(), row.installed))
        .collect()
}

/// The content rows the instance screen is showing, as `(name, file_name)`.
fn content(window: &AppWindow) -> Vec<(String, String)> {
    window
        .global::<InstanceState>()
        .get_content()
        .iter()
        .map(|row| (row.name.to_string(), row.file_name.to_string()))
        .collect()
}

/// The instance's mods folder.
fn mods_dir(app: &TestApp) -> PathBuf {
    app.launcher
        .root()
        .instance_dir(SLUG)
        .join(".minecraft")
        .join("mods")
}

/// The index of the version row for `number`.
fn version_index(window: &AppWindow, number: &str) -> usize {
    versions(window)
        .iter()
        .position(|(row, _)| row == number)
        .unwrap_or_else(|| panic!("the versions tab lists {number}: {:?}", versions(window)))
}

/// Waits until the browser has finished loading its sources and its instance list.
async fn wait_for_browser(app: &TestApp) {
    const OPENED: [&str; 3] = [
        "Ready to search",
        "CurseForge disabled: set CURSEFORGE_API_KEY",
        "No content source is enabled",
    ];
    app.wait_until(
        "the browser to open",
        |window| {
            window.global::<App>().get_screen() == Screen::Browser
                && OPENED.contains(&window.global::<BrowserState>().get_status().as_str())
        },
        QUICK,
    )
    .await;
}

/// Waits until the details screen has a project and is not loading anything.
async fn wait_for_details(app: &TestApp) {
    app.wait_until(
        "the details screen to load the project",
        |window| {
            window.global::<App>().get_screen() == Screen::Project
                && window.global::<ProjectState>().get_title() == support::MOD_TITLE
                && !window.global::<ProjectState>().get_loading()
        },
        QUICK,
    )
    .await;
}

/// Opens the instance's detail screen from the instance list.
async fn open_the_instance(app: &TestApp) {
    app.click("Rail::rail_instances");
    // The window was built before the setup instance existed, so nothing has loaded it yet.
    app.click("InstancesScreen::refresh_button");
    app.wait_until(
        "the instance list to show the setup instance",
        |window| instance_names(window).iter().any(|name| name == INSTANCE),
        QUICK,
    )
    .await;
    let index = instance_names(&app.window)
        .iter()
        .position(|name| name == INSTANCE)
        .expect("the setup instance is in the list");
    app.click_nth("InstancesScreen::row_open", index);
    app.wait_until(
        "the detail screen to show the instance",
        |window| {
            window.global::<App>().get_screen() == Screen::Instance
                && window.global::<InstanceState>().get_name() == INSTANCE
        },
        QUICK,
    )
    .await;
}

/// (a) A search page fetches and decodes every row's icon.
async fn searching_shows_a_decoded_icon(app: &TestApp) {
    open_the_instance(app).await;
    app.click("InstanceScreen::add_content_button");
    wait_for_browser(app).await;

    app.type_into("SearchBox::search_field", "sodium");
    support::pump();
    app.click("BrowserScreen::search_button");
    app.wait_until(
        "the search results to arrive",
        |window| !row_titles(window).is_empty(),
        QUICK,
    )
    .await;
    assert_eq!(
        row_titles(&app.window).first().map(String::as_str),
        Some(support::MOD_TITLE),
        "the first row is the first hit of the recorded search"
    );

    app.wait_until(
        "the first row's icon to be decoded",
        |window| first_icon_width(window) > 0,
        QUICK,
    )
    .await;
    assert_eq!(
        first_icon_width(&app.window),
        2,
        "the row carries the decoded icon the mock served, not a placeholder"
    );
}

/// (b) A row's title opens the details screen with the project's description.
async fn a_row_title_opens_the_description(app: &TestApp) {
    app.click_nth("BrowserScreen::row_title", 0);
    wait_for_details(app).await;

    let state = app.window.global::<ProjectState>();
    assert_eq!(
        state.get_target_slug().to_string(),
        SLUG,
        "the details screen installs into the instance the browser was targeting"
    );
    assert!(
        !state.get_author().to_string().is_empty(),
        "the header carries the author the search row already knew"
    );
    assert!(
        !state.get_downloads().to_string().is_empty(),
        "and its download count"
    );
    assert_eq!(
        state.get_tab(),
        0,
        "the details screen opens on Description"
    );

    let blocks = blocks(&app.window);
    assert!(
        blocks
            .iter()
            .any(|(kind, text)| kind.starts_with("heading") && text == support::MOD_BODY_HEADING),
        "the description shows the body's heading: {blocks:?}"
    );
    assert!(
        blocks
            .iter()
            .any(|(kind, text)| kind == "bullet" && text == support::MOD_BODY_BULLET),
        "and its bullets: {blocks:?}"
    );
    assert!(
        blocks.iter().all(|(_, text)| !text.contains('<')),
        "and no markup at all: {blocks:?}"
    );
    assert!(
        blocks
            .iter()
            .any(|(kind, text)| kind == "quote" && text == support::MOD_BODY_QUOTE),
        "the body's quote reaches a quote block of its own: {blocks:?}"
    );

    let nested = app
        .window
        .global::<ProjectState>()
        .get_blocks()
        .iter()
        .find(|block| block.text == support::MOD_BODY_NESTED_BULLET)
        .expect("the description carries the nested bullet");
    assert_eq!(
        (nested.kind.to_string(), nested.depth),
        ("bullet".to_string(), 1),
        "a bullet one level in keeps its depth, so the screen can indent it"
    );

    let table = block_of_kind(&app.window, "table").expect("the description carries a table");
    assert_eq!(table.columns, 2, "the pipe table has two columns");
    assert_eq!(
        table.cells.iter().next().map(|cell| cell.to_string()),
        Some(support::MOD_BODY_TABLE_HEADER.to_string()),
        "cell 0 is the header row's first cell, row-major from there"
    );
    assert_eq!(
        table.cells.iter().count(),
        4,
        "one header row and one body row, two cells each"
    );

    // The image is fetched and decoded off the UI thread after the blocks land, so this is
    // the one description assertion that has to wait.
    app.wait_until(
        "the description image to be fetched and decoded",
        |window| block_of_kind(window, "image").is_some_and(|block| block.image_state == "ready"),
        QUICK,
    )
    .await;
    let image = block_of_kind(&app.window, "image").expect("the description carries an image");
    assert_eq!(
        image.text.to_string(),
        support::MOD_BODY_IMAGE_ALT,
        "the block keeps the alt text, which is what a failed load would show instead"
    );
    assert!(
        image.image.size().width > 0 && image.image.size().height > 0,
        "and carries the decoded pixels the mock served, not an empty image"
    );
}

/// (b2) Back returns to the browser with the same page of hits still on screen, and the
/// title still opens the same project.
async fn back_keeps_the_search_results_and_reopens_the_same_row(app: &TestApp) {
    let titles_before = row_titles(&app.window);

    app.click("ProjectScreen::back_button");
    app.wait_until(
        "the browser to come back",
        |window| window.global::<App>().get_screen() == Screen::Browser,
        QUICK,
    )
    .await;
    assert_eq!(
        row_titles(&app.window),
        titles_before,
        "Back must not clear the rows the search already found"
    );

    app.click_nth("BrowserScreen::row_title", 0);
    wait_for_details(app).await;
}

/// (c) The versions tab lists both versions and installs the one it is asked for.
async fn the_versions_tab_installs_the_older_version(app: &TestApp) {
    app.click_nth("TabBar::tab_entry", 1);
    app.wait_until(
        "the versions tab to list both versions",
        |window| window.global::<ProjectState>().get_tab() == 1 && versions(window).len() == 2,
        QUICK,
    )
    .await;
    assert!(
        versions(&app.window)
            .iter()
            .all(|(_, installed)| !installed),
        "nothing is installed yet, so no row is marked"
    );

    let index = version_index(&app.window, support::OLD_MOD_VERSION);
    app.click_nth("ProjectScreen::version_install_button", index);
    app.wait_until(
        "the install to finish",
        |window| {
            window.global::<ProjectState>().get_status()
                == format!("Installed {}", support::OLD_MOD_VERSION)
        },
        QUICK,
    )
    .await;
    assert!(
        mods_dir(app).join(support::OLD_MOD_FILE_NAME).is_file(),
        "the older version's jar is in the instance's mods folder"
    );
}

/// (c2) A version row's Notes button opens the modal on that version's changelog, and Escape
/// closes it again.
async fn the_notes_button_shows_the_changelog(app: &TestApp) {
    let index = version_index(&app.window, support::MOD_VERSION);
    app.click_nth("ProjectScreen::version_notes_button", index);
    app.wait_until(
        "the notes modal to open and load the changelog",
        |window| {
            let state = window.global::<ProjectState>();
            state.get_notes_open() && !state.get_notes_loading()
        },
        QUICK,
    )
    .await;

    assert!(
        app.has("Dialog::cancel_button"),
        "the modal is mounted, with a Close button to press"
    );
    assert!(
        app.has_label(&format!("{} notes", support::MOD_VERSION)),
        "and its title names the version the button belongs to. Showing: {:?}",
        app.ids()
    );

    let blocks = notes_blocks(&app.window);
    assert!(
        blocks
            .iter()
            .any(|(kind, text)| kind.starts_with("heading")
                && text == support::MOD_CHANGELOG_HEADING),
        "the modal shows the changelog's heading: {blocks:?}"
    );
    assert!(
        blocks
            .iter()
            .any(|(kind, text)| kind == "bullet" && text == support::MOD_CHANGELOG_BULLET),
        "and its bullet: {blocks:?}"
    );

    app.press_key(slint::platform::Key::Escape);
    app.wait_until(
        "Escape to close the notes modal",
        |window| !window.global::<ProjectState>().get_notes_open(),
        QUICK,
    )
    .await;
    assert!(
        !app.has("Dialog::cancel_button"),
        "and the modal is unmounted, not merely hidden"
    );
}

/// Which of the closing buttons on screen belongs to the error dialog, which prints
/// "Dismiss" where every other dialog prints "Close".
fn dismiss_index(app: &TestApp) -> usize {
    app.all("Dialog::cancel_button")
        .iter()
        .position(|button| button.accessible_label().as_deref() == Some("Dismiss"))
        .expect("the error dialog is one of the dialogs on screen")
}

/// (c3) A changelog the source refuses reports the failure and leaves both the error and the
/// modal closable.
async fn a_changelog_the_source_refuses_shows_the_error(app: &TestApp) {
    let index = version_index(&app.window, support::OLD_MOD_VERSION);
    app.click_nth("ProjectScreen::version_notes_button", index);
    app.wait_until(
        "the failed changelog to raise the error dialog",
        |window| {
            window.global::<App>().get_error_open()
                && !window.global::<ProjectState>().get_notes_loading()
        },
        QUICK,
    )
    .await;
    assert!(
        notes_blocks(&app.window).is_empty(),
        "a failed fetch leaves the modal with no blocks rather than the last version's"
    );
    assert!(
        app.window
            .global::<ProjectState>()
            .get_notes_status()
            .to_string()
            .starts_with("Could not load notes:"),
        "the modal shows why, in place of the blank \"No notes\" state a fetch that never \
         ran would show: {:?}",
        app.window.global::<ProjectState>().get_notes_status()
    );

    // Two dialogs are up at once, so each closing button is picked by the word it prints.
    // The error dialog must be the one on top: its Dismiss button is only reachable by a
    // pointer if the notes modal's overlay does not cover it.
    app.click_nth("Dialog::cancel_button", dismiss_index(app));
    app.wait_until(
        "the error dialog to close",
        |window| !window.global::<App>().get_error_open(),
        QUICK,
    )
    .await;

    app.click("Dialog::cancel_button");
    app.wait_until(
        "the notes modal to close behind it",
        |window| !window.global::<ProjectState>().get_notes_open(),
        QUICK,
    )
    .await;
}

/// (d) The content list shows the project's title over the file it installed.
async fn the_content_list_shows_the_title_over_the_file_name(app: &TestApp) {
    app.click("Rail::rail_instance");
    app.wait_until(
        "the content tab to list the mod",
        |window| !content(window).is_empty(),
        QUICK,
    )
    .await;

    let rows = content(&app.window);
    assert_eq!(
        rows,
        vec![(
            support::MOD_TITLE.to_string(),
            support::OLD_MOD_FILE_NAME.to_string()
        )],
        "the row names the project, with the installed file under it"
    );
    let mut sorted: Vec<String> = rows.iter().map(|(name, _)| name.to_lowercase()).collect();
    sorted.sort();
    assert_eq!(
        sorted,
        rows.iter()
            .map(|(name, _)| name.to_lowercase())
            .collect::<Vec<_>>(),
        "the content list is in name order, case-insensitively"
    );
}

/// (d2) Check updates finds the newer version and backfills a title an older writer left
/// out, and the reload that follows shows it without losing the mark.
async fn checking_updates_backfills_the_title_and_keeps_the_mark(app: &TestApp) {
    let toml_path = app.launcher.root().instance_dir(SLUG).join("instance.toml");
    let toml = std::fs::read_to_string(&toml_path).expect("read instance.toml");
    let stripped: String = toml
        .lines()
        .filter(|line| !line.trim_start().starts_with("title ="))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&toml_path, stripped)
        .expect("strip the title, as an entry saved before it existed would have none");

    // The screen is already showing this instance from the step before, so nothing here
    // re-reads it from disk until Check updates does.
    app.click("InstanceScreen::check_updates_button");
    app.wait_until(
        "check updates to backfill the title and keep the row marked for an update",
        |window| {
            window
                .global::<InstanceState>()
                .get_content()
                .iter()
                .any(|row| row.name.as_str() == support::MOD_TITLE && row.update_available)
        },
        QUICK,
    )
    .await;
}

/// (e) A content row's source button reopens the details, with the installed row marked.
async fn the_source_button_reopens_the_details(app: &TestApp) {
    app.click_nth("InstanceScreen::row_source_button", 0);
    wait_for_details(app).await;
    assert_eq!(
        app.window.global::<ProjectState>().get_return_to(),
        Screen::Instance,
        "Back returns to the screen this open came from"
    );

    app.click_nth("TabBar::tab_entry", 1);
    app.wait_until(
        "the versions tab to mark the installed version",
        |window| {
            window.global::<ProjectState>().get_tab() == 1
                && versions(window).iter().any(|(_, installed)| *installed)
        },
        QUICK,
    )
    .await;
    let marked: Vec<String> = versions(&app.window)
        .into_iter()
        .filter(|(_, installed)| *installed)
        .map(|(number, _)| number)
        .collect();
    assert_eq!(
        marked,
        vec![support::OLD_MOD_VERSION.to_string()],
        "only the installed version carries the marker"
    );
}

/// (f) Installing another version replaces the file and leaves one entry behind.
async fn installing_the_newer_version_replaces_the_file(app: &TestApp) {
    let index = version_index(&app.window, support::MOD_VERSION);
    app.click_nth("ProjectScreen::version_install_button", index);
    app.wait_until(
        "the install to finish",
        |window| {
            window.global::<ProjectState>().get_status()
                == format!("Installed {}", support::MOD_VERSION)
        },
        QUICK,
    )
    .await;

    assert!(
        mods_dir(app).join(support::MOD_FILE_NAME).is_file(),
        "the newer version's jar is in the mods folder"
    );
    assert!(
        !mods_dir(app).join(support::OLD_MOD_FILE_NAME).exists(),
        "and the older one it replaced is gone"
    );
    let toml =
        std::fs::read_to_string(app.launcher.root().instance_dir(SLUG).join("instance.toml"))
            .expect("read the instance file");
    assert_eq!(
        toml.matches("[[content]]").count(),
        1,
        "the install replaced the entry rather than adding a second one:\n{toml}"
    );
    app.wait_until(
        "the versions tab to move the marker to the newer version",
        |window| {
            versions(window)
                .iter()
                .any(|(number, installed)| number == support::MOD_VERSION && *installed)
        },
        QUICK,
    )
    .await;
}

/// (g) Back returns to the screen the details were opened from.
async fn back_returns_to_the_instance_screen(app: &TestApp) {
    app.click("ProjectScreen::back_button");
    app.wait_until(
        "the instance screen to come back",
        |window| window.global::<App>().get_screen() == Screen::Instance,
        QUICK,
    )
    .await;
    assert_eq!(
        app.window.global::<App>().get_current_slug().to_string(),
        SLUG,
        "and it is still the instance the details were opened from"
    );
    app.wait_until(
        "the content list to show the version the details screen installed",
        |window| {
            content(window)
                == vec![(
                    support::MOD_TITLE.to_string(),
                    support::MOD_FILE_NAME.to_string(),
                )]
        },
        QUICK,
    )
    .await;
}

/// (i) Opening a version's notes while an earlier, slower fetch for a different version is
/// still in flight must not let the slow one win once it lands: `open_notes`'s done closure
/// checks its own generation before painting anything, per the `slint-ui` skill's rule that a
/// project (or a modal) the user has since navigated away from never gets a stale paint.
///
/// Called directly on `ProjectState`, not through the Notes button on a displayed row: the two
/// versions this race needs are mocked under ids of their own, not the two the versions tab
/// already shows.
async fn opening_notes_again_before_a_slow_fetch_lands_keeps_the_new_one(app: &TestApp) {
    let state = app.window.global::<ProjectState>();
    state.invoke_open_notes(support::RACE_SLOW_VERSION_ID.into(), "slow".into());
    support::pump();
    state.invoke_open_notes(support::RACE_FAST_VERSION_ID.into(), "fast".into());

    app.wait_until(
        "the fast changelog to land",
        |window| {
            let state = window.global::<ProjectState>();
            state.get_notes_open()
                && !state.get_notes_loading()
                && state.get_notes_title() == "fast"
        },
        QUICK,
    )
    .await;
    assert!(
        notes_blocks(&app.window)
            .iter()
            .any(|(kind, text)| kind.starts_with("heading") && text == support::RACE_FAST_HEADING),
        "the fast changelog's own heading is on screen: {:?}",
        notes_blocks(&app.window)
    );

    // The slow fetch, started first, is still due to land and post its own result. If
    // `open_notes`'s done closure painted whatever arrived last instead of checking its own
    // generation, this wait is where the slow changelog would overwrite the fast one.
    support::yield_to_loop(Duration::from_millis(500)).await;
    assert_eq!(
        app.window
            .global::<ProjectState>()
            .get_notes_title()
            .to_string(),
        "fast",
        "the slow open's stale result must not overwrite the title of the one that replaced it"
    );
    assert!(
        notes_blocks(&app.window)
            .iter()
            .any(|(kind, text)| kind.starts_with("heading") && text == support::RACE_FAST_HEADING),
        "and not its blocks either: {:?}",
        notes_blocks(&app.window)
    );

    app.window.global::<ProjectState>().invoke_close_notes();
    app.wait_until(
        "the notes modal to close",
        |window| !window.global::<ProjectState>().get_notes_open(),
        QUICK,
    )
    .await;
}

/// (j) Opening a project while an earlier, slower open for a different project is still in
/// flight must not let the slow one win once it lands: `open`'s done closure (`run_reporting`)
/// checks its own generation before painting anything, the same guard `open_notes` carries.
///
/// Called directly on `ProjectState.open`, not through a browser row: the two projects this
/// race needs are mocked under ids of their own.
async fn opening_a_project_again_before_a_slow_load_lands_keeps_the_new_one(app: &TestApp) {
    let state = app.window.global::<ProjectState>();
    state.invoke_open(
        "modrinth".into(),
        support::RACE_SLOW_PROJECT.into(),
        SLUG.into(),
        Screen::Browser,
        "".into(),
        "".into(),
    );
    support::pump();
    state.invoke_open(
        "modrinth".into(),
        support::RACE_FAST_PROJECT.into(),
        SLUG.into(),
        Screen::Browser,
        "".into(),
        "".into(),
    );

    app.wait_until(
        "the fast project to load",
        |window| {
            let state = window.global::<ProjectState>();
            state.get_title() == support::RACE_FAST_PROJECT_TITLE && !state.get_loading()
        },
        QUICK,
    )
    .await;

    // The slow open, started first, is still due to land. If `open`'s done closure painted
    // whatever arrived last instead of checking its own generation, this wait is where the
    // slow project would overwrite the fast one's title.
    support::yield_to_loop(Duration::from_millis(500)).await;
    assert_eq!(
        app.window.global::<ProjectState>().get_title().to_string(),
        support::RACE_FAST_PROJECT_TITLE,
        "the slow open's stale result must not overwrite the title of the project that \
         replaced it"
    );
}

/// (h) Escape on the details screen goes Back, same as the button.
async fn escape_does_what_back_does(app: &TestApp) {
    app.click_nth("InstanceScreen::row_source_button", 0);
    wait_for_details(app).await;
    assert_eq!(
        app.window.global::<ProjectState>().get_return_to(),
        Screen::Instance,
        "reopened from the content list, so Back and Escape both return here"
    );

    app.press_key(slint::platform::Key::Escape);
    app.wait_until(
        "Escape to return to the instance screen",
        |window| window.global::<App>().get_screen() == Screen::Instance,
        QUICK,
    )
    .await;
}
