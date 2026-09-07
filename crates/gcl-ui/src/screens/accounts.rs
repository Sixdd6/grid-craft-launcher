//! Wires the accounts screen: the account store, Microsoft sign-in, and the device-code
//! dialog.
//!
//! Every call into `gcl-core` runs off the UI thread, through [`Bridge`] or through the one
//! thread a sign-in owns. The screen itself is pure layout; this module owns the
//! `AccountsState` global that feeds it.
//!
//! A sign-in is not a [`Bridge`] job: it runs for as long as the user takes to approve it, it
//! reports a code back before it finishes, and it has to be cancellable on its own. It gets
//! its own thread and a child of the launcher's cancel token, so cancelling the sign-in
//! leaves every download in flight alone.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gcl_core::auth::msa::DeviceCode;
use gcl_core::auth::offline::offline_account;
use gcl_core::auth::{Account, AccountKind};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tokio_util::sync::CancellationToken;

use crate::bridge::Bridge;
use crate::models::account_row;
use crate::{AccountRow, AccountsState, AppWindow};

/// Shown next to the disabled sign-in button when no Microsoft client id is configured.
///
/// The settings screen writes it too: saving a client id there is what turns sign-in on, so
/// both screens say the same thing about why it is off.
pub const NO_CLIENT_ID: &str = "Microsoft sign-in is off: set a client id in Settings, or \
                            GCL_MSA_CLIENT_ID";

/// The token that cancels the sign-in on screen, and the account a remove is waiting on.
///
/// Both live on the UI thread only, so they are `Rc<RefCell<..>>` rather than locks: every
/// Slint callback runs on that one thread. The token is cleared when the user cancels and
/// replaced when the next sign-in starts; a token left behind by a sign-in that already
/// ended is harmless, because cancelling a finished token does nothing.
#[derive(Clone, Default)]
struct Pending {
    /// Cancels the sign-in the device-code dialog is showing.
    cancel: Rc<RefCell<Option<CancellationToken>>>,
    /// Id of the account the remove confirmation is open for.
    remove: Rc<RefCell<Option<String>>>,
}

/// Binds the `AccountsState` global to the launcher and loads the account list.
pub fn wire(window: &AppWindow, bridge: &Bridge) {
    let state = window.global::<AccountsState>();
    let pending = Pending::default();

    {
        let bridge = bridge.clone();
        state.on_open(move || load(&bridge));
    }

    {
        let weak = bridge.weak().clone();
        state.on_add_offline(move || {
            if let Some(window) = weak.upgrade() {
                let state = window.global::<AccountsState>();
                state.set_prompt_value(SharedString::new());
                state.set_prompt_open(true);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_prompt_ok(move |name| add_offline(&bridge, name.as_str()));
    }

    {
        let weak = bridge.weak().clone();
        state.on_prompt_cancel(move || {
            if let Some(window) = weak.upgrade() {
                window.global::<AccountsState>().set_prompt_open(false);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_select(move |id| select(&bridge, id.as_str()));
    }

    {
        let weak = bridge.weak().clone();
        let pending = pending.clone();
        state.on_remove(move |id| {
            if let Some(window) = weak.upgrade() {
                ask_remove(&window, &pending, id.as_str());
            }
        });
    }

    {
        let bridge = bridge.clone();
        let pending = pending.clone();
        state.on_confirm_yes(move || confirm_remove(&bridge, &pending));
    }

    {
        let weak = bridge.weak().clone();
        let pending = pending.clone();
        state.on_confirm_no(move || {
            *pending.remove.borrow_mut() = None;
            if let Some(window) = weak.upgrade() {
                window.global::<AccountsState>().set_confirm_open(false);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_refresh(move |id| refresh(&bridge, id.as_str()));
    }

    {
        let bridge = bridge.clone();
        let pending = pending.clone();
        state.on_sign_in(move || sign_in(&bridge, &pending));
    }

    {
        let weak = bridge.weak().clone();
        let pending = pending.clone();
        state.on_cancel_sign_in(move || {
            if let Some(token) = pending.cancel.borrow_mut().take() {
                token.cancel();
            }
            if let Some(window) = weak.upgrade() {
                window
                    .global::<AccountsState>()
                    .set_status("Cancelling the sign-in…".into());
            }
        });
    }

    load(bridge);
}

/// Reads every saved account, which one is active, and whether sign-in is possible.
fn load(bridge: &Bridge) {
    busy(bridge, true);
    bridge.run_with_error(
        "Load accounts",
        |launcher| {
            let accounts = launcher.accounts();
            let list = accounts.list()?;
            let active = accounts.active()?.map(|account| account.id);
            Ok((list, active, launcher.msa_available()))
        },
        |window, result| {
            let state = window.global::<AccountsState>();
            state.set_busy(false);
            let Ok((list, active, msa_available)) = result else {
                state.set_status("Could not read the accounts".into());
                return;
            };
            let rows: Vec<AccountRow> = list
                .iter()
                .map(|account| account_row(account, active.as_deref() == Some(&account.id)))
                .collect();
            state.set_status(account_status(rows.len()).into());
            state.set_rows(ModelRc::new(VecModel::from(rows)));
            state.set_msa_available(msa_available);
            state.set_msa_hint(if msa_available { "" } else { NO_CLIENT_ID }.into());
        },
    );
}

/// Saves an offline account for `name` and reloads the list.
fn add_offline(bridge: &Bridge, name: &str) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    if let Some(window) = bridge.weak().upgrade() {
        let state = window.global::<AccountsState>();
        state.set_prompt_open(false);
        state.set_prompt_value(SharedString::new());
    }
    let bridge_after = bridge.clone();
    busy(bridge, true);
    bridge.run_with_error(
        "Add offline account",
        move |launcher| {
            launcher
                .accounts()
                .add(offline_account(&name))
                .map_err(Into::into)
        },
        move |window, result| {
            after(
                window,
                &bridge_after,
                result.map(|account| added_status(&account)),
            );
        },
    );
}

/// Makes one account the launch default.
fn select(bridge: &Bridge, id: &str) {
    let id = id.to_string();
    let bridge_after = bridge.clone();
    busy(bridge, true);
    bridge.run_with_error(
        "Select account",
        move |launcher| launcher.accounts().select(&id).map_err(Into::into),
        move |window, result| {
            after(
                window,
                &bridge_after,
                result.map(|account| format!("{} is the active account", account.name)),
            );
        },
    );
}

/// Opens the remove confirmation for one account.
fn ask_remove(window: &AppWindow, pending: &Pending, id: &str) {
    let state = window.global::<AccountsState>();
    let name = name_of(&state, id).unwrap_or_else(|| id.to_string());
    *pending.remove.borrow_mut() = Some(id.to_string());
    state.set_confirm_text(remove_text(&name).into());
    state.set_confirm_open(true);
}

/// Removes the account the confirmation was opened for.
fn confirm_remove(bridge: &Bridge, pending: &Pending) {
    let Some(id) = pending.remove.borrow_mut().take() else {
        return;
    };
    if let Some(window) = bridge.weak().upgrade() {
        window.global::<AccountsState>().set_confirm_open(false);
    }
    let bridge_after = bridge.clone();
    busy(bridge, true);
    bridge.run_with_error(
        "Remove account",
        move |launcher| launcher.accounts().remove(&id).map_err(Into::into),
        move |window, result| {
            after(
                window,
                &bridge_after,
                result.map(|()| "Account removed".to_string()),
            );
        },
    );
}

/// Signs a saved Microsoft account in again from its stored refresh token.
fn refresh(bridge: &Bridge, id: &str) {
    let id = id.to_string();
    let bridge_after = bridge.clone();
    busy(bridge, true);
    bridge.run_with_error(
        "Refresh account",
        move |launcher| launcher.msa_refresh(&id),
        move |window, result| {
            after(
                window,
                &bridge_after,
                result.map(|account| format!("{} is signed in again", account.name)),
            );
        },
    );
}

/// Starts a Microsoft sign-in on its own thread and opens the device-code dialog.
///
/// The thread blocks in `msa_login_with_cancel` until the user approves the code, the code
/// expires, or the token is cancelled. `on_code` runs on that same thread, before the login
/// finishes: it must not block and must not call back into the launcher, so it only posts the
/// code to the dialog.
fn sign_in(bridge: &Bridge, pending: &Pending) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<AccountsState>();
    if state.get_busy() {
        return;
    }
    let cancel = bridge.launcher().cancel_token().child_token();
    *pending.cancel.borrow_mut() = Some(cancel.clone());

    state.set_busy(true);
    state.set_device_code(SharedString::new());
    state.set_device_uri(SharedString::new());
    state.set_device_open(true);
    state.set_status("Asking Microsoft for a sign-in code…".into());

    let launcher = Arc::clone(bridge.launcher());
    let bridge_after = bridge.clone();
    let weak = bridge.weak().clone();
    let code_weak = bridge.weak().clone();
    std::thread::spawn(move || {
        let on_code = move |code: &DeviceCode| {
            let user_code = code.user_code.clone();
            let uri = code.verification_uri.clone();
            let _ = code_weak.upgrade_in_event_loop(move |window| {
                let state = window.global::<AccountsState>();
                state.set_device_code(user_code.into());
                state.set_device_uri(uri.into());
            });
        };
        let result = launcher.msa_login_with_cancel(&on_code, cancel);
        let _ = weak.upgrade_in_event_loop(move |window| {
            finish_sign_in(&window, &bridge_after, result);
        });
    });
}

/// Closes the device-code dialog and reports how the sign-in ended.
fn finish_sign_in(window: &AppWindow, bridge: &Bridge, result: Result<Account, gcl_core::Error>) {
    let state = window.global::<AccountsState>();
    state.set_busy(false);
    state.set_device_open(false);
    state.set_device_code(SharedString::new());
    state.set_device_uri(SharedString::new());
    match result {
        Ok(account) => {
            state.set_status(signed_in_status(&account.name).into());
            load(bridge);
        }
        // A cancelled sign-in is what the user asked for, so it closes the dialog and says
        // so; it is not an error to put in front of them.
        Err(err) if is_cancelled(&err) => state.set_status("Sign-in cancelled".into()),
        Err(err) => {
            state.set_status("Sign-in failed".into());
            bridge.show_error(window, "Sign in with Microsoft", &err);
        }
    }
}

/// Clears `busy`, reports `status` or a failed step, and reloads the list.
fn after(window: &AppWindow, bridge: &Bridge, status: Result<String, gcl_core::Error>) {
    let state = window.global::<AccountsState>();
    state.set_busy(false);
    match status {
        Ok(text) => {
            state.set_status(text.into());
            load(bridge);
        }
        // The error dialog is already up: `run_with_error` opened it.
        Err(_) => state.set_status("That did not work".into()),
    }
}

/// Sets or clears the flag every button on the screen is disabled by.
fn busy(bridge: &Bridge, busy: bool) {
    if let Some(window) = bridge.weak().upgrade() {
        window.global::<AccountsState>().set_busy(busy);
    }
}

/// The display name of the account with this id, from the rows on screen.
fn name_of(state: &AccountsState<'_>, id: &str) -> Option<String> {
    state
        .get_rows()
        .iter()
        .find(|row| row.id == id)
        .map(|row| row.name.to_string())
}

/// The status line for a list of `count` accounts.
pub fn account_status(count: usize) -> String {
    match count {
        0 => "No accounts yet".to_string(),
        _ => format!("{count} account(s)"),
    }
}

/// The status line after an account was added.
pub fn added_status(account: &Account) -> String {
    match account.kind {
        AccountKind::Offline => format!("added offline account {}", account.name),
        AccountKind::Msa => format!("added Microsoft account {}", account.name),
    }
}

/// The status line after a Microsoft sign-in finished.
pub fn signed_in_status(name: &str) -> String {
    format!("signed in as {name}")
}

/// The body of the remove confirmation for one account.
pub fn remove_text(name: &str) -> String {
    format!(
        "{name} is removed from this launcher. Nothing is changed at Microsoft, and the game \
         files stay where they are."
    )
}

/// Whether an error is the sign-in being cancelled, rather than a failure to report.
pub fn is_cancelled(err: &gcl_core::Error) -> bool {
    matches!(err, gcl_core::Error::Auth(gcl_core::auth::Error::Cancelled))
}

#[cfg(test)]
mod tests;
