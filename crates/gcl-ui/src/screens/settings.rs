//! Wires the settings screen: the app root, downloads, JVM defaults, API keys, the game
//! defaults, and the source check.
//!
//! Every call into `gcl-core` runs off the UI thread through [`Bridge`]. The screen itself is
//! pure layout; this module owns the `SettingsState` global that feeds it.
//!
//! Nothing here talks to the network except `verify_sources`, and only when the user asks
//! for it. A saved API key is never read back into the screen: the config view carries
//! whether a key is set, never the key.

use std::path::PathBuf;

use gcl_core::config::Config;
use gcl_core::sources::{ContentKind, SearchQuery};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bridge::{Bridge, error_chain};
use crate::screens::settings_editor::{EditTarget, Editor};
use crate::{AccountsState, AppWindow, SettingsState};

/// The project the source check searches for. One hit is enough to prove the parser.
const CHECK_PROJECT: &str = "sodium";

/// Lowest download concurrency the screen offers. Matches the SpinBox.
const MIN_PARALLEL: i32 = 1;

/// Highest download concurrency the screen offers. Matches the SpinBox.
const MAX_PARALLEL: i32 = 32;

/// The config as the screen shows it: no key values, only whether a key is there.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigView {
    /// The root written in the config file, empty when the platform default is used.
    pub root: String,
    /// Maximum number of concurrent downloads.
    pub parallel: i32,
    /// Default minimum heap, in MiB.
    pub jvm_min: i32,
    /// Default maximum heap, in MiB.
    pub jvm_max: i32,
    /// Path to a specific `java`, empty when the launcher may choose one.
    pub java_path: String,
    /// Whether this build carries a CurseForge API key, from the file or the environment.
    pub curseforge_enabled: bool,
    /// Whether a Microsoft client id is configured.
    pub msa_client_id_set: bool,
    /// `options.txt` keys preseeded into every new instance, in key order.
    pub game_defaults: Vec<(String, String)>,
}

/// Binds the `SettingsState` global to the launcher and reads the config once.
///
/// `editor` is the shared typed settings editor: this screen opens it on the launcher
/// defaults, and every save here reads it again, because a raw key added under Advanced is
/// one more row it has to show.
pub fn wire(window: &AppWindow, bridge: &Bridge, editor: &Editor) {
    let state = window.global::<SettingsState>();

    {
        let (bridge, editor) = (bridge.clone(), editor.clone());
        state.on_open(move || open(&bridge, &editor));
    }

    {
        let (bridge, editor) = (bridge.clone(), editor.clone());
        state.on_save_root(move |path| save_root(&bridge, &editor, path.as_str()));
    }

    {
        let (bridge, editor) = (bridge.clone(), editor.clone());
        state.on_save_downloads(move |count| save_downloads(&bridge, &editor, count));
    }

    {
        let (bridge, editor) = (bridge.clone(), editor.clone());
        state.on_save_jvm(move |min, max, java_path| {
            save_jvm(
                &bridge,
                &editor,
                min.as_str(),
                max.as_str(),
                java_path.as_str(),
            );
        });
    }

    {
        let (bridge, editor) = (bridge.clone(), editor.clone());
        state.on_save_msa_client_id(move |id| {
            save_key(&bridge, &editor, Key::MsaClientId, id.as_str());
        });
    }

    {
        let (bridge, editor) = (bridge.clone(), editor.clone());
        state.on_choose_root(move || choose_root(&bridge, &editor));
    }

    {
        let bridge = bridge.clone();
        state.on_verify_sources(move || verify_sources(&bridge));
    }

    open(bridge, editor);
}

/// The screen's entry point: points the shared editor at the launcher defaults, then loads.
///
/// This is the only place that retargets the editor. One editor serves this screen and the
/// instance Settings tab, so a later refresh must not pull it back here: a job that finishes
/// after the user has navigated away would otherwise show instance rows the wrong layer, or
/// write a preseed value the user meant as an override.
fn open(bridge: &Bridge, editor: &Editor) {
    editor.open(bridge, EditTarget::Defaults);
    read_config(bridge);
}

/// Reads the config and the root in use again, and re-reads the editor's rows while the
/// editor is still on this screen's layer.
fn load(bridge: &Bridge, editor: &Editor) {
    if editor.target() == EditTarget::Defaults {
        editor.reload(bridge);
    }
    read_config(bridge);
}

/// Reads the config and the root in use, then fills the screen.
fn read_config(bridge: &Bridge) {
    bridge.run_with_error(
        "Read settings",
        |launcher| {
            // The config guard is dropped at the end of this statement, before the root is
            // read: never hold one across another launcher call.
            let config = launcher.config().clone();
            let root = launcher.root().path().display().to_string();
            Ok((config_view(&config), root))
        },
        |window, result| {
            let state = window.global::<SettingsState>();
            state.set_busy(false);
            let Ok((view, root)) = result else {
                state.set_status("Could not read the config".into());
                return;
            };
            state.set_root_path(root.into());
            apply_view(window, &view);
        },
    );
}

/// Copies a config view into the screen, and into the accounts screen's sign-in button.
fn apply_view(window: &AppWindow, view: &ConfigView) {
    let state = window.global::<SettingsState>();
    state.set_parallel(view.parallel);
    state.set_jvm_min(view.jvm_min);
    state.set_jvm_max(view.jvm_max);
    state.set_java_path(view.java_path.as_str().into());
    state.set_curseforge_enabled(view.curseforge_enabled);
    state.set_msa_client_id_set(view.msa_client_id_set);
    // Saving a client id here is what turns Microsoft sign-in on, so the other screen's
    // button, and the hint under it, are told at the same time rather than waiting for its
    // own reload.
    let accounts = window.global::<AccountsState>();
    accounts.set_msa_available(view.msa_client_id_set);
    accounts.set_msa_hint(if view.msa_client_id_set {
        "".into()
    } else {
        crate::screens::accounts::NO_CLIENT_ID.into()
    });
}

/// Opens the OS folder picker for a new app root, starting at the root in use now.
///
/// The dialog itself runs in the job closure, off the UI thread, the same as any other
/// `Bridge::run` call; `pick_folder` blocks the thread it runs on until the user answers. A
/// picked path goes through [`save_root`], the one writer of the config's root. A cancelled
/// picker, or a platform with no portal to answer it, comes back `None` and nothing happens:
/// this is not an error, so no dialog opens for it.
fn choose_root(bridge: &Bridge, editor: &Editor) {
    let after = (bridge.clone(), editor.clone());
    bridge.run(
        "Choose app root",
        |launcher| {
            let current = launcher.root().path().to_path_buf();
            let picked = rfd::FileDialog::new()
                .set_title("Choose app root")
                .set_directory(current)
                .pick_folder();
            Ok(picked)
        },
        move |_window, picked| {
            let Some(path) = picked else {
                return;
            };
            save_root(&after.0, &after.1, path.display().to_string().as_str());
        },
    );
}

/// Points the launcher at another app root. Nothing is moved.
fn save_root(bridge: &Bridge, editor: &Editor, path: &str) {
    let path = path.trim().to_string();
    if path.is_empty() {
        return;
    }
    let after = (bridge.clone(), editor.clone());
    busy(bridge, true);
    bridge.run_with_error(
        "Change app root",
        move |launcher| {
            let old = launcher.root().path().display().to_string();
            launcher.update_config(|config| config.root = Some(PathBuf::from(path)))?;
            Ok(old)
        },
        move |window, result| {
            if let Ok(old) = &result {
                window
                    .global::<SettingsState>()
                    .set_status(root_change_status(old).into());
            }
            done(window, &after.0, &after.1, result.is_ok());
        },
    );
}

/// Changes how many downloads run at once.
fn save_downloads(bridge: &Bridge, editor: &Editor, count: i32) {
    let count = count.clamp(MIN_PARALLEL, MAX_PARALLEL) as usize;
    let after = (bridge.clone(), editor.clone());
    busy(bridge, true);
    bridge.run_with_error(
        "Save downloads",
        move |launcher| launcher.update_config(|config| config.parallel_downloads = count),
        move |window, result| {
            if result.is_ok() {
                window
                    .global::<SettingsState>()
                    .set_status(format!("{count} parallel download(s)").into());
            }
            done(window, &after.0, &after.1, result.is_ok());
        },
    );
}

/// Saves the default heap bounds and the java path, as typed.
fn save_jvm(bridge: &Bridge, editor: &Editor, min: &str, max: &str, java_path: &str) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<SettingsState>();
    let (Some(min_mib), Some(max_mib)) = (parse_mib(min), parse_mib(max)) else {
        state.set_status("Heap sizes are a number of MiB, e.g. 4096 or 8G".into());
        return;
    };
    if min_mib > max_mib {
        state.set_status(format!("min heap {min_mib} MiB is above max heap {max_mib} MiB").into());
        return;
    }
    let java_path = java_path.trim().to_string();
    let after = (bridge.clone(), editor.clone());
    busy(bridge, true);
    bridge.run_with_error(
        "Save Java defaults",
        move |launcher| {
            launcher.update_config(|config| {
                config.jvm.min_mib = min_mib;
                config.jvm.max_mib = max_mib;
                config.jvm.java_path = (!java_path.is_empty()).then(|| PathBuf::from(&java_path));
            })
        },
        move |window, result| {
            if result.is_ok() {
                window
                    .global::<SettingsState>()
                    .set_status(format!("heap {min_mib}-{max_mib} MiB saved").into());
            }
            done(window, &after.0, &after.1, result.is_ok());
        },
    );
}

/// Which secret a save is about.
#[derive(Clone, Copy)]
enum Key {
    /// The Microsoft client id.
    MsaClientId,
}

impl Key {
    /// What the status line calls this key.
    fn label(self) -> &'static str {
        match self {
            Key::MsaClientId => "Microsoft client id",
        }
    }
}

/// Saves or clears one key. The value is never logged, shown, or read back.
fn save_key(bridge: &Bridge, editor: &Editor, key: Key, value: &str) {
    let value = value.trim().to_string();
    let cleared = value.is_empty();
    let stored = (!cleared).then_some(value);
    if let Some(window) = bridge.weak().upgrade() {
        // The field is emptied before the save, so the typed value is on screen no longer
        // than it takes to press the button.
        let state = window.global::<SettingsState>();
        match key {
            Key::MsaClientId => state.set_msa_client_id_input(SharedString::new()),
        }
    }
    let after = (bridge.clone(), editor.clone());
    busy(bridge, true);
    bridge.run_with_error(
        "Save key",
        move |launcher| {
            launcher.update_config(|config| match key {
                Key::MsaClientId => config.keys.msa_client_id = stored,
            })
        },
        move |window, result| {
            if result.is_ok() {
                window
                    .global::<SettingsState>()
                    .set_status(key_status(key.label(), cleared).into());
            }
            done(window, &after.0, &after.1, result.is_ok());
        },
    );
}

/// Searches every configured source and reads the Mojang manifest, then lists the verdicts.
///
/// This is the one thing on the screen that goes to the network, and only on this click.
/// Each check is the smallest live call that still runs a real response through our parsers.
fn verify_sources(bridge: &Bridge) {
    if let Some(window) = bridge.weak().upgrade() {
        let state = window.global::<SettingsState>();
        state.set_busy(true);
        state.set_verify_lines(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
        state.set_status("Checking the sources…".into());
    }
    bridge.run_with_error(
        "Verify sources",
        |launcher| {
            let query = SearchQuery {
                text: CHECK_PROJECT.to_string(),
                kind: Some(ContentKind::Mod),
                limit: 1,
                ..SearchQuery::default()
            };
            let mut lines: Vec<String> = launcher
                .sources()
                .iter()
                .map(|source| {
                    let id = source.id();
                    verify_line(&id.to_string(), &launcher.search(id, &query))
                })
                .collect();
            lines.push(verify_line("mojang", &launcher.list_versions()));
            Ok(lines)
        },
        |window, result| {
            let state = window.global::<SettingsState>();
            state.set_busy(false);
            let Ok(lines) = result else {
                state.set_status("The source check could not run".into());
                return;
            };
            state.set_status(verify_status(&lines).into());
            let rows: Vec<SharedString> = lines.iter().map(SharedString::from).collect();
            state.set_verify_lines(ModelRc::new(VecModel::from(rows)));
        },
    );
}

/// Clears `busy`, and re-reads the config when the save landed.
fn done(window: &AppWindow, bridge: &Bridge, editor: &Editor, saved: bool) {
    window.global::<SettingsState>().set_busy(false);
    if saved {
        load(bridge, editor);
    } else {
        // The error dialog is already up: `run_with_error` opened it.
        window
            .global::<SettingsState>()
            .set_status("That did not work".into());
    }
}

/// Sets or clears the flag every button on the screen is disabled by.
fn busy(bridge: &Bridge, busy: bool) {
    if let Some(window) = bridge.weak().upgrade() {
        window.global::<SettingsState>().set_busy(busy);
    }
}

/// Builds the screen's view of a config, with the key values left out.
///
/// `curseforge_enabled` reports whether this build carries a CurseForge key at all; there is
/// no field to save one from the screen. `msa_client_id_set` is a boolean on purpose: the
/// screen shows `<set>` or `<unset>` and can never show, log, or send back a saved secret. It
/// reads the environment first, the same way every other caller does, so a key set outside the
/// file still shows as set.
pub fn config_view(config: &Config) -> ConfigView {
    ConfigView {
        root: config
            .root
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        parallel: i32::try_from(config.parallel_downloads).unwrap_or(MAX_PARALLEL),
        jvm_min: i32::try_from(config.jvm.min_mib).unwrap_or(i32::MAX),
        jvm_max: i32::try_from(config.jvm.max_mib).unwrap_or(i32::MAX),
        java_path: config
            .jvm
            .java_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        curseforge_enabled: config.curseforge_api_key().is_some(),
        msa_client_id_set: config.msa_client_id().is_some(),
        game_defaults: config
            .game_defaults
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    }
}

/// Reads a heap size as MiB: a plain number, or one with an `M`, `MiB`, `G`, or `GiB` unit.
///
/// `None` for anything else, including zero and a negative number, so a typo cannot save a
/// heap the JVM would refuse.
pub fn parse_mib(text: &str) -> Option<u32> {
    let lower = text.trim().to_ascii_lowercase();
    let (number, factor) = match strip_unit(&lower, &["gib", "gb", "g"]) {
        Some(rest) => (rest, 1024u32),
        None => (
            strip_unit(&lower, &["mib", "mb", "m"]).unwrap_or(&lower),
            1u32,
        ),
    };
    let value: u32 = number.trim().parse().ok()?;
    if value == 0 {
        return None;
    }
    value.checked_mul(factor)
}

/// The text before the first of `units` it ends with.
fn strip_unit<'a>(text: &'a str, units: &[&str]) -> Option<&'a str> {
    units.iter().find_map(|unit| text.strip_suffix(unit))
}

/// One PASS or FAIL line for a check that has already run.
pub fn verify_line<T>(name: &str, result: &Result<T, gcl_core::Error>) -> String {
    match result {
        Ok(_) => format!("PASS {name}"),
        Err(err) => format!("FAIL {name}: {}", error_chain(err)),
    }
}

/// The status line for a finished source check.
pub fn verify_status(lines: &[String]) -> String {
    let failed = lines.iter().filter(|line| line.starts_with("FAIL")).count();
    match failed {
        0 => format!("{} check(s) passed", lines.len()),
        _ => format!("{failed} of {} check(s) failed", lines.len()),
    }
}

/// The status line after the app root was changed.
pub fn root_change_status(old_root: &str) -> String {
    format!("root will change on next start; existing data stays at {old_root}")
}

/// The status line after a key was saved or cleared.
pub fn key_status(label: &str, cleared: bool) -> String {
    match cleared {
        true => format!("{label} cleared"),
        false => format!("{label} saved"),
    }
}

#[cfg(test)]
mod tests;
