//! The content browser, the instance's content tab, and the two modpack installs, driven end
//! to end through the Slint testing backend.
//!
//! One process may hold one Slint backend, so every flow runs inside a single `#[test]`, in
//! order, over one window: search and add a mod, disable it, enable it, remove it, then
//! install a modpack from the pack search and another from a file on disk.
//!
//! Modrinth is a wiremock host over the recorded fixtures, and the one file the mod version
//! points at is served by that same host, so nothing here touches the network.

#![cfg(unix)]

mod support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gcl_core::content::AddRequest;
use gcl_core::instances::model::Loader;
use gcl_core::sources::SourceId;
use gcl_ui::{App, AppWindow, BrowserState, InstanceState, InstancesState, Screen};
use slint::{ComponentHandle, Model};
use support::{Mocks, TestApp};

/// How long a flow waits for a job that only touches the mock host or the disk.
const QUICK: Duration = Duration::from_secs(30);

/// The instance every content sub-flow adds to.
const INSTANCE: &str = "Fabric Test";

/// Its directory name, which is what every launcher call takes.
const SLUG: &str = "fabric-test";

/// The name the modpack installed from the pack search is given.
const PACK_INSTANCE: &str = "Packed";

/// The name the modpack installed from a file on disk is given.
const FILE_INSTANCE: &str = "From File";

/// The instance the latest-version sub-flow measures its rows against. It is its own
/// instance because it starts with content in it, and every sub-flow before it counts the
/// rows of [`INSTANCE`]'s content list.
const LATEST_INSTANCE: &str = "Latest Test";

/// Its directory name.
const LATEST_SLUG: &str = "latest-test";

#[test]
fn the_browser_adds_content_and_installs_a_modpack() {
    support::init_backend();
    let app = Rc::new(TestApp::with(Mocks::modrinth()));
    // Setup only: the instance the flows add to is made through the launcher, because
    // creating one is `flow_instances`' subject, not this one's.
    app.launcher
        .instances()
        .create(
            INSTANCE,
            support::MC,
            Loader::Fabric,
            Some(support::FABRIC.to_string()),
            &BTreeMap::new(),
        )
        .expect("create the instance the flows add to");
    // Setup only: a second instance carrying one mod at an older version and one at the
    // newest, so the browser has all three install states on screen at once. The versions
    // are pinned, because "install the older one" is what makes the Update row.
    app.launcher
        .instances()
        .create(
            LATEST_INSTANCE,
            support::MC,
            Loader::Fabric,
            Some(support::FABRIC.to_string()),
            &BTreeMap::new(),
        )
        .expect("create the instance the latest-version flow measures against");
    add_pinned(&app, support::OLDER_PROJECT, support::OLDER_INSTALLED_ID);
    add_pinned(&app, support::CURRENT_PROJECT, support::CURRENT_VERSION_ID);

    let driver = Rc::clone(&app);
    support::run(async move {
        let app = &driver;
        searching_and_adding_a_mod_installs_its_file(app).await;
        disabling_and_enabling_renames_the_file(app).await;
        removing_deletes_the_file(app).await;
        installing_a_modpack_from_the_pack_search(app).await;
        installing_a_modpack_from_a_file(app).await;
        rows_show_the_latest_version_and_what_is_installed(app).await;
        the_content_list_stripes_its_rows(app).await;
    });
}

/// Installs one project into [`LATEST_SLUG`] at exactly `version`, before any flow runs.
fn add_pinned(app: &TestApp, project: &str, version: &str) {
    app.launcher
        .add_content(
            LATEST_SLUG,
            AddRequest {
                source: SourceId::Modrinth,
                project: project.to_string(),
                version: Some(version.to_string()),
                kind: None,
                world: None,
            },
        )
        .unwrap_or_else(|err| panic!("install {project} at {version}: {err}"));
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

/// The names of the content rows the detail screen is showing.
fn content_names(window: &AppWindow) -> Vec<String> {
    window
        .global::<InstanceState>()
        .get_content()
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

/// The titles of the modpack rows the browser is showing.
fn pack_titles(window: &AppWindow) -> Vec<String> {
    window
        .global::<BrowserState>()
        .get_pack_rows()
        .iter()
        .map(|row| row.title.to_string())
        .collect()
}

/// The file the added mod has on disk, enabled or disabled.
fn mod_file(app: &TestApp, disabled: bool) -> PathBuf {
    let name = match disabled {
        true => format!("{}.disabled", support::MOD_FILE_NAME),
        false => support::MOD_FILE_NAME.to_string(),
    };
    app.launcher
        .root()
        .instance_dir(SLUG)
        .join(".minecraft")
        .join("mods")
        .join(name)
}

/// Opens the instance list, then the one instance's detail screen.
async fn open_the_instance(app: &TestApp) {
    app.click("Rail::rail_instances");
    // The window was built before the setup instance existed, and the shell starts on this
    // screen, so nothing has navigated to it yet. Refresh is what a user would press.
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

/// Waits until the browser has finished loading its sources and its instance list.
///
/// The status line is the last thing one open writes, and it can only hold one of these
/// three afterwards, so it is the signal that the whole load has landed.
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

/// (a) Add content from the detail screen, search Modrinth, and add the first hit.
async fn searching_and_adding_a_mod_installs_its_file(app: &TestApp) {
    open_the_instance(app).await;

    app.click("InstanceScreen::add_content_button");
    wait_for_browser(app).await;
    assert_eq!(
        app.window.global::<BrowserState>().get_query().to_string(),
        "",
        "the browser opens on an empty query: nobody has searched for anything yet"
    );
    assert!(
        row_titles(&app.window).is_empty(),
        "and on no hits. A sample row would be Sodium under the same project id as the \
         fixture, so it would pass the search assertions below without a search having run"
    );
    assert!(
        !app.has("BrowserScreen::results_header"),
        "and on no column header: there are no rows for it to sit over yet"
    );
    assert_eq!(
        app.window
            .global::<BrowserState>()
            .get_target_slug()
            .to_string(),
        SLUG,
        "Add content names the instance it was pressed on as the target"
    );

    // Enter in the field runs the search, the same as pressing the button: a real click
    // focuses it first, since a flow's `type_into` sets the value through an accessible
    // action rather than a keystroke.
    app.click("SearchBox::search_field");
    app.type_into("SearchBox::search_field", "sodium");
    support::pump();
    app.press_key(slint::platform::Key::Return);
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
    assert_eq!(
        app.el_nth("BrowserScreen::row_install", 0)
            .accessible_label(),
        Some(format!("Add to {INSTANCE}").into()),
        "the row's button names the instance it adds to"
    );
    assert!(
        app.has("BrowserScreen::results_header"),
        "the column header comes up with the rows it labels"
    );
    assert!(
        !app.has("BrowserScreen::title_touch"),
        "and the title is not a click target of its own: it has no `TouchArea`, no hover \
         cue and no underline. `ListRow`'s own area under it is what opens the details"
    );
    a_long_title_wraps_and_a_description_loses_its_line_breaks(app);

    app.click_nth("BrowserScreen::row_install", 0);
    app.wait_until(
        "the add to finish",
        |window| window.global::<BrowserState>().get_status() == "installed 1",
        QUICK,
    )
    .await;
    assert!(
        mod_file(app, false).is_file(),
        "the mod's file is in the instance's mods folder"
    );
}

/// (a2) A row shows the whole of a long title and folds a description's line breaks away.
///
/// The mock gives the second hit a 90-character title and a description written over three
/// lines. A row that elided the title, or capped its width, would show something shorter than
/// the title itself; a description that kept its `\n` would print blank lines inside the row.
fn a_long_title_wraps_and_a_description_loses_its_line_breaks(app: &TestApp) {
    let title = app.el_nth("BrowserScreen::row_title", 1);
    assert_eq!(
        title.accessible_label().map(|label| label.to_string()),
        Some(support::LONG_TITLE.to_string()),
        "the row carries the whole title, not a shortened one"
    );
    assert!(
        app.el_nth("BrowserScreen::row_open", 1).size().height
            > app.el_nth("BrowserScreen::row_open", 0).size().height,
        "and is taller than the row above it, so the wrapped line is not clipped away"
    );

    let description = app
        .window
        .global::<BrowserState>()
        .get_rows()
        .iter()
        .nth(1)
        .map(|row| row.description.to_string())
        .expect("the search found the second hit");
    assert!(
        !description.contains('\n') && !description.contains('\r'),
        "the description reaches the row as one line the row wraps itself: {description:?}"
    );
    assert_eq!(
        description, "A Sodium addon. It adds options and more options.",
        "every line break became one space, and no text was lost"
    );
}

/// (b) The content tab lists the mod, and its button renames the file both ways.
async fn disabling_and_enabling_renames_the_file(app: &TestApp) {
    app.click("Rail::rail_instance");
    app.wait_until(
        "the content tab to list the mod",
        |window| !content_names(window).is_empty(),
        QUICK,
    )
    .await;

    assert_eq!(
        app.el_nth("InstanceScreen::row_toggle", 0)
            .accessible_label(),
        Some("Disable".into()),
        "an enabled row offers Disable"
    );
    app.click_nth("InstanceScreen::row_toggle", 0);
    app.wait_until(
        "the row to read as disabled",
        |window| {
            window
                .global::<InstanceState>()
                .get_content()
                .iter()
                .any(|row| !row.enabled)
        },
        QUICK,
    )
    .await;
    assert!(
        mod_file(app, true).is_file() && !mod_file(app, false).exists(),
        "disabling renamed the file to `.disabled`"
    );

    app.click_nth("InstanceScreen::row_toggle", 0);
    app.wait_until(
        "the row to read as enabled again",
        |window| {
            window
                .global::<InstanceState>()
                .get_content()
                .iter()
                .any(|row| row.enabled)
        },
        QUICK,
    )
    .await;
    assert!(
        mod_file(app, false).is_file() && !mod_file(app, true).exists(),
        "enabling renamed the file back"
    );
}

/// (c) Remove drops the row and deletes the file.
async fn removing_deletes_the_file(app: &TestApp) {
    app.click_nth("InstanceScreen::row_delete", 0);
    app.wait_until(
        "the content list to empty",
        |window| content_names(window).is_empty(),
        QUICK,
    )
    .await;
    assert!(
        !mod_file(app, false).exists() && !mod_file(app, true).exists(),
        "the file is gone from the mods folder"
    );
    assert!(
        app.launcher
            .list_content(SLUG)
            .expect("read content")
            .is_empty(),
        "the instance no longer records the entry"
    );
}

/// (d) The modpack kind searches for packs, and Install makes a new instance.
async fn installing_a_modpack_from_the_pack_search(app: &TestApp) {
    app.click("Rail::rail_browser");
    wait_for_browser(app).await;

    let kinds = app.window.global::<BrowserState>().get_kind_labels();
    let modpack = (0..kinds.row_count())
        .find(|i| kinds.row_data(*i).is_some_and(|label| label == "modpack"))
        .expect("Modrinth offers the modpack kind");
    app.select_combo("BrowserScreen::kind_combo", modpack);
    app.wait_until(
        "the modpack panel to come up",
        |_| app.has("BrowserScreen::install_pack_button"),
        QUICK,
    )
    .await;

    app.type_into("SearchBox::search_field", "fabulously");
    support::pump();
    app.click("BrowserScreen::search_button");
    app.wait_until(
        "the modpack results to arrive",
        |window| !pack_titles(window).is_empty(),
        QUICK,
    )
    .await;
    assert_eq!(
        pack_titles(&app.window).first().map(String::as_str),
        Some(support::PACK_TITLE),
        "the first row is the first hit of the recorded pack search"
    );
    assert_eq!(
        app.el_nth("BrowserScreen::pack_row", 0).accessible_label(),
        Some(support::PACK_TITLE.into()),
        "the row is labelled with the pack's title"
    );

    app.click_nth("BrowserScreen::pack_install_button", 0);
    app.wait_until(
        "the name prompt to open",
        |window| window.global::<BrowserState>().get_name_open(),
        QUICK,
    )
    .await;
    app.type_into("PromptDialog::name_field", PACK_INSTANCE);
    support::pump();
    app.click("Dialog::confirm_button");

    wait_for_new_instance(app, PACK_INSTANCE, "packed").await;
}

/// (e) The file panel imports a `.mrpack` sitting on disk.
async fn installing_a_modpack_from_a_file(app: &TestApp) {
    let pack = app.root().join("hand-made.mrpack");
    std::fs::write(&pack, support::mrpack_bytes(&app.server_uri(), "Hand Made"))
        .expect("write the modpack archive");

    app.click("Rail::rail_browser");
    wait_for_browser(app).await;
    app.wait_until(
        "the modpack panel to come up",
        |_| app.has("BrowserScreen::pack_path_field"),
        QUICK,
    )
    .await;

    app.type_into("BrowserScreen::pack_path_field", &pack.to_string_lossy());
    support::pump();
    app.click("BrowserScreen::install_file_button");
    app.wait_until(
        "the name prompt to open",
        |window| window.global::<BrowserState>().get_name_open(),
        QUICK,
    )
    .await;
    app.type_into("PromptDialog::name_field", FILE_INSTANCE);
    support::pump();
    app.click("Dialog::confirm_button");

    // `wait_for_new_instance` ends on the instance list, having checked the new instance is
    // there; open its detail screen for the content-list assertions below.
    wait_for_new_instance(app, FILE_INSTANCE, "from-file").await;
    app.click("Rail::rail_instance");
    app.wait_until(
        "the detail screen to show the new instance",
        |window| window.global::<InstanceState>().get_name() == FILE_INSTANCE,
        QUICK,
    )
    .await;

    // (e2) The packaged mod's hash matches no known project, so it is recorded with the
    // `file` source and its row offers no source page.
    app.wait_for("InstanceScreen::row_source_button", QUICK)
        .await;
    let source_button = app.el_nth("InstanceScreen::row_source_button", 0);
    assert_eq!(
        source_button.accessible_label(),
        Some("No source page".into()),
        "a `file` entry names no page to open"
    );
    assert_eq!(
        source_button.accessible_enabled(),
        Some(false),
        "and its button is disabled"
    );

    // (e3) Adding a second mod keeps the content list in name order.
    app.click("InstanceScreen::add_content_button");
    wait_for_browser(app).await;
    // The kind combo is still on `modpack` from installing the file above; back to `mod`
    // so Search runs a content search rather than another pack search.
    let kinds = app.window.global::<BrowserState>().get_kind_labels();
    let mod_kind = (0..kinds.row_count())
        .find(|i| kinds.row_data(*i).is_some_and(|label| label == "mod"))
        .expect("Modrinth offers the mod kind");
    app.select_combo("BrowserScreen::kind_combo", mod_kind);
    app.type_into("SearchBox::search_field", "sodium");
    support::pump();
    app.click("BrowserScreen::search_button");
    app.wait_until(
        "the search results to arrive",
        |window| !row_titles(window).is_empty(),
        QUICK,
    )
    .await;
    app.click_nth("BrowserScreen::row_install", 0);
    app.wait_until(
        "the add to finish",
        |window| window.global::<BrowserState>().get_status() == "installed 1",
        QUICK,
    )
    .await;

    app.click("Rail::rail_instance");
    app.wait_until(
        "the content list to show both entries",
        |window| content_names(window).len() == 2,
        QUICK,
    )
    .await;
    assert_eq!(
        content_names(&app.window),
        vec!["pack-mod".to_string(), support::MOD_TITLE.to_string()],
        "the content list is in name order, case-insensitively"
    );
}

/// Waits for an import to land, then checks the new instance is on disk and in the list.
async fn wait_for_new_instance(app: &TestApp, name: &str, slug: &str) {
    app.wait_until(
        &format!("the import of `{name}` to finish"),
        |window| {
            window.global::<App>().get_screen() == Screen::Instance
                && window.global::<InstanceState>().get_name() == name
        },
        QUICK,
    )
    .await;
    let dir = app.launcher.root().instance_dir(slug);
    assert!(dir.is_dir(), "the new instance is on disk at {dir:?}");
    assert!(
        dir.join(".minecraft")
            .join(support::PACK_MOD_PATH)
            .is_file(),
        "the pack's one file was downloaded into the new instance"
    );

    app.click("Rail::rail_instances");
    app.wait_until(
        "the list to show the new instance",
        |window| instance_names(window).iter().any(|row| row == name),
        QUICK,
    )
    .await;
}

/// (f) Every row says what the newest version for the target instance is, whether that
/// instance has it, and whether the copy it has is behind; Update replaces the file; and
/// changing the target instance re-answers all three for the new one.
///
/// The three hits of the recorded search page stand for the three answers: the first is not
/// installed anywhere, the second is installed at an older version, and the third is
/// installed at the newest one. The setup at the top of this file pinned those two versions.
async fn rows_show_the_latest_version_and_what_is_installed(app: &TestApp) {
    // The rail's browser entry, not the detail screen's Add content: only the rail leaves
    // the target ComboBox on screen, and this flow changes the target with it.
    app.click("Rail::rail_browser");
    wait_for_browser(app).await;
    select_kind(app, "mod");
    select_target(app, LATEST_INSTANCE).await;

    app.type_into("SearchBox::search_field", "sodium");
    support::pump();
    app.click("BrowserScreen::search_button");
    app.wait_until(
        "the search results to arrive",
        |window| row_titles(window).len() == 3,
        QUICK,
    )
    .await;
    app.wait_until(
        "every row to answer what its latest version is",
        |_| !state_texts(app).iter().any(|text| text == CHECKING),
        QUICK,
    )
    .await;

    assert_eq!(
        state_texts(app),
        vec![
            format!("Latest {}", support::MOD_VERSION),
            format!("Installed {} ↑", support::OLDER_INSTALLED_NUMBER),
            format!("Installed {}", support::CURRENT_VERSION_NUMBER),
        ],
        "one row per install state: nothing installed, an older copy, and the newest copy. \
         Each line is the short form that fits the button column: no target named, and the \
         `↑` is what says an update is waiting"
    );
    assert!(
        app.has_label(&format!("Version for {} fabric", support::MC)),
        "the target is named once, in the column header over the buttons"
    );
    assert_eq!(
        stripes(app, "BrowserScreen::row_open"),
        vec!["", "alt", ""],
        "and the results stripe: every odd row is tinted, every even one is not"
    );
    assert_eq!(
        install_labels(app),
        vec![
            format!("Add to {LATEST_INSTANCE}"),
            "Update".to_string(),
            "Installed".to_string(),
        ],
        "and each row's button offers what that state allows"
    );
    assert_eq!(
        app.el_nth("BrowserScreen::row_install", 2)
            .accessible_enabled(),
        Some(false),
        "the row that is already up to date has nothing to press"
    );

    updating_replaces_the_older_file(app).await;
    changing_the_target_re_answers_every_row(app).await;
    adding_leaves_the_row_reading_installed(app).await;
}

/// (f4) A plain Add answers the same way an Update does: the row it was pressed on says the
/// instance now has that mod, without another search.
async fn adding_leaves_the_row_reading_installed(app: &TestApp) {
    // The list must not be rebuilt by an install: a fresh `set_rows` would blink every row
    // away and take the icons with it, since those are decoded once per search. So capture
    // what identity there is to lose — the model behind `BrowserState.rows` and the size of
    // every decoded icon in it — with the icons already landed, and check both afterwards.
    app.wait_until(
        "every row's icon to have been fetched and decoded",
        |window| {
            window
                .global::<BrowserState>()
                .get_rows()
                .iter()
                .all(|row| row.icon.size().width > 0)
        },
        QUICK,
    )
    .await;
    let model_before = app.window.global::<BrowserState>().get_rows();
    let icons_before = icon_sizes(&app.window);

    app.click_nth("BrowserScreen::row_install", 1);
    app.wait_until(
        "the add to finish and the row to catch up with it",
        |window| {
            window
                .global::<BrowserState>()
                .get_rows()
                .iter()
                .nth(1)
                .is_some_and(|row| row.state == "installed")
        },
        QUICK,
    )
    .await;
    assert_eq!(
        state_texts(app)[1],
        format!("Installed {}", support::OLDER_LATEST_NUMBER),
        "the row the Add was pressed on reads as installed"
    );
    assert_eq!(
        install_labels(app)[1],
        "Installed",
        "and its button has nothing left to offer"
    );
    assert!(
        app.launcher
            .root()
            .instance_dir(SLUG)
            .join(".minecraft")
            .join("mods")
            .join(support::OLDER_LATEST_FILE)
            .is_file(),
        "the file really did land in the other instance"
    );

    let model_after = app.window.global::<BrowserState>().get_rows();
    assert!(
        model_before == model_after,
        "the install patched the one row it changed: `BrowserState.rows` is still the same \
         model, not a new one built over the same hits"
    );
    assert_eq!(
        icon_sizes(&app.window),
        icons_before,
        "so every row kept the icon it had already decoded"
    );
    assert!(
        !state_texts(app).iter().any(|text| text == CHECKING),
        "and no row went back to `{CHECKING}`: only the row that changed was rewritten"
    );
}

/// The size of every search row's decoded icon, in row order. `(0, 0)` is a row whose icon
/// has not landed, which is also what a row rebuilt from scratch would read as.
fn icon_sizes(window: &AppWindow) -> Vec<(u32, u32)> {
    window
        .global::<BrowserState>()
        .get_rows()
        .iter()
        .map(|row| {
            let size = row.icon.size();
            (size.width, size.height)
        })
        .collect()
}

/// (g) The instance's content list stripes its rows the same way the browser results do.
///
/// [`LATEST_INSTANCE`] is the one instance with more than one mod in it, so it is the one
/// that can show a tint and no tint next to each other.
async fn the_content_list_stripes_its_rows(app: &TestApp) {
    app.click("Rail::rail_instances");
    app.click("InstancesScreen::refresh_button");
    app.wait_until(
        "the instance list to show the instance with two mods in it",
        |window| instance_names(window).iter().any(|n| n == LATEST_INSTANCE),
        QUICK,
    )
    .await;
    // The rows reach the model one turn before the repeater has built an element for each
    // of them, and this flow addresses them by element, not by index into the model.
    app.wait_until(
        "the list to have drawn a row per instance",
        |_| app.all("InstancesScreen::row_open").len() == instance_names(&app.window).len(),
        QUICK,
    )
    .await;
    let index = instance_names(&app.window)
        .iter()
        .position(|name| name == LATEST_INSTANCE)
        .expect("the instance with two mods is in the list");
    app.click_nth("InstancesScreen::row_open", index);
    app.wait_until(
        "its content tab to list both mods",
        |window| {
            window.global::<App>().get_screen() == Screen::Instance
                && content_names(window).len() == 2
        },
        QUICK,
    )
    .await;
    assert_eq!(
        stripes(app, "InstanceScreen::row_open"),
        vec!["", "alt"],
        "the second row is tinted and the first is not"
    );
}

/// The line every search row shows while its latest version has not been resolved yet.
const CHECKING: &str = "Checking…";

/// The state line under each search row, in row order.
fn state_texts(app: &TestApp) -> Vec<String> {
    app.all("BrowserScreen::row_state_text")
        .iter()
        .map(|row| {
            row.accessible_label()
                .map(|label| label.to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// The label on each search row's install button, in row order.
fn install_labels(app: &TestApp) -> Vec<String> {
    app.all("BrowserScreen::row_install")
        .iter()
        .map(|row| {
            row.accessible_label()
                .map(|label| label.to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// What each row of a striped list reads as: `"alt"` for a tinted row, `""` for a plain
/// one. `ListRow` puts its own `alt` into `accessible-description`, because a background
/// color is not otherwise readable from the element tree.
fn stripes(app: &TestApp, id: &str) -> Vec<String> {
    app.all(id)
        .iter()
        .map(|row| {
            row.accessible_description()
                .map(|text| text.to_string())
                .unwrap_or_default()
        })
        .collect()
}

/// Points the kind ComboBox at one kind by name.
fn select_kind(app: &TestApp, want: &str) {
    let kinds = app.window.global::<BrowserState>().get_kind_labels();
    let index = (0..kinds.row_count())
        .find(|i| kinds.row_data(*i).is_some_and(|label| label == want))
        .unwrap_or_else(|| panic!("Modrinth offers the {want} kind"));
    app.select_combo("BrowserScreen::kind_combo", index);
}

/// Points the target ComboBox at one instance by name and waits for the screen to follow.
async fn select_target(app: &TestApp, name: &str) {
    let targets = app.window.global::<BrowserState>().get_target_labels();
    let index = (0..targets.row_count())
        .find(|i| targets.row_data(*i).is_some_and(|label| label == name))
        .unwrap_or_else(|| panic!("`{name}` is one of the browser's targets"));
    app.select_combo("BrowserScreen::target_combo", index);
    app.wait_until(
        &format!("the browser to add to `{name}`"),
        |window| window.global::<BrowserState>().get_target_name() == name,
        QUICK,
    )
    .await;
}

/// (f2) Update installs the newest version over the older one: one file in `mods/`, one
/// entry in `instance.toml`, and a row that now reads as installed.
async fn updating_replaces_the_older_file(app: &TestApp) {
    let mods = app
        .launcher
        .root()
        .instance_dir(LATEST_SLUG)
        .join(".minecraft")
        .join("mods");
    assert!(
        mods.join(support::OLDER_INSTALLED_FILE).is_file(),
        "the older file is what the setup installed"
    );

    app.click_nth("BrowserScreen::row_install", 1);
    app.wait_until(
        "the update to finish and the row to catch up with it",
        |window| {
            window
                .global::<BrowserState>()
                .get_rows()
                .iter()
                .nth(1)
                .is_some_and(|row| row.state == "installed")
        },
        QUICK,
    )
    .await;

    assert!(
        mods.join(support::OLDER_LATEST_FILE).is_file(),
        "the newest version's file is in the mods folder"
    );
    assert!(
        !mods.join(support::OLDER_INSTALLED_FILE).exists(),
        "and the file it replaced is gone, not left beside it"
    );
    let entries: Vec<_> = app
        .launcher
        .list_content(LATEST_SLUG)
        .expect("read the instance's content")
        .into_iter()
        .filter(|entry| entry.project_id == support::OLDER_PROJECT)
        .collect();
    assert_eq!(entries.len(), 1, "one entry for the project, not two");
    assert_eq!(
        entries[0].version_id,
        support::OLDER_LATEST_ID,
        "recorded at the version Update pinned"
    );
    assert_eq!(
        state_texts(app)[1],
        format!("Installed {}", support::OLDER_LATEST_NUMBER),
        "and the row says so without another search"
    );
    assert_eq!(
        install_labels(app)[1],
        "Installed",
        "so its button has nothing left to offer"
    );
}

/// (f3) Picking another instance re-answers every row against that one: the mod just
/// updated is not in it, so the same row goes back to offering Add.
async fn changing_the_target_re_answers_every_row(app: &TestApp) {
    select_target(app, INSTANCE).await;
    // The rows go back to "Checking…" the moment the target changes — the answers on
    // screen were about the instance the user just left, so `refresh_latest` clears every
    // row before it re-runs the job. That reset is synchronous, but a fast enough mock can
    // already have answered again by the time this line runs (`select_target`'s own await
    // pumps the loop), so this asserts only the settled state rather than reading `CHECKING`
    // at a moment nothing guarantees it is still there.
    app.wait_until(
        "the rows to be re-checked against the new target",
        |_| !state_texts(app).iter().any(|text| text == CHECKING),
        QUICK,
    )
    .await;
    assert_eq!(
        state_texts(app)[1],
        format!("Latest {}", support::OLDER_LATEST_NUMBER),
        "the instance that has none of it hears only what the newest version is"
    );
    assert_eq!(
        install_labels(app)[1],
        format!("Add to {INSTANCE}"),
        "and the button offers to add it there"
    );
}
