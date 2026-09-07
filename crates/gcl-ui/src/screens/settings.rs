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
use crate::models::setting_rows;
use crate::{AccountsState, AppWindow, SettingRow, SettingsState};

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
    /// Whether a CurseForge API key is configured, from the file or the environment.
    pub curseforge_key_set: bool,
    /// Whether a Microsoft client id is configured.
    pub msa_client_id_set: bool,
    /// `options.txt` keys preseeded into every new instance, in key order.
    pub game_defaults: Vec<(String, String)>,
}

/// Binds the `SettingsState` global to the launcher and reads the config once.
pub fn wire(window: &AppWindow, bridge: &Bridge) {
    let state = window.global::<SettingsState>();

    {
        let bridge = bridge.clone();
        state.on_open(move || load(&bridge));
    }

    {
        let bridge = bridge.clone();
        state.on_save_root(move |path| save_root(&bridge, path.as_str()));
    }

    {
        let bridge = bridge.clone();
        state.on_save_downloads(move |count| save_downloads(&bridge, count));
    }

    {
        let bridge = bridge.clone();
        state.on_save_jvm(move |min, max, java_path| {
            save_jvm(&bridge, min.as_str(), max.as_str(), java_path.as_str());
        });
    }

    {
        let bridge = bridge.clone();
        state.on_save_curseforge_key(move |key| save_key(&bridge, Key::CurseForge, key.as_str()));
    }

    {
        let bridge = bridge.clone();
        state.on_save_msa_client_id(move |id| save_key(&bridge, Key::MsaClientId, id.as_str()));
    }

    {
        let bridge = bridge.clone();
        state.on_default_set(move |key, value| default_set(&bridge, key.as_str(), value.as_str()));
    }

    {
        let bridge = bridge.clone();
        state.on_default_unset(move |key| default_unset(&bridge, key.as_str()));
    }

    {
        let bridge = bridge.clone();
        state.on_verify_sources(move || verify_sources(&bridge));
    }

    load(bridge);
}

/// Reads the config and the root in use, then fills the screen.
fn load(bridge: &Bridge) {
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
    state.set_curseforge_key_set(view.curseforge_key_set);
    state.set_msa_client_id_set(view.msa_client_id_set);
    let rows: Vec<SettingRow> = setting_rows(&view.game_defaults.iter().cloned().collect());
    state.set_game_defaults(ModelRc::new(VecModel::from(rows)));
    // Saving a client id here is what turns Microsoft sign-in on, so the other screen's
    // button is told at the same time rather than waiting for its own reload.
    window
        .global::<AccountsState>()
        .set_msa_available(view.msa_client_id_set);
}

/// Points the launcher at another app root. Nothing is moved.
fn save_root(bridge: &Bridge, path: &str) {
    let path = path.trim().to_string();
    if path.is_empty() {
        return;
    }
    let bridge_after = bridge.clone();
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
                    .set_new_root(SharedString::new());
                window
                    .global::<SettingsState>()
                    .set_status(root_change_status(old).into());
            }
            done(window, &bridge_after, result.is_ok());
        },
    );
}

/// Changes how many downloads run at once.
fn save_downloads(bridge: &Bridge, count: i32) {
    let count = count.clamp(MIN_PARALLEL, MAX_PARALLEL) as usize;
    let bridge_after = bridge.clone();
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
            done(window, &bridge_after, result.is_ok());
        },
    );
}

/// Saves the default heap bounds and the java path, as typed.
fn save_jvm(bridge: &Bridge, min: &str, max: &str, java_path: &str) {
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
    let bridge_after = bridge.clone();
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
            done(window, &bridge_after, result.is_ok());
        },
    );
}

/// Which secret a save is about.
#[derive(Clone, Copy)]
enum Key {
    /// The CurseForge API key.
    CurseForge,
    /// The Microsoft client id.
    MsaClientId,
}

impl Key {
    /// What the status line calls this key.
    fn label(self) -> &'static str {
        match self {
            Key::CurseForge => "CurseForge API key",
            Key::MsaClientId => "Microsoft client id",
        }
    }
}

/// Saves or clears one key. The value is never logged, shown, or read back.
fn save_key(bridge: &Bridge, key: Key, value: &str) {
    let value = value.trim().to_string();
    let cleared = value.is_empty();
    let stored = (!cleared).then_some(value);
    if let Some(window) = bridge.weak().upgrade() {
        // The field is emptied before the save, so the typed value is on screen no longer
        // than it takes to press the button.
        let state = window.global::<SettingsState>();
        match key {
            Key::CurseForge => state.set_curseforge_key_input(SharedString::new()),
            Key::MsaClientId => state.set_msa_client_id_input(SharedString::new()),
        }
    }
    let bridge_after = bridge.clone();
    busy(bridge, true);
    bridge.run_with_error(
        "Save key",
        move |launcher| {
            launcher.update_config(|config| match key {
                Key::CurseForge => config.keys.curseforge_api_key = stored,
                Key::MsaClientId => config.keys.msa_client_id = stored,
            })
        },
        move |window, result| {
            if result.is_ok() {
                window
                    .global::<SettingsState>()
                    .set_status(key_status(key.label(), cleared).into());
            }
            done(window, &bridge_after, result.is_ok());
        },
    );
}

/// Adds or replaces one `options.txt` default.
fn default_set(bridge: &Bridge, key: &str, value: &str) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let (key, value) = (key.trim().to_string(), value.to_string());
    // A key with a `:` in it, or a line break in either half, would be read back as
    // something else, so it is refused here rather than written out.
    if let Err(err) = gcl_core::settings::validate_key(&key)
        .and_then(|()| gcl_core::settings::validate_value(&value))
    {
        window
            .global::<SettingsState>()
            .set_status(error_chain(&err).into());
        return;
    }
    let shown = key.clone();
    let bridge_after = bridge.clone();
    busy(bridge, true);
    bridge.run_with_error(
        "Save game default",
        move |launcher| {
            launcher.update_config(|config| {
                config.game_defaults.insert(key.clone(), value.clone());
            })
        },
        move |window, result| {
            if result.is_ok() {
                window
                    .global::<SettingsState>()
                    .set_status(format!("default {shown} saved").into());
            }
            done(window, &bridge_after, result.is_ok());
        },
    );
}

/// Drops one `options.txt` default.
fn default_unset(bridge: &Bridge, key: &str) {
    let key = key.to_string();
    let shown = key.clone();
    let bridge_after = bridge.clone();
    busy(bridge, true);
    bridge.run_with_error(
        "Remove game default",
        move |launcher| {
            launcher.update_config(|config| {
                config.game_defaults.remove(&key);
            })
        },
        move |window, result| {
            if result.is_ok() {
                window
                    .global::<SettingsState>()
                    .set_status(format!("default {shown} removed").into());
            }
            done(window, &bridge_after, result.is_ok());
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
fn done(window: &AppWindow, bridge: &Bridge, saved: bool) {
    window.global::<SettingsState>().set_busy(false);
    if saved {
        load(bridge);
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
/// The two key fields are booleans on purpose: the screen shows `<set>` or `<unset>` and can
/// never show, log, or send back a saved secret. Both read the environment first, the same
/// way every other caller does, so a key set outside the file still shows as set.
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
        curseforge_key_set: config.curseforge_api_key().is_some(),
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
