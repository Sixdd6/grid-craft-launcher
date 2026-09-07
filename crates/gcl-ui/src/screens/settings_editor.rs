//! Wires the typed settings editor, for both places it is mounted.
//!
//! The settings screen edits the launcher preseed (`config.toml`'s `game_defaults`); the
//! instance detail screen's Settings tab edits one instance's overrides. Only one of them is
//! on screen at a time, so one [`Editor`] and one `SettingsEditorState` global serve both:
//! the screen that opens says which layer it edits, and everything after that is the same
//! code path.
//!
//! Every write goes through the launcher, which validates the value and reports what is
//! wrong. After a write the rows are read again, off the UI thread, so what the editor shows
//! is what is on disk — a rejected value snaps the control back on its own.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use gcl_core::settings::doc::{Layer, Row};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::bridge::Bridge;
use crate::models::settings::{choice_token, stored_value, visible_rows};
use crate::{AppWindow, SettingsEditorState};

/// Which layer the open screen writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum EditTarget {
    /// `config.toml`'s `game_defaults`: what a new instance is preseeded with.
    #[default]
    Defaults,
    /// One instance's `settings_overrides`, by slug.
    Instance(String),
}

impl EditTarget {
    /// The layer a value saved here lands in, which is also the only layer Reset drops.
    pub fn layer(&self) -> Layer {
        match self {
            EditTarget::Defaults => Layer::Preseed,
            EditTarget::Instance(_) => Layer::Override,
        }
    }
}

/// The editor's own state: what it edits, the rows it last read, and which groups are shut.
///
/// Cloning shares all three, so a bridge job can carry a handle and put its result back.
/// Only core values live here: a Slint model is built on the UI thread, from these rows,
/// every time the visible list changes.
#[derive(Clone, Default)]
pub struct Editor {
    target: Arc<Mutex<EditTarget>>,
    rows: Arc<Mutex<Vec<Row>>>,
    collapsed: Arc<Mutex<BTreeSet<String>>>,
}

impl Editor {
    /// A fresh editor, pointed at the launcher defaults with every group open.
    pub fn new() -> Editor {
        Editor::default()
    }

    /// Shows `target`'s settings: remembers the layer, then reads its rows.
    pub fn open(&self, bridge: &Bridge, target: EditTarget) {
        *self.lock_target() = target;
        self.reload(bridge);
    }

    /// Reads the rows for the open target again and refills the editor.
    pub fn reload(&self, bridge: &Bridge) {
        let target = self.target();
        let editor = self.clone();
        busy(bridge, true);
        bridge.run_with_error(
            "Read game settings",
            move |launcher| match &target {
                EditTarget::Defaults => Ok(launcher.settings_rows_for_defaults()),
                EditTarget::Instance(slug) => launcher.settings_rows_for_instance(slug),
            },
            move |window, result| {
                window.global::<SettingsEditorState>().set_busy(false);
                if let Ok(rows) = result {
                    *editor.lock_rows() = rows;
                    editor.refresh(window);
                }
            },
        );
    }

    /// Rebuilds the visible list from the rows last read, the search box, and which groups
    /// are shut.
    pub fn refresh(&self, window: &AppWindow) {
        let state = window.global::<SettingsEditorState>();
        let search = state.get_search().to_string();
        let lines = visible_rows(
            &self.lock_rows(),
            self.target().layer(),
            &search,
            &self.lock_collapsed(),
        );
        state.set_layer_name(match self.target() {
            EditTarget::Defaults => "default".into(),
            EditTarget::Instance(_) => "override".into(),
        });
        state.set_rows(ModelRc::new(VecModel::from(lines)));
    }

    /// The target being edited right now.
    pub fn target(&self) -> EditTarget {
        self.lock_target().clone()
    }

    /// Opens or shuts one group.
    fn toggle_group(&self, window: &AppWindow, group: &str) {
        {
            let mut collapsed = self.lock_collapsed();
            if !collapsed.remove(group) {
                collapsed.insert(group.to_string());
            }
        }
        self.refresh(window);
    }

    fn lock_target(&self) -> std::sync::MutexGuard<'_, EditTarget> {
        self.target.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_rows(&self) -> std::sync::MutexGuard<'_, Vec<Row>> {
        self.rows.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn lock_collapsed(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.collapsed.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// Binds the `SettingsEditorState` global and hands back the editor both screens open.
pub fn wire(window: &AppWindow, bridge: &Bridge) -> Editor {
    let editor = Editor::new();
    let state = window.global::<SettingsEditorState>();

    {
        let (editor, bridge) = (editor.clone(), bridge.clone());
        state.on_changed(move |key, value| {
            let value = stored_value(key.as_str(), value.as_str());
            save(&editor, &bridge, key.as_str(), &value);
        });
    }

    {
        let (editor, bridge) = (editor.clone(), bridge.clone());
        state.on_chose(move |key, index| {
            let Some(token) = choice_token(key.as_str(), index) else {
                status(&bridge, &format!("{} has no choice {index}", key.as_str()));
                return;
            };
            save(&editor, &bridge, key.as_str(), token);
        });
    }

    {
        let (editor, bridge) = (editor.clone(), bridge.clone());
        state.on_reset(move |key| reset(&editor, &bridge, key.as_str()));
    }

    {
        let (editor, weak) = (editor.clone(), bridge.weak().clone());
        state.on_toggle_group(move |group| {
            if let Some(window) = weak.upgrade() {
                editor.toggle_group(&window, group.as_str());
            }
        });
    }

    {
        let (editor, weak) = (editor.clone(), bridge.weak().clone());
        state.on_search_changed(move |_text| {
            if let Some(window) = weak.upgrade() {
                editor.refresh(&window);
            }
        });
    }

    editor
}

/// Writes one value into the edited layer, then reads every row back.
fn save(editor: &Editor, bridge: &Bridge, key: &str, value: &str) {
    let target = editor.target();
    let (job_key, job_value) = (key.to_string(), value.to_string());
    let shown = format!("{key} = {value}");
    let after = (editor.clone(), bridge.clone());
    busy(bridge, true);
    bridge.run_with_error(
        "Save game setting",
        move |launcher| match &target {
            EditTarget::Defaults => launcher.set_game_default(&job_key, &job_value),
            EditTarget::Instance(slug) => {
                launcher.set_instance_override(slug, &job_key, &job_value)
            }
        },
        move |window, result| {
            let (editor, bridge) = after;
            let state = window.global::<SettingsEditorState>();
            state.set_status(
                match &result {
                    Ok(()) => format!("{shown} saved"),
                    // The error dialog already says what went wrong; the rows are read again so
                    // the control goes back to the value that is really stored.
                    Err(_) => format!("{shown} was refused"),
                }
                .into(),
            );
            editor.reload(&bridge);
        },
    );
}

/// Drops one key from the edited layer, then reads every row back.
fn reset(editor: &Editor, bridge: &Bridge, key: &str) {
    let target = editor.target();
    let job_key = key.to_string();
    let shown = key.to_string();
    let after = (editor.clone(), bridge.clone());
    busy(bridge, true);
    bridge.run_with_error(
        "Reset game setting",
        move |launcher| match &target {
            EditTarget::Defaults => launcher.unset_game_default(&job_key).map(|_| ()),
            EditTarget::Instance(slug) => {
                launcher.unset_instance_override(slug, &job_key).map(|_| ())
            }
        },
        move |window, result| {
            let (editor, bridge) = after;
            let state = window.global::<SettingsEditorState>();
            if result.is_ok() {
                state.set_status(format!("{shown} reset").into());
            }
            editor.reload(&bridge);
        },
    );
}

/// Sets the flag every control in the editor is disabled by.
fn busy(bridge: &Bridge, busy: bool) {
    if let Some(window) = bridge.weak().upgrade() {
        window.global::<SettingsEditorState>().set_busy(busy);
    }
}

/// Puts one line under the editor.
fn status(bridge: &Bridge, text: &str) {
    if let Some(window) = bridge.weak().upgrade() {
        window
            .global::<SettingsEditorState>()
            .set_status(text.into());
    }
}
