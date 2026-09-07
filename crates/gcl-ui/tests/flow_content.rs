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

use gcl_core::instances::model::Loader;
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

    let driver = Rc::clone(&app);
    support::run(async move {
        let app = &driver;
        searching_and_adding_a_mod_installs_its_file(app).await;
        disabling_and_enabling_renames_the_file(app).await;
        removing_deletes_the_file(app).await;
        installing_a_modpack_from_the_pack_search(app).await;
        installing_a_modpack_from_a_file(app).await;
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
    assert_eq!(
        app.window
            .global::<BrowserState>()
            .get_target_slug()
            .to_string(),
        SLUG,
        "Add content names the instance it was pressed on as the target"
    );

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
    assert_eq!(
        app.el_nth("BrowserScreen::row_install", 0)
            .accessible_label(),
        Some(format!("Add to {INSTANCE}").into()),
        "the row's button names the instance it adds to"
    );

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

    wait_for_new_instance(app, FILE_INSTANCE, "from-file").await;
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
