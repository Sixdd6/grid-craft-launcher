//! Wires the instance detail screen: header, content, settings, JVM, and the game log.
//!
//! Every call into `gcl-core` runs off the UI thread, through [`Bridge`] or, for a launch,
//! through [`crate::launch_flow`], which both this screen and the instances list share. The
//! screen itself is pure layout; this module owns the `InstanceState` global that feeds it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gcl_core::content::{ManualDownload, UpdateCandidate};
use gcl_core::instances::model::{ContentEntry, GcPreset, InstanceJvm};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::bridge::Bridge;
use crate::launch_flow;
use crate::models::{content_row, gc_rows, pending_row};
use crate::screens::settings_editor::{EditTarget, Editor};
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
    /// The typed settings editor both screens share. The Settings tab opens it on this
    /// instance's override layer every time the instance is read again.
    editor: Editor,
    /// Newer versions the last check found, in the order `apply_updates` wants them.
    candidates: Arc<Mutex<Vec<UpdateCandidate>>>,
    /// The manual downloads the loaded instance still needs, by project id.
    pending: Arc<Mutex<Vec<ManualDownload>>>,
    /// Every installed row the last load read, before `content_filter` narrows it.
    /// `InstanceState.content` is what `content_filter` leaves of this; a filter change
    /// re-derives it from here, never from the (already narrowed) rows on screen.
    content: Arc<Mutex<Vec<ContentRow>>>,
    /// The slug whose data is in `InstanceState` right now. It is what tells a fresh open
    /// from a re-read of the instance already on screen: only the first one throws away the
    /// game log and the status line, which no read from disk can fill again.
    shown: Arc<Mutex<String>>,
    /// Bumped every time `load` starts a GC probe job. A probe's `done` closure checks this
    /// against the value it captured before it started, so a slow probe for an instance the
    /// user has since navigated away from (or reloaded again) never paints its answer onto
    /// whatever is on screen by the time it lands.
    gc_generation: Arc<AtomicU64>,
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

    /// Replaces the full, unfiltered content list a load just read.
    fn set_content_rows(&self, rows: Vec<ContentRow>) {
        *self.content.lock().unwrap_or_else(|err| err.into_inner()) = rows;
    }

    /// A copy of the full, unfiltered content list.
    fn content_rows(&self) -> Vec<ContentRow> {
        self.content
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Marks `slug` as the instance on screen, and says whether that is a change.
    fn take_over(&self, slug: &str) -> bool {
        let mut shown = self.shown.lock().unwrap_or_else(|err| err.into_inner());
        if *shown == slug {
            return false;
        }
        *shown = slug.to_string();
        true
    }

    /// Starts a new GC probe generation and returns it. Call once per `load`, before the
    /// probe job is spawned.
    fn bump_gc_generation(&self) -> u64 {
        self.gc_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The current GC probe generation, for a probe job's `done` closure to compare against
    /// the value it captured when it started.
    fn gc_generation(&self) -> u64 {
        self.gc_generation.load(Ordering::SeqCst)
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
pub fn wire(window: &AppWindow, bridge: &Bridge, run: &RunState, editor: &Editor) {
    let state = window.global::<InstanceState>();
    let shared = Shared {
        editor: editor.clone(),
        ..Shared::default()
    };

    {
        let bridge = bridge.clone();
        let run = run.clone();
        let shared = shared.clone();
        state.on_load(move |slug| open(&bridge, &run, &shared, slug.as_str()));
    }

    {
        let weak = bridge.weak().clone();
        let shared = shared.clone();
        state.on_clear(move || {
            if let Some(window) = weak.upgrade() {
                clear_view(&window, &shared);
            }
        });
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
        let weak = bridge.weak().clone();
        let shared = shared.clone();
        state.on_filter_changed(move || {
            if let Some(window) = weak.upgrade() {
                apply_content_filter(&window, &shared);
            }
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
        let run = run.clone();
        let shared = shared.clone();
        state.on_check_updates(move || {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            status(&bridge, "Checking for updates…");
            let after = (bridge.clone(), run.clone(), shared.clone());
            let job_slug = slug.clone();
            bridge.run(
                "Check updates",
                move |launcher| launcher.check_updates(&job_slug),
                move |window, candidates| {
                    let (bridge, run, shared) = after;
                    let state = window.global::<InstanceState>();
                    state.set_status_text(
                        match candidates.len() {
                            0 => "Everything is up to date".to_string(),
                            n => format!("{n} update(s) available"),
                        }
                        .into(),
                    );
                    // `check_updates` may have backfilled a title an older writer left out;
                    // a fresh `load` is what shows it, and it rebuilds `content` with these
                    // candidates too, so the update marks this check found are not lost.
                    shared.set_candidates(candidates);
                    load(&bridge, &run, &shared, &slug);
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
        state.on_jvm_save(move |min, max, extra, java_path| {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            if !jvm_valid(min, max) {
                status(&bridge, "The minimum heap must not be above the maximum");
                return;
            }
            // The heap Save button edits only the heap, extra args, and java path; the GC
            // preset is saved separately by `gc_pick`. Reading it back off `InstanceState`
            // here, rather than defaulting it, is what keeps a heap save from wiping it.
            let gc_token = bridge
                .weak()
                .upgrade()
                .map(|window| window.global::<InstanceState>().get_gc_token().to_string())
                .unwrap_or_default();
            let jvm = instance_jvm(min, max, extra.as_str(), java_path.as_str(), &gc_token);
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
        let run = run.clone();
        let shared = shared.clone();
        state.on_gc_pick(move |token| {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            let Ok(preset) = token.as_str().parse::<GcPreset>() else {
                return;
            };
            if let Some(window) = bridge.weak().upgrade() {
                window.global::<InstanceState>().set_gc_loading(true);
            }
            let after = (bridge.clone(), run.clone(), shared.clone());
            let job_slug = slug.clone();
            bridge.run_with_error(
                "Save garbage collector",
                move |launcher| launcher.set_instance_gc(&job_slug, preset),
                move |_window, result| {
                    let (bridge, run, shared) = after;
                    // `run_with_error` already opened the error dialog on a failure. A
                    // success reloads the whole screen; a refusal reloads the GC block alone,
                    // which puts the combo back on the preset that is really saved instead of
                    // leaving it on the row the user picked and nothing accepted. Either
                    // reload clears `gc_loading` itself.
                    if result.is_ok() {
                        load(&bridge, &run, &shared, &slug);
                    } else {
                        let generation = shared.bump_gc_generation();
                        load_gc_support(&bridge, &shared, generation);
                    }
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

/// The screen's entry point: points the shared editor at this instance, then reads it.
///
/// This is the only place that retargets the editor. `app.slint` calls it when the shell
/// navigates here or the shown slug changes; every refresh below goes through [`load`]
/// instead, so a job that lands after the user walked back to the settings screen cannot
/// drag the editor onto an instance layer, and a refresh cannot wipe the search box.
///
/// `launch_flow::finish` calls this same entry point (through `InstanceState.invoke_load`)
/// once the game exits, to refresh the screen it is showing. That is a re-read of the
/// instance already on screen, not a switch to a new one, so it must not retarget the
/// editor: `take_over` tells the two apart, and only a genuine switch (or the editor
/// pointing somewhere else, such as the settings screen's defaults layer) clears the search
/// box and status line. [`load`] below still refreshes the editor's rows either way, through
/// its own guarded `editor.reload`.
fn open(bridge: &Bridge, run: &RunState, shared: &Shared, slug: &str) {
    // Another instance's data must not be on screen while this one is being read, and a
    // game log belongs to the game that wrote it. A re-read of the instance already shown
    // keeps both: the launch that just ended set the status line and filled the log.
    let switched = shared.take_over(slug);
    if switched && let Some(window) = bridge.weak().upgrade() {
        clear_view(&window, shared);
    }
    if switched || shared.editor.target() != EditTarget::Instance(slug.to_string()) {
        shared
            .editor
            .open(bridge, EditTarget::Instance(slug.to_string()));
    }
    load(bridge, run, shared, slug);
}

/// Empties every property the screen shows, before another instance is read into it.
///
/// The prompt's labels are left alone: they are what the prompt says, not what an instance
/// holds, and the prompt itself is closed here.
fn clear_view(window: &AppWindow, shared: &Shared) {
    shared.set_content_rows(Vec::new());
    let state = window.global::<InstanceState>();
    state.set_name(SharedString::new());
    state.set_minecraft(SharedString::new());
    state.set_loader_label(SharedString::new());
    state.set_installed(false);
    state.set_running(false);
    state.set_status_text(SharedString::new());
    state.set_tab(0);
    state.set_content_filter(SharedString::new());
    state.set_content(ModelRc::new(VecModel::from(Vec::<ContentRow>::new())));
    state.set_pending(ModelRc::new(VecModel::from(Vec::<PendingRow>::new())));
    state.set_has_options(false);
    state.set_options(ModelRc::new(VecModel::from(Vec::<SettingRow>::new())));
    state.set_jvm_min(0);
    state.set_jvm_max(0);
    state.set_jvm_extra(SharedString::new());
    state.set_java_path(SharedString::new());
    state.set_gc_token(SharedString::new());
    state.set_gc_options(ModelRc::new(VecModel::from(
        Vec::<crate::GcPresetRow>::new(),
    )));
    state.set_gc_labels(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
    state.set_gc_selected_index(0);
    state.set_gc_status(SharedString::new());
    state.set_gc_loading(false);
    state.set_gc_unavailable_reason(SharedString::new());
    // A game log is the output of one game. An instance nobody has launched has none.
    state.set_game_log(ModelRc::new(VecModel::from(Vec::<crate::LogLine>::new())));
    state.set_update_count(0);
    state.set_prompt_open(false);
    state.set_prompt_value(SharedString::new());
    // The mode says what an open prompt is for. A closed prompt is for nothing.
    state.set_prompt_mode(SharedString::new());
}

/// Reads the instance and fills every property the screen shows.
fn load(bridge: &Bridge, run: &RunState, shared: &Shared, slug: &str) {
    // The Settings tab shows this instance's layer. Re-read its rows only while the shared
    // editor is still on this instance: another screen may own it by now.
    if shared.editor.target() == EditTarget::Instance(slug.to_string()) {
        shared.editor.reload(bridge);
    }
    // A fresh generation for the GC probe this load starts once the fast fields land, so a
    // probe from an earlier load (or one still running for this same instance) never paints
    // over what a later load already put on screen.
    let generation = shared.bump_gc_generation();
    let (job_slug, run, shared, gc_bridge) = (
        slug.to_string(),
        run.clone(),
        shared.clone(),
        bridge.clone(),
    );
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
            shared.set_content_rows(rows);
            apply_content_filter(window, &shared);

            let pending = loaded.summary.pending_manual.clone();
            state.set_pending(ModelRc::new(VecModel::from(pending_rows(&pending))));
            shared.set_pending(pending);

            state.set_has_options(!loaded.options.is_empty());
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
            state.set_gc_token(jvm.gc.to_string().into());
            load_gc_support(&gc_bridge, &shared, generation);
        },
    );
}

/// Runs the GC probe for the instance the JVM tab's fast fields just loaded, off its own
/// `Bridge::run` job so a runtime download it may need does not hold up the heap fields.
///
/// `generation` is the value [`Shared::bump_gc_generation`] returned right before the fast
/// job started; the probe's answer is discarded once it lands if that is no longer current,
/// which is what keeps a slow probe for one instance from painting into another (or into a
/// later load of the same one).
fn load_gc_support(bridge: &Bridge, shared: &Shared, generation: u64) {
    let Some(slug) = shown_slug(bridge) else {
        return;
    };
    if let Some(window) = bridge.weak().upgrade() {
        let state = window.global::<InstanceState>();
        state.set_gc_loading(true);
        state.set_gc_status(SharedString::new());
        state.set_gc_unavailable_reason(SharedString::new());
    }
    let job_slug = slug.clone();
    let shared = shared.clone();
    // `run`, not `run_with_error`: the probe is background work nobody asked for, and a JVM
    // that cannot answer it belongs on the tab's own status line, not in a modal over
    // whatever the user was doing. The job therefore carries its own `Result` as its payload,
    // so `Bridge` sees an `Ok` either way and opens no dialog.
    bridge.run(
        "Load GC support",
        move |launcher| Ok(launcher.gc_support(&job_slug)),
        move |window, result| {
            if shared.gc_generation() != generation {
                return;
            }
            let state = window.global::<InstanceState>();
            state.set_gc_loading(false);
            match result {
                Ok(view) => {
                    // The saved preset comes from the view, not from `gc_token`: it is the
                    // preset as this Java runs it, so a plain ZGC saved before the runtime
                    // moved to Java 23 selects the generational row that Java really offers.
                    let saved = view.saved;
                    let saved_token = saved.to_string();
                    let rows = gc_rows(&view.presets);
                    let labels: Vec<SharedString> =
                        rows.iter().map(|row| row.label.clone()).collect();
                    let selected = rows.iter().position(|row| row.token == saved_token);
                    state.set_gc_options(ModelRc::new(VecModel::from(rows)));
                    state.set_gc_labels(ModelRc::new(VecModel::from(labels)));
                    // The heap Save button sends this token back, so it has to be the folded
                    // one as well: saving the unfolded name would write a preset this Java
                    // does not list.
                    state.set_gc_token(saved_token.into());
                    match selected {
                        Some(index) => {
                            state.set_gc_selected_index(index as i32);
                            state.set_gc_unavailable_reason(SharedString::new());
                        }
                        None => {
                            state.set_gc_selected_index(0);
                            state.set_gc_unavailable_reason(
                                format!(
                                    "Unavailable: {} — the current Java does not support it",
                                    saved.label()
                                )
                                .into(),
                            );
                        }
                    }
                    state.set_gc_status(view.label.into());
                }
                Err(err) => {
                    state.set_gc_status(SharedString::new());
                    state.set_gc_unavailable_reason(
                        format!(
                            "Could not check the garbage collector: {}",
                            crate::bridge::error_chain(&err)
                        )
                        .into(),
                    );
                }
            }
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

/// Re-derives `InstanceState.content` from the full row list `shared` holds and whatever
/// `InstanceState.content_filter` holds right now. Called after every load and every time the
/// search box changes, so a filtered list is never built from an already-narrowed one.
fn apply_content_filter(window: &AppWindow, shared: &Shared) {
    let state = window.global::<InstanceState>();
    let all = shared.content_rows();
    let filtered = filter_content_rows(&all, state.get_content_filter().as_str());
    state.set_content(ModelRc::new(VecModel::from(filtered)));
}

/// Keeps the rows whose title or file name contains `filter`, ignoring case. An empty filter
/// keeps every row.
pub fn filter_content_rows(rows: &[ContentRow], filter: &str) -> Vec<ContentRow> {
    let needle = filter.trim().to_lowercase();
    if needle.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|row| {
            row.name.to_lowercase().contains(&needle)
                || row.file_name.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect()
}

/// Builds the content rows, marking every entry a candidate names as updatable, sorted by the
/// row's own name (title, or file stem when it has none), case-insensitively.
pub fn content_rows(entries: &[ContentEntry], candidates: &[UpdateCandidate]) -> Vec<ContentRow> {
    let mut rows: Vec<ContentRow> = entries
        .iter()
        .map(|entry| {
            let update = candidates
                .iter()
                .any(|candidate| candidate.entry.project_id == entry.project_id);
            content_row(entry, update)
        })
        .collect();
    rows.sort_by_cached_key(|row| row.name.to_lowercase());
    rows
}

/// Builds the JVM overrides from the heap Save button's four fields, plus the GC preset
/// carried through from `InstanceState.gc_token` so this save cannot wipe it.
///
/// An empty java path means "let the launcher choose", extra arguments are split on
/// whitespace, which is how a command line reads them, and an empty or unparsable
/// `gc_token` (nothing probed yet) keeps [`GcPreset::Default`], the same as a fresh
/// instance's own default.
pub fn instance_jvm(
    min: i32,
    max: i32,
    extra: &str,
    java_path: &str,
    gc_token: &str,
) -> InstanceJvm {
    InstanceJvm {
        min_mib: Some(min.max(0) as u32),
        max_mib: Some(max.max(0) as u32),
        java_path: (!java_path.trim().is_empty()).then(|| PathBuf::from(java_path.trim())),
        extra_args: extra
            .split_whitespace()
            .map(|arg| arg.to_string())
            .collect(),
        gc: gc_token.parse().unwrap_or_default(),
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
