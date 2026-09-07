//! Wires the instance detail screen: header, content, settings, JVM, and the game log.
//!
//! Every call into `gcl-core` runs off the UI thread, through [`Bridge`] or through the two
//! threads a launch owns: one waits for the game, one tails its log file. The screen itself
//! is pure layout; this module owns the `InstanceState` global that feeds it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gcl_core::content::{ManualDownload, UpdateCandidate};
use gcl_core::instances::model::{ContentEntry, ContentKind, InstanceJvm};
use gcl_core::launcher::LaunchOutcome;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use crate::bridge::{Bridge, show_error, warn};
use crate::models::{content_row, pending_row, setting_rows};
use crate::state::RunState;
use crate::{App, AppWindow, ContentRow, InstanceState, LogLine, PendingRow, Screen, SettingRow};

/// Smallest heap the JVM tab offers, in MiB. Matches the `SpinBox` bounds in the screen.
const HEAP_MIN_MIB: i32 = 512;

/// Largest heap the JVM tab offers, in MiB. Matches the `SpinBox` bounds in the screen.
const HEAP_MAX_MIB: i32 = 65536;

/// Heap shown for a maximum an instance does not set. The launch itself falls back to
/// `config.toml`, so this is only what the tab offers before the user saves anything.
const DEFAULT_MAX_MIB: i32 = 2048;

/// How often the log tail re-reads the running game's log file.
const TAIL_INTERVAL: Duration = Duration::from_millis(250);

/// Most game log lines kept in the model. Older lines are dropped from the front.
const GAME_LOG_LIMIT: usize = 2000;

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
        let weak = bridge.weak().clone();
        state.on_stop(move || {
            // The MVP has no way to stop the game: `RunningLaunch` carries a pid, not a
            // handle that can kill it. Say so rather than doing nothing silently.
            if let Some(window) = weak.upgrade() {
                window
                    .global::<InstanceState>()
                    .set_status_text("Stopping the game is not supported yet".into());
            }
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
            if let Some(window) = weak.upgrade() {
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
        let shared = shared.clone();
        state.on_launch(move || {
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            launch(&bridge, &run, &shared, &slug, None);
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
            let Some(slug) = shown_slug(&bridge) else {
                return;
            };
            launch(&bridge, &run, &shared, &slug, Some(name));
        });
    }

    {
        let weak = bridge.weak().clone();
        state.on_prompt_cancel(move || {
            if let Some(window) = weak.upgrade() {
                let state = window.global::<InstanceState>();
                state.set_prompt_open(false);
                state.set_status_text("Launch cancelled: no account".into());
            }
        });
    }
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
    let kind = pending_kind(&pending);
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

/// Starts the game and follows it: one thread waits for it, one tails its log.
///
/// `offline_user` is the name typed into the prompt, and is `None` for the first attempt: a
/// launch with no account comes back as [`gcl_core::auth::Error::NoAccount`], and that is what
/// opens the prompt.
fn launch(
    bridge: &Bridge,
    run: &RunState,
    shared: &Shared,
    slug: &str,
    offline_user: Option<String>,
) {
    if !run.start(slug) {
        return;
    }
    if let Some(window) = bridge.weak().upgrade() {
        let state = window.global::<InstanceState>();
        state.set_running(true);
        state.set_status_text("Starting…".into());
        state.set_game_log(ModelRc::new(VecModel::from(Vec::<LogLine>::new())));
    }

    let slug = slug.to_string();
    let launcher = Arc::clone(bridge.launcher());
    let weak = bridge.weak().clone();
    let bridge = bridge.clone();
    let run = run.clone();
    let shared = shared.clone();
    std::thread::spawn(move || {
        let started = launcher.launch_instance_async(&slug, None, offline_user.as_deref());
        let outcome = match started {
            Ok(running) => {
                let stop = Arc::new(AtomicBool::new(false));
                let tail = start_tail(weak.clone(), running.log_path.clone(), Arc::clone(&stop));
                let outcome = running.wait_blocking(&launcher);
                stop.store(true, Ordering::Relaxed);
                let _ = tail.join();
                outcome
            }
            Err(err) => Err(err),
        };
        run.finish(&slug);
        finish_launch(&weak, &bridge, &run, &shared, &slug, outcome);
    });
}

/// Reports what the game did and reloads the screen. Runs on the launch thread.
fn finish_launch(
    weak: &Weak<AppWindow>,
    bridge: &Bridge,
    run: &RunState,
    shared: &Shared,
    slug: &str,
    outcome: Result<LaunchOutcome, gcl_core::Error>,
) {
    let (bridge, run, shared) = (bridge.clone(), run.clone(), shared.clone());
    let slug = slug.to_string();
    let _ = weak.upgrade_in_event_loop(move |window| {
        let state = window.global::<InstanceState>();
        state.set_running(false);
        match outcome {
            Ok(LaunchOutcome::Exited { code, hint, .. }) if code != 0 => {
                let hint = hint.unwrap_or_else(|| "see the instance log".to_string());
                let text = format!("Minecraft exited with code {code}: {hint}");
                state.set_status_text(text.as_str().into());
                warn(&window, &text);
            }
            Ok(_) => state.set_status_text("Minecraft exited".into()),
            Err(err) if is_no_account(&err) => {
                // The prompt is the answer to this error, so it replaces the error dialog.
                state.set_status_text("No account: choose an offline name".into());
                state.set_prompt_title("Play offline".into());
                state.set_prompt_label("Player name".into());
                state.set_prompt_open(true);
            }
            Err(err) => {
                state.set_status_text("Launch failed".into());
                show_error(&window, "Launch", &err);
            }
        }
        load(&bridge, &run, &shared, &slug);
    });
}

/// Starts the thread that copies new lines of the game's log into the screen.
///
/// The game writes both of its output streams to one file, so tailing that file is how the
/// UI sees the game without core growing a second event kind.
fn start_tail(
    weak: Weak<AppWindow>,
    path: PathBuf,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut read = 0usize;
        loop {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                let (lines, next) = tail_new_lines(read, &contents);
                read = next;
                if !lines.is_empty()
                    && weak
                        .upgrade_in_event_loop(move |window| append_game_log(&window, &lines))
                        .is_err()
                {
                    return;
                }
            }
            // Checked after the read, so the last lines of a finished game still land.
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(TAIL_INTERVAL);
        }
    })
}

/// Appends log lines, dropping the oldest once the model is full. Runs on the UI thread.
fn append_game_log(window: &AppWindow, lines: &[String]) {
    let state = window.global::<InstanceState>();
    let mut log: Vec<LogLine> = state.get_game_log().iter().collect();
    log.extend(lines.iter().map(|text| LogLine {
        level: "info".into(),
        text: text.as_str().into(),
    }));
    if log.len() > GAME_LOG_LIMIT {
        log.drain(..log.len() - GAME_LOG_LIMIT);
    }
    state.set_game_log(ModelRc::new(VecModel::from(log)));
}

/// The complete lines `contents` grew since `prev_len` bytes, and the new byte count.
///
/// A partial last line is left for the next call, so a line is only shown once it is whole. A
/// file that shrank, or an offset that is not a character boundary, is read again from the
/// start: a truncated log is rare, and re-reading it is better than losing it.
pub fn tail_new_lines(prev_len: usize, contents: &str) -> (Vec<String>, usize) {
    let from = if prev_len <= contents.len() && contents.is_char_boundary(prev_len) {
        prev_len
    } else {
        0
    };
    let fresh = &contents[from..];
    let complete = match fresh.rfind('\n') {
        Some(at) => at + 1,
        None => return (Vec::new(), from),
    };
    let lines = fresh[..complete]
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    (lines, from + complete)
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

/// What kind of content a hand download is, as far as the pending entry says.
///
/// The pending list does not record the kind, so it is read back off the entry: a data pack
/// names the world it belongs to, and a jar is a mod. Anything else is treated as a resource
/// pack, which is where a loose zip belongs. `content import --kind` in the CLI has the full
/// choice for the cases this guesses wrong.
pub fn pending_kind(pending: &ManualDownload) -> ContentKind {
    if pending.world.is_some() {
        ContentKind::DataPack
    } else if pending.file_name.to_lowercase().ends_with(".jar") {
        ContentKind::Mod
    } else {
        ContentKind::ResourcePack
    }
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
    pending
        .iter()
        .map(|item| pending_row(item, &pending_kind(item).to_string()))
        .collect()
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

/// Whether a launch failed only because no account is selected.
fn is_no_account(err: &gcl_core::Error) -> bool {
    matches!(err, gcl_core::Error::Auth(gcl_core::auth::Error::NoAccount))
}

#[cfg(test)]
mod tests;
