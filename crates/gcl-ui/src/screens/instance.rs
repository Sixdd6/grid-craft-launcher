//! Wires the instance detail screen: header, content, settings, JVM, and the game log.
//!
//! Every call into `gcl-core` runs off the UI thread, through [`Bridge`] or, for a launch,
//! through [`crate::launch_flow`], which both this screen and the instances list share. The
//! screen itself is pure layout; this module owns the `InstanceState` global that feeds it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gcl_core::content::{ManualDownload, UpdateCandidate};
use gcl_core::instances::model::{ContentEntry, InstanceJvm};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::bridge::Bridge;
use crate::launch_flow;
use crate::models::{content_row, pending_row, setting_rows};
use crate::state::RunState;
use crate::{
    App, AppWindow, BrowserState, ContentRow, InstanceState, PendingRow, Screen, SettingRow,
};

/// Smallest heap the JVM tab offers, in MiB. Matches the `SpinBox` bounds in the screen.
const HEAP_MIN_MIB: i32 = 512;

/// Largest heap the JVM tab offers, in MiB. Matches the `SpinBox` bounds in the screen.
const HEAP_MAX_MIB: i32 = 65536;

/// Heap shown for a maximum an instance does not set. The launch itself falls back to
/// `config.toml`, so this is only what the tab offers before the user saves anything.
const DEFAULT_MAX_MIB: i32 = 2048;

/// What the prompt is for when it is asking for a new instance name.
const PROMPT_RENAME: &str = "rename";

/// What one loaded instance put on the screen, so the callbacks can look things up again.
///
/// The candidates come from a "Check updates", the pending downloads from the last load. Both
/// are shared with the bridge jobs, which is why they are behind a mutex rather than kept in
/// the Slint models: only the ids reach the UI.
#[derive(Clone, Default)]
struct Shared {
    /// Newer versions the last check found, in the order `apply_updates` wants them.
    candidates: Arc<Mutex<Vec<UpdateCandidate>>>,
    /// The manual downloads the loaded instance still needs, by project id.
    pending: Arc<Mutex<Vec<ManualDownload>>>,
}

impl Shared {
    /// Replaces the update candidates.
    fn set_candidates(&self, list: Vec<UpdateCandidate>) {
        *self
            .candidates
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = list;
    }

    /// A copy of the update candidates.
    fn candidates(&self) -> Vec<UpdateCandidate> {
        self.candidates
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Replaces the pending manual downloads.
    fn set_pending(&self, list: Vec<ManualDownload>) {
        *self.pending.lock().unwrap_or_else(|err| err.into_inner()) = list;
    }

    /// The pending manual download with this project id, if the last load carried one.
    fn pending_for(&self, project_id: &str) -> Option<ManualDownload> {
        self.pending
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .find(|item| item.project_id == project_id)
            .cloned()
    }
}

/// Binds the `InstanceState` global to the launcher.
///
/// Nothing is loaded here: `app.slint` calls `load` when the shell navigates to this screen or
/// the shown slug changes.
pub fn wire(window: &AppWindow, bridge: &Bridge, run: &RunState) {
    let state = window.global::<InstanceState>();
    let shared = Shared::default();

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_load(move |slug| load(&bridge, &run, &shared, slug.as_str()));
    }

    {
        let weak = bridge.weak().clone();
        state.on_back(move || {
            if let Some(window) = weak.upgrade() {
                window.global::<App>().set_screen(Screen::Instances);
            }
        });
    }

    {
        let weak = bridge.weak().clone();
        state.on_set_tab(move |tab| {
            if let Some(window) = weak.upgrade() {
                window.global::<InstanceState>().set_tab(tab);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_stop(move || {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            status(&bridge, "Stopping…");
            // The launch thread is the one that reports the exit, so nothing is done here on
            // success: `launch_flow` rewrites the status and the running flag when the game
            // goes. `run_with_error` opens the shared error dialog on a failure and still
            // runs the closure, so the status line is rewritten either way.
            bridge.run_with_error(
                "Stop instance",
                move |launcher| launcher.stop_instance(&slug),
                |window, result| {
                    if let Err(err) = result {
                        tracing::warn!(error = %crate::bridge::error_chain(&err), "stop failed");
                        window
                            .global::<InstanceState>()
                            .set_status_text("Stop failed; see the error".into());
                    }
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_install(move || {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            status(&bridge, "Installing…");
            let after = (bridge.clone(), run.clone(), shared.clone());
            let job_slug = slug.clone();
            bridge.run(
                "Install instance",
                move |launcher| launcher.install_instance(&job_slug).map(|_| ()),
                move |_window, ()| {
                    let (bridge, run, shared) = after;
                    status(&bridge, "Installed");
                    load(&bridge, &run, &shared, &slug);
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_content_toggle(move |project_id, enabled| {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            let (job_slug, id) = (slug.clone(), project_id.to_string());
            let after = (bridge.clone(), run.clone(), shared.clone());
            bridge.run(
                "Change content",
                move |launcher| {
                    launcher
                        .set_content_enabled(&job_slug, &id, enabled)
                        .map(|_| ())
                },
                move |_window, ()| {
                    let (bridge, run, shared) = after;
                    load(&bridge, &run, &shared, &slug);
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_content_remove(move |project_id| {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            let (job_slug, id) = (slug.clone(), project_id.to_string());
            let after = (bridge.clone(), run.clone(), shared.clone());
            bridge.run(
                "Remove content",
                move |launcher| launcher.remove_content(&job_slug, &id),
                move |_window, ()| {
                    let (bridge, run, shared) = after;
                    status(&bridge, "Removed");
                    load(&bridge, &run, &shared, &slug);
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_check_updates(move || {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            status(&bridge, "Checking for updates…");
            let shared = shared.clone();
            bridge.run(
                "Check updates",
                move |launcher| launcher.check_updates(&slug),
                move |window, candidates| {
                    let state = window.global::<InstanceState>();
                    state.set_update_count(candidates.len() as i32);
                    state.set_status_text(
                        match candidates.len() {
                            0 => "Everything is up to date".to_string(),
                            n => format!("{n} update(s) available"),
                        }
                        .into(),
                    );
                    mark_updates(window, &candidates);
                    shared.set_candidates(candidates);
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_apply_updates(move || {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            let candidates = shared.candidates();
            if candidates.is_empty() {
                return;
            }
            status(&bridge, "Applying updates…");
            let after = (bridge.clone(), run.clone(), shared.clone());
            let job_slug = slug.clone();
            bridge.run(
                "Apply updates",
                move |launcher| launcher.apply_updates(&job_slug, &candidates).map(|_| ()),
                move |_window, ()| {
                    let (bridge, run, shared) = after;
                    shared.set_candidates(Vec::new());
                    status(&bridge, "Updates applied");
                    load(&bridge, &run, &shared, &slug);
                },
            );
        });
    }

    {
        let weak = bridge.weak().clone();
        state.on_add_content(move || {
            // `App.current_slug` is already this instance, which is what the browser adds to.
            // `fixed_target` says so: it drops the browser's target ComboBox and prefills the
            // search filters from this instance. The rail clears it again.
            if let Some(window) = weak.upgrade() {
                window.global::<BrowserState>().set_fixed_target(true);
                window.global::<App>().set_screen(Screen::Browser);
            }
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_import_manual(move |project_id, path| {
            import_manual(
                &bridge,
                &run,
                &shared,
                project_id.as_str(),
                Path::new(path.as_str()),
            );
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_override_set(move |key, value| {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            let (job_slug, key, value) = (slug.clone(), key.to_string(), value.to_string());
            let after = (bridge.clone(), run.clone(), shared.clone());
            bridge.run(
                "Set override",
                move |launcher| launcher.set_instance_override(&job_slug, &key, &value),
                move |_window, ()| {
                    let (bridge, run, shared) = after;
                    status(&bridge, "Override saved");
                    load(&bridge, &run, &shared, &slug);
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_override_unset(move |key| {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            let (job_slug, key) = (slug.clone(), key.to_string());
            let after = (bridge.clone(), run.clone(), shared.clone());
            bridge.run(
                "Unset override",
                move |launcher| {
                    launcher
                        .unset_instance_override(&job_slug, &key)
                        .map(|_| ())
                },
                move |_window, ()| {
                    let (bridge, run, shared) = after;
                    status(&bridge, "Override dropped");
                    load(&bridge, &run, &shared, &slug);
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_jvm_save(move |min, max, extra, java_path| {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            if !jvm_valid(min, max) {
                status(&bridge, "The minimum heap must not be above the maximum");
                return;
            }
            let jvm = instance_jvm(min, max, extra.as_str(), java_path.as_str());
            let after = (bridge.clone(), run.clone(), shared.clone());
            let job_slug = slug.clone();
            bridge.run(
                "Save JVM settings",
                move |launcher| launcher.set_instance_jvm(&job_slug, jvm),
                move |_window, ()| {
                    let (bridge, run, shared) = after;
                    status(&bridge, "JVM settings saved");
                    load(&bridge, &run, &shared, &slug);
                },
            );
        });
    }

    {
        let bridge = bridge.clone();
        state.on_open_log_folder(move || {
            // No shell open in the MVP: the path goes to the status line, ready to copy.
            let path = bridge.launcher().root().logs_dir();
            if let Some(window) = bridge.weak().upgrade() {
                let text: SharedString = format!("Logs are in {}", path.display()).into();
                window.global::<App>().set_status_text(text.clone());
                window.global::<InstanceState>().set_status_text(text);
            }
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        state.on_launch(move || {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            launch_flow::launch(&bridge, &run, slug, None);
        });
    }

    {
        let bridge = bridge.clone();
        state.on_rename(move || {
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            let state = window.global::<InstanceState>();
            state.set_prompt_mode(PROMPT_RENAME.into());
            state.set_prompt_title("Rename instance".into());
            state.set_prompt_label("Instance name".into());
            state.set_prompt_accept("Rename".into());
            // Prefilled with the name it has: a rename is usually an edit, not a retype.
            state.set_prompt_value(state.get_name());
            state.set_prompt_open(true);
        });
    }

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_prompt_ok(move |name| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            let state = window.global::<InstanceState>();
            state.set_prompt_open(false);
            let mode = state.get_prompt_mode().to_string();
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            if mode == PROMPT_RENAME {
                rename(&bridge, &run, &shared, &slug, &name);
            } else {
                launch_flow::launch(&bridge, &run, slug, Some(name));
            }
        });
    }

    {
        let weak = bridge.weak().clone();
        state.on_prompt_cancel(move || {
            if let Some(window) = weak.upgrade() {
                let state = window.global::<InstanceState>();
                state.set_prompt_open(false);
                let cancelled = if state.get_prompt_mode() == PROMPT_RENAME {
                    "Rename cancelled"
                } else {
                    "Launch cancelled: no account"
                };
                state.set_status_text(cancelled.into());
            }
        });
    }
}

/// Renames the instance, then reloads its header and the instances list.
///
/// The slug never changes, so nothing else on the screen has to be told: the name in the
/// header and the row in the list are the only two places it shows.
fn rename(bridge: &Bridge, run: &RunState, shared: &Shared, slug: &str, name: &str) {
    let (job_slug, new_name) = (slug.to_string(), name.to_string());
    let slug = slug.to_string();
    let after = (bridge.clone(), run.clone(), shared.clone());
    status(bridge, "Renaming\u{2026}");
    bridge.run(
        "Rename instance",
        move |launcher| Ok(launcher.instances().rename(&job_slug, &new_name)?),
        move |window, _instance| {
            let (bridge, run, shared) = after;
            status(&bridge, "Renamed");
            load(&bridge, &run, &shared, &slug);
            window.global::<crate::InstancesState>().invoke_refresh();
        },
    );
}

/// Everything one load reads off disk, as the UI-thread half of the load then shows it.
struct Loaded {
    /// The instance's own summary.
    summary: gcl_core::launcher::InstanceSummary,
    /// What `instance.toml` records as installed.
    content: Vec<ContentEntry>,
    /// The pairs `options.txt` holds right now.
    options: Vec<(String, String)>,
}

/// Reads the instance and fills every property the screen shows.
fn load(bridge: &Bridge, run: &RunState, shared: &Shared, slug: &str) {
    let (job_slug, run, shared) = (slug.to_string(), run.clone(), shared.clone());
    let slug = slug.to_string();
    bridge.run(
        "Load instance",
        move |launcher| {
            Ok(Loaded {
                summary: launcher.instance_summary(&job_slug)?,
                content: launcher.list_content(&job_slug)?,
                options: launcher.instance_options(&job_slug)?,
            })
        },
        move |window, loaded| {
            let state = window.global::<InstanceState>();
            let config = &loaded.summary.instance.config;
            state.set_name(config.name.as_str().into());
            state.set_minecraft(config.minecraft.as_str().into());
            state.set_loader_label(loader_label(&loaded.summary.instance).into());
            state.set_installed(loaded.summary.installed_version_id.is_some());
            state.set_running(run.is_running(&slug));

            let candidates = shared.candidates();
            let rows = content_rows(&loaded.content, &candidates);
            state.set_update_count(rows.iter().filter(|row| row.update_available).count() as i32);
            state.set_content(ModelRc::new(VecModel::from(rows)));

            let pending = loaded.summary.pending_manual.clone();
            state.set_pending(ModelRc::new(VecModel::from(pending_rows(&pending))));
            shared.set_pending(pending);

            state.set_overrides(ModelRc::new(VecModel::from(setting_rows(
                &config.settings_overrides,
            ))));
            state.set_options(ModelRc::new(VecModel::from(option_rows(&loaded.options))));

            // An instance that sets no heap shows the offered defaults. Saving the tab then
            // writes them as its own, which is what pressing Save means.
            let jvm = &config.jvm;
            state.set_jvm_min(heap_or_default(jvm.min_mib, HEAP_MIN_MIB));
            state.set_jvm_max(heap_or_default(jvm.max_mib, DEFAULT_MAX_MIB));
            state.set_jvm_extra(jvm.extra_args.join(" ").into());
            state.set_java_path(
                jvm.java_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
                    .into(),
            );
        },
    );
}

/// Installs a hand-downloaded file against the pending entry it belongs to.
fn import_manual(bridge: &Bridge, run: &RunState, shared: &Shared, project_id: &str, path: &Path) {
    let Some(slug) = shown_slug(bridge) else {
        return;
    };
    let Some(pending) = shared.pending_for(project_id) else {
        status(bridge, "That download is no longer pending");
        return;
    };
    // The pending entry records the kind the add resolved, so the import uses it as-is.
    let kind = pending.kind;
    let (job_slug, file) = (slug.clone(), path.to_path_buf());
    let after = (bridge.clone(), run.clone(), shared.clone());
    status(bridge, "Importing…");
    bridge.run(
        "Import download",
        move |launcher| {
            launcher
                .import_manual_file(&job_slug, &pending, &file, kind)
                .map(|_| ())
        },
        move |_window, ()| {
            let (bridge, run, shared) = after;
            status(&bridge, "Imported");
            load(&bridge, &run, &shared, &slug);
        },
    );
}

/// Whether a heap range can be saved: both bounds in range, and the minimum not above the
/// maximum.
pub fn jvm_valid(min: i32, max: i32) -> bool {
    (HEAP_MIN_MIB..=HEAP_MAX_MIB).contains(&min)
        && (HEAP_MIN_MIB..=HEAP_MAX_MIB).contains(&max)
        && min <= max
}

/// Builds the content rows, marking every entry a candidate names as updatable.
pub fn content_rows(entries: &[ContentEntry], candidates: &[UpdateCandidate]) -> Vec<ContentRow> {
    entries
        .iter()
        .map(|entry| {
            let update = candidates
                .iter()
                .any(|candidate| candidate.entry.project_id == entry.project_id);
            content_row(entry, update)
        })
        .collect()
}

/// Builds the JVM overrides from the four fields of the JVM tab.
///
/// An empty java path means "let the launcher choose", and extra arguments are split on
/// whitespace, which is how a command line reads them.
pub fn instance_jvm(min: i32, max: i32, extra: &str, java_path: &str) -> InstanceJvm {
    InstanceJvm {
        min_mib: Some(min.max(0) as u32),
        max_mib: Some(max.max(0) as u32),
        java_path: (!java_path.trim().is_empty()).then(|| PathBuf::from(java_path.trim())),
        extra_args: extra
            .split_whitespace()
            .map(|arg| arg.to_string())
            .collect(),
    }
}

/// The loader and build of an instance, as the header shows it.
pub fn loader_label(instance: &gcl_core::instances::Instance) -> String {
    let loader = instance.config.loader.to_string();
    if loader == "none" {
        return "vanilla".to_string();
    }
    match instance.config.loader_version.as_deref() {
        Some(version) => format!("{loader} {version}"),
        None => loader,
    }
}

/// Builds the pending rows, each labelled with the kind its import will use.
fn pending_rows(pending: &[ManualDownload]) -> Vec<PendingRow> {
    pending.iter().map(pending_row).collect()
}

/// Builds the read-only rows for the pairs `options.txt` holds.
fn option_rows(options: &[(String, String)]) -> Vec<SettingRow> {
    options
        .iter()
        .map(|(key, value)| SettingRow {
            key: key.as_str().into(),
            value: value.as_str().into(),
        })
        .collect()
}

/// A saved heap bound, or `fallback` when the instance sets none.
fn heap_or_default(mib: Option<u32>, fallback: i32) -> i32 {
    mib.map(|value| value.clamp(HEAP_MIN_MIB as u32, HEAP_MAX_MIB as u32) as i32)
        .unwrap_or(fallback)
}

/// Rewrites `update_available` on the rows the candidates name. Runs on the UI thread.
fn mark_updates(window: &AppWindow, candidates: &[UpdateCandidate]) {
    let state = window.global::<InstanceState>();
    let rows: Vec<ContentRow> = state
        .get_content()
        .iter()
        .map(|mut row| {
            row.update_available = candidates
                .iter()
                .any(|candidate| candidate.entry.project_id.as_str() == row.project_id.as_str());
            row
        })
        .collect();
    state.set_content(ModelRc::new(VecModel::from(rows)));
}

/// The slug the detail screen is showing, if it is up.
fn shown_slug(bridge: &Bridge) -> Option<String> {
    let window = bridge.weak().upgrade()?;
    let slug = window.global::<App>().get_current_slug().to_string();
    (!slug.is_empty()).then_some(slug)
}

/// Puts one line on the screen's status text.
fn status(bridge: &Bridge, text: &str) {
    if let Some(window) = bridge.weak().upgrade() {
        window
            .global::<InstanceState>()
            .set_status_text(text.into());
    }
}

#[cfg(test)]
mod tests;
