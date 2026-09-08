//! Wires the content browser: search, add to an instance, and modpack install.
//!
//! Every call into `gcl-core` runs off the UI thread through [`Bridge`]. The screen is pure
//! layout; this module owns the `BrowserState` global that feeds it, and it owns the ids the
//! screen's labels stand for: the source list, the instance list, and whatever a modal is
//! waiting to finish.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gcl_core::content::{AddRequest, DependencyConflict};
use gcl_core::instances::Instance;
use gcl_core::instances::model::{ContentKind, Loader};
use gcl_core::sources::{SearchHit, SearchQuery, SourceId};
use slint::{
    ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel,
};

use crate::bridge::{Bridge, warn};
use crate::models::{decode_icon, search_row};
use crate::{App, AppWindow, BrowserState, Screen, SearchRow};

/// How many hits one page asks for. Matches `BrowserState.page_size`.
const PAGE_SIZE: i32 = 20;

/// The kind that is installed as its own instance instead of into one.
const MODPACK: &str = "modpack";

/// What the loader ComboBox shows for "do not filter by loader".
const ANY_LOADER: &str = "any";

/// The loader ComboBox rows, in the order `BrowserState.loader_options` declares them.
///
/// A ComboBox only moves its selection through `current-index`, so the screen reports an
/// index and this list is what turns it back into the slug the search filter carries.
const LOADER_OPTIONS: [&str; 5] = [ANY_LOADER, "fabric", "quilt", "forge", "neoforge"];

/// Shown when CurseForge is the source that is missing.
const NO_CURSEFORGE: &str = "CurseForge disabled: set CURSEFORGE_API_KEY";

/// One instance the browser can install into.
#[derive(Clone)]
struct Target {
    /// Directory name, which is what every launcher call takes.
    slug: String,
    /// Display name, which is what the ComboBox and the Add button show.
    name: String,
    /// Its Minecraft version, prefilled into the search filters.
    minecraft: String,
    /// Its loader slug, prefilled into the search filters.
    loader: String,
}

/// A project the world chooser is open for, and the kind the row it came from named.
#[derive(Clone)]
struct Pending {
    /// Project id at the source.
    project: String,
    /// The row's own kind, which is what the add installs as.
    kind: Option<ContentKind>,
}

/// What the name prompt is about to install as a new instance.
#[derive(Clone)]
enum Pack {
    /// A project id or slug to fetch from the chosen source.
    Project(String),
    /// A `.mrpack` or CurseForge zip already on disk.
    File(PathBuf),
}

/// What the screen's indices and its modals stand for.
///
/// The labels in the Slint models carry no ids, so the lists behind them live here. Cloning
/// shares one set, which is what lets every callback look the same thing up.
#[derive(Clone, Default)]
struct Shared {
    /// Sources that are enabled, in the order `source_labels` shows them.
    sources: Arc<Mutex<Vec<SourceId>>>,
    /// Instances that can be a target, in the order `target_labels` shows them.
    targets: Arc<Mutex<Vec<Target>>>,
    /// The project the world chooser is open for, with the kind its row named.
    pending_world: Arc<Mutex<Option<Pending>>>,
    /// The pack the name prompt is open for, from a source or from disk.
    pending_pack: Arc<Mutex<Option<Pack>>>,
    /// Bumped by every `search`, so an icon batch that finishes after a newer search has
    /// started does not paint its rows over that newer search's own rows.
    icon_generation: Arc<AtomicU64>,
}

impl Shared {
    /// Replaces the enabled sources.
    fn set_sources(&self, list: Vec<SourceId>) {
        *self.sources.lock().unwrap_or_else(|err| err.into_inner()) = list;
    }

    /// The source at one ComboBox index, if it names one.
    fn source_at(&self, index: i32) -> Option<SourceId> {
        let sources = self.sources.lock().unwrap_or_else(|err| err.into_inner());
        usize::try_from(index)
            .ok()
            .and_then(|i| sources.get(i))
            .copied()
    }

    /// Replaces the instances the browser can install into.
    fn set_targets(&self, list: Vec<Target>) {
        *self.targets.lock().unwrap_or_else(|err| err.into_inner()) = list;
    }

    /// The target at one ComboBox index, if it names one.
    fn target_at(&self, index: i32) -> Option<Target> {
        let targets = self.targets.lock().unwrap_or_else(|err| err.into_inner());
        usize::try_from(index)
            .ok()
            .and_then(|i| targets.get(i))
            .cloned()
    }

    /// The target with this slug, if the last load carried one.
    fn target_for(&self, slug: &str) -> Option<Target> {
        self.targets
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .find(|target| target.slug == slug)
            .cloned()
    }

    /// Remembers, or forgets, the project the world chooser is open for.
    fn set_pending_world(&self, project: Option<Pending>) {
        *self
            .pending_world
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = project;
    }

    /// Takes the project the world chooser was open for.
    fn take_pending_world(&self) -> Option<Pending> {
        self.pending_world
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
    }

    /// Remembers, or forgets, the pack the name prompt is open for.
    fn set_pending_pack(&self, project: Option<Pack>) {
        *self
            .pending_pack
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = project;
    }

    /// Takes the pack the name prompt was open for.
    fn take_pending_pack(&self) -> Option<Pack> {
        self.pending_pack
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
    }
}

/// Binds the `BrowserState` global to the launcher.
///
/// Nothing is loaded here: `app.slint` calls `open` when the shell navigates to this screen,
/// because that is when the source list and the prefilled filters are decided.
pub fn wire(window: &AppWindow, bridge: &Bridge) {
    let state = window.global::<BrowserState>();
    let shared = Shared::default();

    state.set_page_size(PAGE_SIZE);

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_open(move || open(&bridge, &shared));
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_search(move || {
            set_page(&bridge, 0);
            search(&bridge, &shared);
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_search_packs(move || {
            set_page(&bridge, 0);
            search_packs(&bridge, &shared);
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_next_page(move || {
            let page = page_of(&bridge);
            set_page(&bridge, page + 1);
            search_this_kind(&bridge, &shared);
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_prev_page(move || {
            let page = page_of(&bridge);
            if page == 0 {
                return;
            }
            set_page(&bridge, page - 1);
            search_this_kind(&bridge, &shared);
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_set_source(move |index| {
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            let state = window.global::<BrowserState>();
            state.set_source_index(index);
            // A source that cannot serve a kind must not offer it, so the list is rebuilt
            // and whatever the user had picked is pulled back into range.
            if let Some(source) = shared.source_at(index) {
                set_kinds(&state, source);
            }
            clear_rows(&state);
        });
    }

    {
        let bridge = bridge.clone();
        state.on_set_kind(move |index| {
            if let Some(window) = bridge.weak().upgrade() {
                let state = window.global::<BrowserState>();
                state.set_kind_index(index);
                clear_rows(&state);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_set_loader(move |index| {
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            let state = window.global::<BrowserState>();
            state.set_loader_index(index);
            state.set_loader(loader_option_at(index).into());
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_set_target(move |index| {
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            let state = window.global::<BrowserState>();
            state.set_target_index(index);
            apply_target(&state, shared.target_at(index).as_ref());
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_add(move |project_id, kind| {
            add(&bridge, &shared, project_id.as_str(), kind.as_str(), None)
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_pick_world(move |project_id, kind| {
            pick_world(&bridge, &shared, project_id.as_str(), kind.as_str())
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_choice_ok(move |world| {
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            window.global::<BrowserState>().set_choice_open(false);
            let Some(pending) = shared.take_pending_world() else {
                return;
            };
            install(
                &bridge,
                &shared,
                &pending.project,
                pending.kind,
                Some(world.to_string()),
            );
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_choice_cancel(move || {
            shared.set_pending_world(None);
            if let Some(window) = bridge.weak().upgrade() {
                let state = window.global::<BrowserState>();
                state.set_choice_open(false);
                state.set_status("Add cancelled: no world chosen".into());
            }
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_install_pack(move |project_id, title| {
            let project = project_id.trim().to_string();
            if project.is_empty() {
                return;
            }
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            // The pack's own title is the name a user recognises, so the prompt starts
            // there. A pack installed by typing an id carries no title, and then the id is
            // all there is to offer.
            let suggested = match title.trim() {
                "" => project.as_str(),
                title => title,
            }
            .to_string();
            shared.set_pending_pack(Some(Pack::Project(project)));
            let state = window.global::<BrowserState>();
            state.set_name_value(suggested.as_str().into());
            state.set_name_open(true);
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_install_pack_file(move |path| {
            let path = PathBuf::from(path.trim());
            if path.as_os_str().is_empty() {
                return;
            }
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            let state = window.global::<BrowserState>();
            // The archive's own name is the obvious first name for the instance, and the
            // prompt lets the user change it before anything is written.
            let suggested = pack_name(&path);
            shared.set_pending_pack(Some(Pack::File(path)));
            state.set_name_value(suggested.as_str().into());
            state.set_name_open(true);
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_name_ok(move |name| install_pack(&bridge, &shared, name.as_str()));
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_name_cancel(move || {
            shared.set_pending_pack(None);
            if let Some(window) = bridge.weak().upgrade() {
                let state = window.global::<BrowserState>();
                state.set_name_open(false);
                state.set_status("Modpack install cancelled".into());
            }
        });
    }
}

/// What one open reads off disk, as the UI-thread half of the open then shows it.
struct Opened {
    /// The sources that are enabled right now.
    sources: Vec<SourceId>,
    /// Every instance, as a possible target.
    targets: Vec<Target>,
}

/// Loads the sources and the instance list, then prefills the filters.
///
/// The target is the instance the shell already has open. With none — the browser was reached
/// from the rail — the screen offers a ComboBox of every instance instead.
fn open(bridge: &Bridge, shared: &Shared) {
    let shared = shared.clone();
    run_reporting(
        bridge,
        "Open browser",
        |launcher| {
            let sources = launcher.sources().iter().map(|s| s.id()).collect();
            let targets = launcher.instances().list()?.iter().map(target).collect();
            Ok(Opened { sources, targets })
        },
        move |window, opened| {
            let state = window.global::<BrowserState>();
            // A page of hits belongs to the search that found it. Coming back to the screen
            // from somewhere else is not that search, so the pager and both lists start
            // over — unless `ProjectState.back()` just sent us here, in which case this
            // page of hits is the one the user searched for and Back must not lose it.
            let returning = state.get_returning();
            state.set_returning(false);
            if !returning {
                clear_rows(&state);
            }
            let labels: Vec<SharedString> = opened
                .sources
                .iter()
                .map(|s| s.to_string().into())
                .collect();
            state.set_source_labels(ModelRc::new(VecModel::from(labels)));
            let source_index = clamp_index(state.get_source_index(), opened.sources.len());
            state.set_source_index(source_index);
            shared.set_sources(opened.sources.clone());
            if let Some(source) = opened.sources.get(source_index.max(0) as usize) {
                set_kinds(&state, *source);
            }

            shared.set_targets(opened.targets.clone());
            // The detail screen's Add content sets `fixed_target`; the rail clears it. A
            // fixed target with no slug behind it can only be a bug, so the ComboBox is
            // offered rather than leaving the screen with nothing to add to.
            let slug = window.global::<App>().get_current_slug().to_string();
            if slug.is_empty() || !state.get_fixed_target() {
                let names: Vec<SharedString> = opened
                    .targets
                    .iter()
                    .map(|target| target.name.as_str().into())
                    .collect();
                let index = clamp_index(state.get_target_index().max(0), opened.targets.len());
                state.set_fixed_target(false);
                state.set_target_labels(ModelRc::new(VecModel::from(names)));
                state.set_target_index(index);
                apply_target(&state, shared.target_at(index).as_ref());
            } else {
                // The shell already names the instance, so the ComboBox would only offer a
                // choice the user has made. An instance that vanished under us keeps its
                // slug as its label, and the add fails with a clear error.
                state.set_target_labels(ModelRc::new(VecModel::from(Vec::<SharedString>::new())));
                state.set_target_index(-1);
                match shared.target_for(&slug) {
                    Some(target) => apply_target(&state, Some(&target)),
                    None => {
                        state.set_target_slug(slug.as_str().into());
                        state.set_target_name(slug.as_str().into());
                    }
                }
            }

            state.set_status(source_status(&opened.sources).into());
        },
    );
}

/// Runs whichever search the chosen kind means, on the page the screen is already on.
///
/// The pager sends both kinds of search through here, so Next and Prev keep listing the same
/// thing the last Search button press listed.
fn search_this_kind(bridge: &Bridge, shared: &Shared) {
    let modpack = bridge
        .weak()
        .upgrade()
        .is_some_and(|window| kind_label(&window.global::<BrowserState>()) == MODPACK);
    if modpack {
        search_packs(bridge, shared);
    } else {
        search(bridge, shared);
    }
}

/// Searches the chosen source for modpacks and shows its page.
///
/// A modpack is not a [`ContentKind`], so it has its own search and its own row list: every
/// hit becomes a new instance rather than a file inside one.
fn search_packs(bridge: &Bridge, shared: &Shared) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<BrowserState>();
    let Some(source) = shared.source_at(state.get_source_index()) else {
        state.set_status("No content source is enabled".into());
        return;
    };
    let page = state.get_page().max(0);
    let query = SearchQuery {
        text: state.get_query().to_string(),
        // A pack search takes neither: `search_packs` asks for the modpack project type
        // itself, and a pack states its loader in the pack index.
        kind: None,
        minecraft: some_text(state.get_minecraft().as_str()),
        loader: None,
        offset: (page * PAGE_SIZE) as u32,
        limit: PAGE_SIZE as u32,
    };

    state.set_loading(true);
    state.set_status("Searching modpacks…".into());
    run_reporting(
        bridge,
        "Search modpacks",
        move |launcher| launcher.search_packs(source, &query),
        move |window, found| {
            let state = window.global::<BrowserState>();
            let rows: Vec<SearchRow> = found.hits.iter().map(search_row).collect();
            state.set_has_more(page_bounds(page, PAGE_SIZE, rows.len(), found.total));
            state.set_status(result_status(rows.len(), found.total).into());
            state.set_pack_rows(ModelRc::new(VecModel::from(rows)));
        },
    );
}

/// Runs the search the filters describe and shows its page.
fn search(bridge: &Bridge, shared: &Shared) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<BrowserState>();
    let Some(source) = shared.source_at(state.get_source_index()) else {
        state.set_status("No content source is enabled".into());
        return;
    };
    let kind_label = kind_label(&state);
    if kind_label == MODPACK {
        // The modpack kind has a search of its own, because a pack is not content.
        search_packs(bridge, shared);
        return;
    }
    let page = state.get_page().max(0);
    let query = SearchQuery {
        text: state.get_query().to_string(),
        kind: ContentKind::parse(&kind_label),
        minecraft: some_text(state.get_minecraft().as_str()),
        loader: parse_loader(state.get_loader().as_str()),
        offset: (page * PAGE_SIZE) as u32,
        limit: PAGE_SIZE as u32,
    };

    state.set_loading(true);
    state.set_status("Searching…".into());
    // Stamped before the search runs, so any icon batch from an older search that is still
    // in flight when this page lands is recognized as stale and drops its result instead of
    // painting over these rows.
    let generation = shared.icon_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let bridge_for_icons = bridge.clone();
    let shared_for_icons = shared.clone();
    run_reporting(
        bridge,
        "Search",
        move |launcher| launcher.search(source, &query),
        move |window, found| {
            let state = window.global::<BrowserState>();
            let rows: Vec<SearchRow> = found.hits.iter().map(search_row).collect();
            state.set_has_more(page_bounds(page, PAGE_SIZE, rows.len(), found.total));
            state.set_status(result_status(rows.len(), found.total).into());
            state.set_rows(ModelRc::new(VecModel::from(rows)));
            fetch_icons(
                &bridge_for_icons,
                &shared_for_icons,
                generation,
                &found.hits,
            );
        },
    );
}

/// Adds one row, asking for a world first when the row is a data pack.
///
/// `kind` is the row's own kind, not the Type selector's: the selector can be changed after a
/// search, and the rows on screen keep the kind they were found with.
fn add(bridge: &Bridge, shared: &Shared, project_id: &str, kind: &str, world: Option<String>) {
    if world.is_none() && needs_world(kind) {
        pick_world(bridge, shared, project_id, kind);
        return;
    }
    install(bridge, shared, project_id, ContentKind::parse(kind), world);
}

/// Installs one project into the chosen instance, into `world` when it is a data pack.
fn install(
    bridge: &Bridge,
    shared: &Shared,
    project_id: &str,
    kind: Option<ContentKind>,
    world: Option<String>,
) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<BrowserState>();
    let Some(source) = shared.source_at(state.get_source_index()) else {
        state.set_status("No content source is enabled".into());
        return;
    };
    let slug = state.get_target_slug().to_string();
    if slug.is_empty() {
        state.set_status("Pick an instance to add to first".into());
        return;
    }
    let request = add_request(source, project_id, kind, world);

    state.set_loading(true);
    state.set_status("Installing\u{2026}".into());
    run_reporting(
        bridge,
        "Add content",
        move |launcher| launcher.add_content(&slug, request),
        move |window, outcome| {
            let state = window.global::<BrowserState>();
            if !outcome.manual.is_empty() {
                warn(window, &manual_note(outcome.manual.len()));
            }
            if !outcome.conflicts.is_empty() {
                warn(window, &conflict_note(&outcome.conflicts));
            }
            state.set_status(format!("installed {}", outcome.installed.len()).into());
        },
    );
}

/// Asks which world a data pack goes into, then adds it.
///
/// Anything that is not a data pack is installed straight away, so the screen can send every
/// row through here without knowing the rule.
fn pick_world(bridge: &Bridge, shared: &Shared, project_id: &str, kind: &str) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<BrowserState>();
    if !needs_world(kind) {
        install(bridge, shared, project_id, ContentKind::parse(kind), None);
        return;
    }
    let slug = state.get_target_slug().to_string();
    if slug.is_empty() {
        state.set_status("Pick an instance to add to first".into());
        return;
    }

    let pending = Pending {
        project: project_id.to_string(),
        kind: ContentKind::parse(kind),
    };
    let shared = shared.clone();
    state.set_loading(true);
    run_reporting(
        bridge,
        "List worlds",
        move |launcher| launcher.list_worlds(&slug),
        move |window, worlds| {
            let state = window.global::<BrowserState>();
            if worlds.is_empty() {
                state.set_status("No world to install a data pack into".into());
                error_dialog(
                    window,
                    "No world yet",
                    "A data pack is installed into one world's datapacks folder. \
                     Launch this instance and create a world first.",
                );
                return;
            }
            let options: Vec<SharedString> =
                worlds.iter().map(|name| name.as_str().into()).collect();
            shared.set_pending_world(Some(pending));
            state.set_choice_options(ModelRc::new(VecModel::from(options)));
            state.set_choice_index(0);
            state.set_choice_open(true);
        },
    );
}

/// Imports the modpack the prompt named as a new instance, then opens it.
///
/// The pack is either fetched from the chosen source or read off disk, depending on which
/// button opened the prompt. Everything after the fetch is the same, so both go through one
/// job and one report.
fn install_pack(bridge: &Bridge, shared: &Shared, name: &str) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<BrowserState>();
    state.set_name_open(false);
    let Some(pack) = shared.take_pending_pack() else {
        return;
    };
    let source = match &pack {
        // A file names its own source inside the archive; only a fetch needs one here.
        Pack::File(_) => None,
        Pack::Project(_) => match shared.source_at(state.get_source_index()) {
            Some(source) => Some(source),
            None => {
                state.set_status("No content source is enabled".into());
                return;
            }
        },
    };

    state.set_loading(true);
    state.set_status("Installing the modpack…".into());
    run_reporting(
        bridge,
        "Install modpack",
        move |launcher| match (&pack, source) {
            (Pack::File(path), _) => launcher.import_modpack_file(path, Some(name)),
            (Pack::Project(project), Some(source)) => {
                launcher.import_modpack(source, project, None, Some(name))
            }
            // Refused above: a project with no source never reaches the job.
            (Pack::Project(_), None) => {
                Err(std::io::Error::other("no content source is enabled").into())
            }
        },
        move |window, outcome| {
            let state = window.global::<BrowserState>();
            if !outcome.manual.is_empty() {
                warn(window, &manual_note(outcome.manual.len()));
            }
            state.set_status(format!("installed {} file(s)", outcome.installed).into());
            state.set_pack_path("".into());
            // The instance list was read before this import, so it does not know about the
            // instance the import just made. Nothing else re-reads it: the list reloads on a
            // create and on a delete of its own, and neither happened here.
            window.global::<crate::InstancesState>().invoke_refresh();
            // The new instance is what the user asked for, so the shell moves to it.
            let app = window.global::<App>();
            app.set_current_slug(outcome.instance.slug.as_str().into());
            app.set_screen(Screen::Instance);
        },
    );
}

/// The name an archive suggests for the instance it becomes.
///
/// The file stem, with nothing else read: the pack's own manifest names it too, but that is
/// inside the zip, and this only has to fill a field the user can still change.
pub fn pack_name(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string()
}

/// The content kinds one source can serve, in ComboBox order.
///
/// `modpack` is offered by both, even though neither source searches for one: the screen
/// turns that kind into an install-by-id panel, because a modpack becomes its own instance.
pub fn kinds_for(source: SourceId) -> Vec<&'static str> {
    match source {
        // Modrinth has no `world` project type.
        SourceId::Modrinth => vec!["mod", "resourcepack", "shader", "datapack", MODPACK],
        SourceId::CurseForge => vec![
            "mod",
            "resourcepack",
            "shader",
            "datapack",
            "world",
            MODPACK,
        ],
    }
}

/// Whether the source has hits past the page that just came back.
///
/// A source that reports a total is believed. One that reports zero — CurseForge pagination
/// leaves it out on some responses — falls back to "a full page probably has more behind it".
pub fn page_bounds(page: i32, page_size: i32, hits_len: usize, total: u64) -> bool {
    let page_size = page_size.max(1);
    if total == 0 {
        return hits_len as i32 >= page_size;
    }
    let offset = page.max(0) as u64 * page_size as u64;
    total > offset + hits_len as u64
}

/// Whether a row has to name a world before it can be added.
///
/// The row's own kind decides, never the Type selector: a data pack found before the selector
/// moved is still a data pack, and adding it without a world would drop it in `mods/`.
pub fn needs_world(row_kind: &str) -> bool {
    ContentKind::parse(row_kind) == Some(ContentKind::DataPack)
}

/// Builds the request one Add sends.
///
/// `version` is always `None`: the browser installs the newest compatible version, and
/// pinning one is the CLI's job.
pub fn add_request(
    source: SourceId,
    project: &str,
    kind: Option<ContentKind>,
    world: Option<String>,
) -> AddRequest {
    AddRequest {
        source,
        project: project.to_string(),
        version: None,
        kind,
        world,
    }
}

/// The loader slug at one ComboBox row. An index outside the list reads as "do not filter".
pub fn loader_option_at(index: i32) -> &'static str {
    usize::try_from(index)
        .ok()
        .and_then(|index| LOADER_OPTIONS.get(index))
        .copied()
        .unwrap_or(ANY_LOADER)
}

/// The ComboBox row for one loader slug. An unknown slug is the `any` row.
pub fn loader_option_index(slug: &str) -> i32 {
    LOADER_OPTIONS
        .iter()
        .position(|option| *option == slug)
        .unwrap_or(0) as i32
}

/// Reads the loader filter. `any`, an empty field, and an unknown name all mean "do not
/// filter", which is what the source sees as `None`.
pub fn parse_loader(text: &str) -> Option<Loader> {
    match text.trim() {
        "fabric" => Some(Loader::Fabric),
        "quilt" => Some(Loader::Quilt),
        "forge" => Some(Loader::Forge),
        "neoforge" => Some(Loader::NeoForge),
        _ => None,
    }
}

/// The toast shown when an add left files for the user to fetch by hand.
pub fn manual_note(count: usize) -> String {
    format!("{count} file(s) need a manual download; see the instance's Content tab")
}

/// The toast shown when a dependency wanted another version of an installed project.
///
/// One conflict names the project; more than one is counted, since a toast has one line.
pub fn conflict_note(conflicts: &[DependencyConflict]) -> String {
    match conflicts {
        [one] => format!(
            "Kept {} at {}; {} wanted {}",
            one.title, one.installed_version_id, one.wanted_by, one.wanted_version_id
        ),
        many => format!(
            "Kept the installed version of {} mod(s) a dependency pinned",
            many.len()
        ),
    }
}

/// The status line for a page of hits.
pub fn result_status(shown: usize, total: u64) -> String {
    match (shown, total) {
        (0, _) => "No results".to_string(),
        (shown, 0) => format!("{shown} result(s)"),
        (shown, total) => format!("{shown} of {total} result(s)"),
    }
}

/// The status line for the sources that are enabled.
pub fn source_status(sources: &[SourceId]) -> String {
    if sources.is_empty() {
        return "No content source is enabled".to_string();
    }
    if !sources.contains(&SourceId::CurseForge) {
        return NO_CURSEFORGE.to_string();
    }
    "Ready to search".to_string()
}

/// Pulls an index back into a list of `len` entries. An empty list has no index, which is -1.
pub fn clamp_index(index: i32, len: usize) -> i32 {
    if len == 0 {
        return -1;
    }
    index.clamp(0, len as i32 - 1)
}

/// The instance's slug, name, and filters, as one possible target.
fn target(instance: &Instance) -> Target {
    Target {
        slug: instance.slug.clone(),
        name: instance.config.name.clone(),
        minecraft: instance.config.minecraft.clone(),
        loader: match instance.config.loader {
            Loader::None => ANY_LOADER.to_string(),
            loader => loader.to_string(),
        },
    }
}

/// Points the Add button at one instance and prefills the filters from it.
fn apply_target(state: &BrowserState<'_>, target: Option<&Target>) {
    let Some(target) = target else {
        state.set_target_slug("".into());
        state.set_target_name("".into());
        return;
    };
    state.set_target_slug(target.slug.as_str().into());
    state.set_target_name(target.name.as_str().into());
    state.set_minecraft(target.minecraft.as_str().into());
    set_loader_filter(state, &target.loader);
}

/// Writes the loader filter and the ComboBox row that shows it, so the two never disagree.
fn set_loader_filter(state: &BrowserState<'_>, slug: &str) {
    let index = loader_option_index(slug);
    state.set_loader_index(index);
    state.set_loader(loader_option_at(index).into());
}

/// Rebuilds the kind ComboBox for one source, keeping the picked kind in range.
fn set_kinds(state: &BrowserState<'_>, source: SourceId) {
    let kinds = kinds_for(source);
    let labels: Vec<SharedString> = kinds.iter().map(|kind| (*kind).into()).collect();
    let index = clamp_index(state.get_kind_index(), labels.len());
    state.set_kind_labels(ModelRc::new(VecModel::from(labels)));
    state.set_kind_index(index);
}

/// Drops the page on screen and goes back to page one.
///
/// A hit found under one source or kind must not stay on screen under another: its Add would
/// still work, but the list would claim results the filters did not ask for.
fn clear_rows(state: &BrowserState<'_>) {
    state.set_page(0);
    state.set_has_more(false);
    state.set_rows(ModelRc::new(VecModel::from(Vec::<SearchRow>::new())));
    state.set_pack_rows(ModelRc::new(VecModel::from(Vec::<SearchRow>::new())));
}

/// The kind the filters name, or an empty string when the source offers none.
fn kind_label(state: &BrowserState<'_>) -> String {
    let labels = state.get_kind_labels();
    usize::try_from(state.get_kind_index())
        .ok()
        .and_then(|index| labels.row_data(index))
        .map(|label| label.to_string())
        .unwrap_or_default()
}

/// The page the screen is on.
fn page_of(bridge: &Bridge) -> i32 {
    match bridge.weak().upgrade() {
        Some(window) => window.global::<BrowserState>().get_page().max(0),
        None => 0,
    }
}

/// Moves the screen to a page.
fn set_page(bridge: &Bridge, page: i32) {
    if let Some(window) = bridge.weak().upgrade() {
        window.global::<BrowserState>().set_page(page.max(0));
    }
}

/// A trimmed field, or `None` when the user left it empty.
fn some_text(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Opens the shell's error dialog with a message of our own.
///
/// [`crate::bridge::Bridge::show_error`] needs a `gcl_core::Error`; this is for the cases the browser refuses before
/// core is ever called, such as a data pack with no world to go into.
fn error_dialog(window: &AppWindow, title: &str, text: &str) {
    let app = window.global::<App>();
    app.set_error_title(title.into());
    app.set_error_text(text.into());
    app.set_error_open(true);
}

/// Fetches and decodes every row's icon on this page, one job for the whole page rather than
/// one thread per row.
///
/// Runs sequentially inside that one job: `Launcher::fetch_icon` is already single-flight per
/// URL and the icon cache makes a repeat visit to the same page free, so nothing here needs
/// its own concurrency. The bytes are read and decoded off the UI thread; only the final
/// `slint::Image` is built in the `done` closure, on the UI thread, per the `slint-ui` skill.
/// A row whose icon fails to fetch or decode is left with its placeholder tile; the loop does
/// not stop for it.
fn fetch_icons(bridge: &Bridge, shared: &Shared, generation: u64, hits: &[SearchHit]) {
    let urls: Vec<(usize, String)> = hits
        .iter()
        .enumerate()
        .filter_map(|(index, hit)| hit.icon_url.clone().map(|url| (index, url)))
        .collect();
    if urls.is_empty() {
        return;
    }
    let shared = shared.clone();
    bridge.run(
        "Search icons",
        move |launcher| {
            let decoded: Vec<DecodedRowIcon> = urls
                .into_iter()
                .map(|(index, url)| DecodedRowIcon {
                    index,
                    icon: launcher
                        .fetch_icon(&url)
                        .ok()
                        .and_then(|path| std::fs::read(path).ok())
                        .and_then(|bytes| decode_icon(&bytes).ok())
                        .map(DecodedIcon::from),
                })
                .collect();
            Ok(decoded)
        },
        move |window, decoded: Vec<DecodedRowIcon>| {
            // A newer search has replaced these rows; this page's icons are moot.
            if shared.icon_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let state = window.global::<BrowserState>();
            let mut rows: Vec<SearchRow> = state.get_rows().iter().collect();
            for decoded in decoded {
                let (Some(row), Some(icon)) = (rows.get_mut(decoded.index), decoded.icon) else {
                    continue;
                };
                let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                    &icon.pixels,
                    icon.width,
                    icon.height,
                );
                row.icon = slint::Image::from_rgba8(buffer);
            }
            state.set_rows(ModelRc::new(VecModel::from(rows)));
        },
    );
}

/// One search row's decoded icon, or none when it has no icon or the fetch or decode failed.
struct DecodedRowIcon {
    /// Position of the row this icon belongs to, in the page that was on screen when the
    /// fetch started.
    index: usize,
    icon: Option<DecodedIcon>,
}

/// Raw RGBA8 pixels and dimensions for one icon, decoded off the UI thread.
struct DecodedIcon {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl From<(u32, u32, Vec<u8>)> for DecodedIcon {
    fn from((width, height, pixels): (u32, u32, Vec<u8>)) -> Self {
        Self {
            width,
            height,
            pixels,
        }
    }
}

/// Runs a launcher call and clears `loading` however it ends.
///
/// [`Bridge::run_with_error`] opens the error dialog and always calls back, so this only has
/// to drop the flag the screen was left on and say which step failed.
fn run_reporting<T: Send + 'static>(
    bridge: &Bridge,
    label: &'static str,
    job: impl FnOnce(&gcl_core::Launcher) -> Result<T, gcl_core::Error> + Send + 'static,
    done: impl FnOnce(&AppWindow, T) + Send + 'static,
) {
    bridge.run_with_error(label, job, move |window, result| {
        let state = window.global::<BrowserState>();
        state.set_loading(false);
        match result {
            Ok(value) => done(window, value),
            Err(_) => state.set_status(format!("{label} failed").into()),
        }
    });
}

#[cfg(test)]
mod tests;
