//! Wires the instances screen: list, filter, create, delete, and launch.
//!
//! Every call into `gcl-core` runs off the UI thread, through [`Bridge`] or through the one
//! thread a launch owns. The screen itself is pure layout; this module owns the `InstancesState`
//! global that feeds it.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use gcl_core::instances::model::Loader;
use gcl_core::launcher::LaunchOutcome;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use crate::bridge::{Bridge, show_error};
use crate::models::{instance_row, loader_version_row, version_row};
use crate::{App, AppWindow, InstanceRow, InstancesState, LogLine, Screen};

/// The set of instance slugs whose game is still running.
type Running = Arc<Mutex<HashSet<String>>>;

/// Binds the `InstancesState` global to the launcher and loads the first list.
pub fn wire(window: &AppWindow, bridge: &Bridge) {
    let state = window.global::<InstancesState>();
    let running: Running = Arc::new(Mutex::new(HashSet::new()));

    {
        let bridge = bridge.clone();
        let running = Arc::clone(&running);
        state.on_refresh(move || load(&bridge, &running));
    }

    {
        let weak = bridge.weak().clone();
        state.on_filter_changed(move |_| {
            if let Some(window) = weak.upgrade() {
                apply_filter(&window);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_create(move || open_create_form(&bridge));
    }

    {
        let weak = bridge.weak().clone();
        state.on_snapshots_changed(move |_| {
            if let Some(window) = weak.upgrade() {
                apply_version_filter(&window);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_load_loader_versions(move |mc, loader| {
            load_loader_versions(&bridge, mc.as_str(), loader.as_str());
        });
    }

    {
        let bridge = bridge.clone();
        let running = Arc::clone(&running);
        state.on_create_confirm(move |name, mc, loader, loader_version| {
            create_instance(
                &bridge,
                &running,
                name.as_str(),
                mc.as_str(),
                loader.as_str(),
                loader_version.as_str(),
            );
        });
    }

    {
        let weak = bridge.weak().clone();
        state.on_create_cancel(move || {
            if let Some(window) = weak.upgrade() {
                window.global::<InstancesState>().set_create_open(false);
            }
        });
    }

    {
        let weak = bridge.weak().clone();
        state.on_delete(move |slug| {
            if let Some(window) = weak.upgrade() {
                ask_delete(&window, slug.as_str());
            }
        });
    }

    {
        let bridge = bridge.clone();
        let running = Arc::clone(&running);
        state.on_confirm_yes(move || confirm_delete(&bridge, &running));
    }

    {
        let weak = bridge.weak().clone();
        state.on_confirm_no(move || {
            if let Some(window) = weak.upgrade() {
                let state = window.global::<InstancesState>();
                state.set_confirm_open(false);
                state.set_confirm_slug(SharedString::new());
            }
        });
    }

    {
        let bridge = bridge.clone();
        let running = Arc::clone(&running);
        state.on_launch(move |slug| launch(&bridge, &running, slug.as_str()));
    }

    load(bridge, &running);
}

/// Reads every instance and its installed flag, then rebuilds the list.
fn load(bridge: &Bridge, running: &Running) {
    if let Some(window) = bridge.weak().upgrade() {
        window.global::<InstancesState>().set_loading(true);
    }
    let running = Arc::clone(running);
    bridge.run(
        "Load instances",
        |launcher| {
            let instances = launcher.instances().list()?;
            // `installed_version_id` is `Some` only once the version JSON a launch resolves
            // is cached, which is exactly what "installed" means in the list.
            let rows = instances
                .into_iter()
                .map(|instance| {
                    let installed = launcher
                        .instance_summary(&instance.slug)
                        .map(|summary| summary.installed_version_id.is_some())
                        .unwrap_or(false);
                    (instance, installed)
                })
                .collect::<Vec<_>>();
            Ok(rows)
        },
        move |window, instances| {
            let live = snapshot(&running);
            let rows: Vec<InstanceRow> = instances
                .iter()
                .map(|(instance, installed)| {
                    instance_row(instance, *installed, live.contains(&instance.slug))
                })
                .collect();
            let state = window.global::<InstancesState>();
            state.set_loading(false);
            state.set_all_rows(ModelRc::new(VecModel::from(rows)));
            apply_filter(window);
        },
    );
}

/// Re-derives the visible rows from the unfiltered ones and the current filter.
fn apply_filter(window: &AppWindow) {
    let state = window.global::<InstancesState>();
    let all: Vec<InstanceRow> = state.get_all_rows().iter().collect();
    let rows = filter_rows(&all, state.get_filter().as_str());
    state.set_rows(ModelRc::new(VecModel::from(rows)));
}

/// Keeps the rows whose name or slug contains `filter`, ignoring case. Empty filter keeps all.
pub fn filter_rows(rows: &[InstanceRow], filter: &str) -> Vec<InstanceRow> {
    let needle = filter.trim().to_lowercase();
    if needle.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|row| {
            row.name.to_lowercase().contains(&needle) || row.slug.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect()
}

/// Whether the new-instance form names an instance that can be created.
///
/// A loader other than "none" needs a build chosen as well as a name and a version.
pub fn can_create(
    name: &str,
    version_set: bool,
    loader_index: i32,
    loader_version_set: bool,
) -> bool {
    !name.trim().is_empty() && version_set && (loader_index == 0 || loader_version_set)
}

/// The loader for a slug as [`Loader`]'s `Display` writes it.
pub fn loader_from_slug(slug: &str) -> Loader {
    match slug {
        "fabric" => Loader::Fabric,
        "quilt" => Loader::Quilt,
        "forge" => Loader::Forge,
        "neoforge" => Loader::NeoForge,
        _ => Loader::None,
    }
}

/// The label the loader-build ComboBox shows for one build.
pub fn loader_version_label(version: &str, stable: bool, recommended: bool) -> String {
    if recommended {
        format!("{version} (recommended)")
    } else if stable {
        version.to_string()
    } else {
        format!("{version} (unstable)")
    }
}

/// Fetches the Mojang manifest, fills the form, and opens it.
fn open_create_form(bridge: &Bridge) {
    bridge.run(
        "Load Minecraft versions",
        |launcher| launcher.list_versions(),
        |window, manifest| {
            let all: Vec<_> = manifest.versions.iter().map(version_row).collect();
            let state = window.global::<InstancesState>();
            state.set_all_versions(ModelRc::new(VecModel::from(all)));
            state.set_create_name(SharedString::new());
            state.set_loader_index(0);
            state.set_loader_versions(ModelRc::new(VecModel::from(Vec::new())));
            state.set_loader_version_labels(ModelRc::new(VecModel::from(Vec::new())));
            state.set_loader_version_index(-1);
            apply_version_filter(window);
            state.set_create_open(true);
        },
    );
}

/// Re-derives the offered versions from the snapshots checkbox and selects the newest one.
fn apply_version_filter(window: &AppWindow) {
    let state = window.global::<InstancesState>();
    let snapshots = state.get_show_snapshots();
    let kept: Vec<_> = state
        .get_all_versions()
        .iter()
        .filter(|row| snapshots || row.kind.as_str() == "release")
        .collect();
    let labels: Vec<SharedString> = kept.iter().map(|row| row.id.clone()).collect();
    let index = if kept.is_empty() { -1 } else { 0 };
    state.set_versions(ModelRc::new(VecModel::from(kept)));
    state.set_version_labels(ModelRc::new(VecModel::from(labels)));
    state.set_version_index(index);
}

/// Fetches the builds of one loader for one Minecraft version and preselects the recommended.
fn load_loader_versions(bridge: &Bridge, mc: &str, loader: &str) {
    let loader = loader_from_slug(loader);
    if loader == Loader::None || mc.is_empty() {
        return;
    }
    if let Some(window) = bridge.weak().upgrade() {
        window
            .global::<InstancesState>()
            .set_loading_loader_versions(true);
    }
    let mc = mc.to_string();
    bridge.run(
        "Load loader versions",
        // The error is carried, not returned, so the spinner is cleared either way.
        move |launcher| Ok(launcher.list_loader_versions(loader, &mc)),
        |window, result| {
            let state = window.global::<InstancesState>();
            state.set_loading_loader_versions(false);
            let versions = match result {
                Ok(versions) => versions,
                Err(err) => {
                    show_error(window, "Load loader versions", &err);
                    return;
                }
            };
            let labels: Vec<SharedString> = versions
                .iter()
                .map(|v| loader_version_label(&v.version, v.stable, v.recommended).into())
                .collect();
            let rows: Vec<_> = versions.iter().map(loader_version_row).collect();
            let index = match rows.iter().position(|row| row.recommended) {
                Some(at) => at as i32,
                None if rows.is_empty() => -1,
                None => 0,
            };
            state.set_loader_versions(ModelRc::new(VecModel::from(rows)));
            state.set_loader_version_labels(ModelRc::new(VecModel::from(labels)));
            state.set_loader_version_index(index);
        },
    );
}

/// Creates the instance, installs its loader, and reloads the list.
fn create_instance(
    bridge: &Bridge,
    running: &Running,
    name: &str,
    mc: &str,
    loader: &str,
    loader_version: &str,
) {
    let loader = loader_from_slug(loader);
    let index = if loader == Loader::None { 0 } else { 1 };
    if !can_create(name, !mc.is_empty(), index, !loader_version.is_empty()) {
        return;
    }
    if let Some(window) = bridge.weak().upgrade() {
        window.global::<InstancesState>().set_create_open(false);
    }

    let (name, mc) = (name.to_string(), mc.to_string());
    let loader_version = (!loader_version.is_empty()).then(|| loader_version.to_string());
    let after = bridge.clone();
    let running = Arc::clone(running);
    bridge.run(
        "Create instance",
        move |launcher| {
            // The guard is dropped at the end of the statement: never hold one across a call.
            let defaults = launcher.config().game_defaults.clone();
            let instance =
                launcher
                    .instances()
                    .create(&name, &mc, loader, loader_version, &defaults)?;
            // Install now so the row is ready to launch. Progress reaches the task strip
            // through the event forwarder.
            if loader != Loader::None {
                launcher.install_loader(&instance.slug)?;
            }
            Ok(())
        },
        move |_window, ()| load(&after, &running),
    );
}

/// Opens the yes/no modal for a delete.
fn ask_delete(window: &AppWindow, slug: &str) {
    let state = window.global::<InstancesState>();
    let name = state
        .get_all_rows()
        .iter()
        .find(|row| row.slug.as_str() == slug)
        .map(|row| row.name.to_string())
        .unwrap_or_else(|| slug.to_string());
    state.set_confirm_slug(slug.into());
    state.set_confirm_title("Delete instance?".into());
    state.set_confirm_text(
        format!("`{name}` and everything in its folder are removed. This cannot be undone.").into(),
    );
    state.set_confirm_label("Delete".into());
    state.set_confirm_danger(true);
    state.set_confirm_open(true);
}

/// Deletes the instance the modal is asking about, then reloads the list.
fn confirm_delete(bridge: &Bridge, running: &Running) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<InstancesState>();
    let slug = state.get_confirm_slug().to_string();
    state.set_confirm_open(false);
    state.set_confirm_slug(SharedString::new());
    if slug.is_empty() {
        return;
    }

    let after = bridge.clone();
    let running = Arc::clone(running);
    bridge.run(
        "Delete instance",
        move |launcher| Ok(launcher.instances().delete(&slug)?),
        move |_window, ()| load(&after, &running),
    );
}

/// Shows the instance's detail screen and starts its game.
///
/// The launch runs on a thread of its own: `launch_instance_async` returns once the process
/// is up, and the same thread then blocks on the game and posts the outcome back.
fn launch(bridge: &Bridge, running: &Running, slug: &str) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let app = window.global::<App>();
    app.set_current_slug(slug.into());
    app.set_screen(Screen::Instance);

    // A slug already in the set has a game up; the button is disabled, but a stale row could
    // still send this.
    let fresh = match running.lock() {
        Ok(mut set) => set.insert(slug.to_string()),
        Err(_) => false,
    };
    if !fresh {
        return;
    }
    mark_running(&window, running);

    let slug = slug.to_string();
    let launcher = Arc::clone(bridge.launcher());
    let weak = bridge.weak().clone();
    let bridge = bridge.clone();
    let running = Arc::clone(running);
    std::thread::spawn(move || {
        let started = launcher.launch_instance_async(&slug, None, None);
        let outcome = match started {
            Ok(launch) => launch.wait_blocking(&launcher),
            Err(err) => Err(err),
        };
        finish_launch(&weak, &bridge, &running, &slug, outcome);
    });
}

/// Clears the running flag and reports what the game did. Runs on the launch thread.
fn finish_launch(
    weak: &Weak<AppWindow>,
    bridge: &Bridge,
    running: &Running,
    slug: &str,
    outcome: Result<LaunchOutcome, gcl_core::Error>,
) {
    if let Ok(mut set) = running.lock() {
        set.remove(slug);
    }
    let bridge = bridge.clone();
    let running = Arc::clone(running);
    let _ = weak.upgrade_in_event_loop(move |window| {
        match outcome {
            Ok(LaunchOutcome::Exited { code, hint, .. }) if code != 0 => {
                let hint = hint.unwrap_or_else(|| "see the instance log".to_string());
                warn(
                    &window,
                    &format!("Minecraft exited with code {code}: {hint}"),
                );
            }
            Ok(_) => {}
            Err(err) if is_no_account(&err) => {
                let app = window.global::<App>();
                app.set_error_title("No account".into());
                app.set_error_text(
                    "Launching needs an account. Add one on the Accounts screen, or set an \
                     offline user name there."
                        .into(),
                );
                app.set_error_open(true);
            }
            Err(err) => show_error(&window, "Launch", &err),
        }
        // The last-launched stamp and the running flag both changed.
        load(&bridge, &running);
    });
}

/// Whether a launch failed only because no account is selected.
fn is_no_account(err: &gcl_core::Error) -> bool {
    matches!(err, gcl_core::Error::Auth(gcl_core::auth::Error::NoAccount))
}

/// Rewrites the `running` flag on every row from the live set.
fn mark_running(window: &AppWindow, running: &Running) {
    let live = snapshot(running);
    let state = window.global::<InstancesState>();
    let rows: Vec<InstanceRow> = state
        .get_all_rows()
        .iter()
        .map(|mut row| {
            row.running = live.contains(row.slug.as_str());
            row
        })
        .collect();
    state.set_all_rows(ModelRc::new(VecModel::from(rows)));
    apply_filter(window);
}

/// A copy of the running set. An empty set stands in for a poisoned lock.
fn snapshot(running: &Running) -> HashSet<String> {
    running.lock().map(|set| set.clone()).unwrap_or_default()
}

/// Appends a warning to the app log, which the window shows as a toast.
fn warn(window: &AppWindow, text: &str) {
    let app = window.global::<App>();
    let mut log: Vec<LogLine> = app.get_app_log().iter().collect();
    log.push(LogLine {
        level: "warning".into(),
        text: text.into(),
    });
    app.set_app_log(ModelRc::new(VecModel::from(log)));
}

#[cfg(test)]
mod tests;
