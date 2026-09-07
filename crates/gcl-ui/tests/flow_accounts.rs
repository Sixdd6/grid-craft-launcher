//! The accounts screen, driven end to end through the Slint testing backend.
//!
//! One process may hold one Slint backend, so every flow runs inside a single `#[test]`, in
//! order. It runs over three windows, because what the screen offers depends on how the
//! launcher behind it was built: one with no Microsoft client id, where sign-in is off; one
//! with a client id and a mock Microsoft that approves the sign-in; and one whose mock never
//! approves it, so Cancel has something to cancel.
//!
//! Every Microsoft endpoint is a wiremock host, so nothing here touches the network, no
//! keyring is opened — the launcher keeps its refresh token in a memory store — and no token
//! is ever printed.

#![cfg(unix)]

mod support;

use std::rc::Rc;
use std::time::Duration;

use gcl_ui::{AccountsState, App, AppWindow, Screen};
use slint::{ComponentHandle, Model};
use support::{Mocks, TestApp};

/// How long a flow waits for a job that only touches the mock host or the disk.
const QUICK: Duration = Duration::from_secs(30);

/// The first offline account the flow adds, which becomes the active one.
const FIRST: &str = "Steve";

/// The second offline account, which the flow then makes active and removes.
const SECOND: &str = "Alex";

#[test]
fn the_accounts_screen_adds_selects_removes_and_signs_in() {
    support::init_backend();
    // All three windows are built here, not inside the flow: a `TestApp` dropped inside the
    // flow's future would be dropped as the process tears its thread locals down, and the
    // mock server's own drop needs those. Only one is on screen at a time.
    let plain = Rc::new(TestApp::with(Mocks::default()));
    let signed_in = Rc::new(TestApp::with(Mocks::msa()));
    signed_in.hide();
    let cancelled = Rc::new(TestApp::with(Mocks::msa_pending()));
    cancelled.hide();

    let (a, b, c) = (
        Rc::clone(&plain),
        Rc::clone(&signed_in),
        Rc::clone(&cancelled),
    );
    support::run(async move {
        adding_selecting_and_removing_offline_accounts(&a).await;
        a.hide();

        b.show();
        signing_in_with_microsoft_adds_the_profile(&b).await;
        b.hide();

        c.show();
        cancelling_a_sign_in_adds_nothing(&c).await;
        c.hide();
    });
}

/// The names of the account rows the screen is showing.
fn names(window: &AppWindow) -> Vec<String> {
    window
        .global::<AccountsState>()
        .get_rows()
        .iter()
        .map(|row| row.name.to_string())
        .collect()
}

/// The position of the row with this name.
fn index_of(window: &AppWindow, name: &str) -> usize {
    names(window)
        .iter()
        .position(|row| row == name)
        .unwrap_or_else(|| panic!("no account named `{name}` in {:?}", names(window)))
}

/// Whether the account with this name is the launch default.
fn is_active(window: &AppWindow, name: &str) -> bool {
    window
        .global::<AccountsState>()
        .get_rows()
        .iter()
        .any(|row| row.name == name && row.active)
}

/// Opens the accounts screen and waits for the store to be read.
async fn open_accounts(app: &TestApp) {
    app.click("Rail::rail_accounts");
    app.wait_until(
        "the accounts screen to load",
        |window| {
            window.global::<App>().get_screen() == Screen::Accounts
                && !window.global::<AccountsState>().get_busy()
        },
        QUICK,
    )
    .await;
}

/// Adds one offline account through the name prompt and waits for its row.
async fn add_offline(app: &TestApp, name: &str) {
    app.click("AccountsScreen::add_offline_button");
    app.wait_until(
        "the name prompt to open",
        |window| window.global::<AccountsState>().get_prompt_open(),
        QUICK,
    )
    .await;
    app.type_into("PromptDialog::name_field", name);
    support::pump();
    app.click("Dialog::confirm_button");
    app.wait_until(
        &format!("the row for `{name}` to appear"),
        |window| names(window).iter().any(|row| row == name),
        QUICK,
    )
    .await;
}

/// (a) With no client id: sign-in is off, and offline accounts can be added, picked, removed.
async fn adding_selecting_and_removing_offline_accounts(app: &TestApp) {
    open_accounts(app).await;
    assert!(
        names(&app.window).is_empty(),
        "a fresh root has no accounts"
    );
    assert_eq!(
        app.el("AccountsScreen::sign_in_button")
            .accessible_enabled(),
        Some(false),
        "with no client id, Microsoft sign-in is off"
    );
    assert!(!app.window.global::<AccountsState>().get_msa_available());
    assert!(
        app.window
            .global::<AccountsState>()
            .get_msa_hint()
            .contains("client id"),
        "the screen says why sign-in is off"
    );

    add_offline(app, FIRST).await;
    assert!(
        is_active(&app.window, FIRST),
        "the first account added becomes the active one"
    );

    add_offline(app, SECOND).await;
    let second = index_of(&app.window, SECOND);
    app.click_nth("AccountsScreen::row_select", second);
    app.wait_until(
        "the second account to become active",
        |window| is_active(window, SECOND),
        QUICK,
    )
    .await;
    assert!(
        !is_active(&app.window, FIRST),
        "only one account is active at a time"
    );
    assert_eq!(
        app.launcher
            .accounts()
            .active()
            .expect("read the active account")
            .map(|account| account.name),
        Some(SECOND.to_string()),
        "the choice was written to the account store"
    );

    let second = index_of(&app.window, SECOND);
    app.click_nth("AccountsScreen::row_delete", second);
    app.wait_until(
        "the remove confirmation to open",
        |window| window.global::<AccountsState>().get_confirm_open(),
        QUICK,
    )
    .await;
    app.click("Dialog::confirm_button");
    app.wait_until(
        "the row to go",
        |window| !names(window).iter().any(|row| row == SECOND),
        QUICK,
    )
    .await;
    assert_eq!(
        names(&app.window),
        vec![FIRST.to_string()],
        "the other account is untouched"
    );
}

/// (b) With a client id and a mock Microsoft: the device code is shown and the sign-in lands.
async fn signing_in_with_microsoft_adds_the_profile(app: &TestApp) {
    open_accounts(app).await;
    assert!(
        app.window.global::<AccountsState>().get_msa_available(),
        "a configured client id turns sign-in on"
    );

    app.click("AccountsScreen::sign_in_button");
    app.wait_until(
        "the device code to arrive",
        |window| {
            let state = window.global::<AccountsState>();
            state.get_device_open() && state.get_device_code() == support::MSA_USER_CODE
        },
        QUICK,
    )
    .await;
    assert_eq!(
        app.el("DeviceCodeDialog::code_text").accessible_value(),
        Some(support::MSA_USER_CODE.into()),
        "the dialog shows the code the user has to type"
    );
    assert_eq!(
        app.window
            .global::<AccountsState>()
            .get_device_uri()
            .to_string(),
        support::MSA_URI,
        "and the page to type it on"
    );

    app.wait_until(
        "the signed-in account to appear",
        |window| names(window).iter().any(|row| row == support::MSA_NAME),
        QUICK,
    )
    .await;
    assert!(
        !app.window.global::<AccountsState>().get_device_open(),
        "a finished sign-in closes the dialog"
    );
    let account = app
        .launcher
        .accounts()
        .find(support::MSA_NAME)
        .expect("read the account store")
        .expect("the profile was saved");
    assert_eq!(account.kind, gcl_core::auth::AccountKind::Msa);
}

/// (c) A sign-in the user cancels closes the dialog and saves nothing.
async fn cancelling_a_sign_in_adds_nothing(app: &TestApp) {
    open_accounts(app).await;

    app.click("AccountsScreen::sign_in_button");
    app.wait_until(
        "the device code to arrive",
        |window| {
            let state = window.global::<AccountsState>();
            state.get_device_open() && state.get_device_code() == support::MSA_USER_CODE
        },
        QUICK,
    )
    .await;

    app.click("Dialog::cancel_button");
    app.wait_until(
        "the sign-in to end",
        |window| {
            let state = window.global::<AccountsState>();
            !state.get_device_open() && !state.get_busy()
        },
        QUICK,
    )
    .await;
    assert_eq!(
        app.window
            .global::<AccountsState>()
            .get_status()
            .to_string(),
        "Sign-in cancelled",
        "a cancel is reported as one, not as a failure"
    );
    assert!(
        names(&app.window).is_empty(),
        "nothing was saved: {:?}",
        names(&app.window)
    );
    assert!(
        app.launcher
            .accounts()
            .list()
            .expect("read the account store")
            .is_empty()
    );
}
