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
use gcl_core::launcher::{InstallState, LatestVersion, VersionTarget};
use gcl_core::mojang::manifest::{ManifestEntry, VersionType};
use gcl_core::sources::{SearchHit, SearchQuery, SourceId};
use slint::{
    ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel,
};

use crate::bridge::{Bridge, warn};
use crate::models::{decode_icon, latest_row_fields, search_row};
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
    /// The version to install once a world is picked: `Some(latest_id)` when the world
    /// chooser was opened by Update, pinning the exact version that resolved for the
    /// target, the same as a plain Update on any other kind. `None` for a plain Add, which
    /// lets `content::add` pick the newest version on its own.
    version: Option<String>,
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
    /// Bumped by every `search` and every target change, so a latest-version batch that
    /// finishes after a newer one has started does not paint its rows over that newer
    /// batch's own rows.
    latest_generation: Arc<AtomicU64>,
    /// The hits the page on screen was built from, kept so a target change can re-run the
    /// latest-version job without searching again.
    last_hits: Arc<Mutex<Vec<SearchHit>>>,
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

    /// Remembers the hits the page on screen was built from.
    fn set_last_hits(&self, hits: Vec<SearchHit>) {
        *self.last_hits.lock().unwrap_or_else(|err| err.into_inner()) = hits;
    }

    /// The hits the page on screen was built from, empty before the first search.
    fn last_hits(&self) -> Vec<SearchHit> {
        self.last_hits
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
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
        state.on_set_minecraft(move |index| {
            let Some(window) = bridge.weak().upgrade() else {
                return;
            };
            let state = window.global::<BrowserState>();
            let options: Vec<String> = state
                .get_minecraft_options()
                .iter()
                .map(|o| o.to_string())
                .collect();
            state.set_minecraft_index(index);
            state.set_minecraft(minecraft_string_at(&options, index).into());
            set_page(&bridge, 0);
            search(&bridge, &shared);
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
            // The target names which instance's Minecraft version and loader the latest
            // job resolves against, so a new target re-runs it. The rows themselves are
            // unchanged, only what each one reports about them.
            refresh_latest(&bridge, &shared);
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
        state.on_update(move |project_id, kind, version_id| {
            // A data pack still needs a world before anything lands in it, Update no less
            // than Add: `pick_world` already checks `needs_world` and installs straight
            // away for every other kind, so routing every Update through it here is what
            // keeps the two buttons agreeing on the rule.
            pick_world(
                &bridge,
                &shared,
                project_id.as_str(),
                kind.as_str(),
                Some(version_id.to_string()),
            );
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_pick_world(move |project_id, kind| {
            pick_world(&bridge, &shared, project_id.as_str(), kind.as_str(), None)
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
                pending.version,
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
    let bridge_for_search = bridge.clone();
    let shared_for_search = shared.clone();
    let bridge_for_manifest = bridge.clone();
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
                let index = target_index_for(state.get_target_index(), &slug, &opened.targets);
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

            // The combo shows `["any", <prefilled version>]` at once; `load_minecraft_options`
            // fills the rest once the manifest job answers.
            let target_version = state.get_minecraft().to_string();
            let placeholder = minecraft_options(&[], some_text(&target_version).as_deref());
            let index = minecraft_index_for(&placeholder, &target_version);
            state.set_minecraft_options(ModelRc::new(VecModel::from(
                placeholder
                    .iter()
                    .map(|option| SharedString::from(option.as_str()))
                    .collect::<Vec<_>>(),
            )));
            state.set_minecraft_index(index);

            // A page of hits belongs to a search that ran. Coming back from the project
            // screen already has one on screen (`returning`, handled above); every other
            // open runs the current kind's search itself, with whatever query and filters
            // are already on screen (normally none), so the browser never opens empty.
            if !returning {
                set_page(&bridge_for_search, 0);
                search(&bridge_for_search, &shared_for_search);
            }
            load_minecraft_options(&bridge_for_manifest, target_version);
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
    // A modpack row carries no latest-version job of its own, and its hits are never what
    // `refresh_latest`/`refresh_row` re-run against: bumping `latest_generation` here
    // orphans a content search's latest-version job that is still in flight, so its late
    // answers do not paint over rows this page has since replaced, and clearing
    // `last_hits` stops a target change from re-running that job over hits that no longer
    // belong to the page on screen.
    shared.latest_generation.fetch_add(1, Ordering::SeqCst);
    shared.set_last_hits(Vec::new());
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
    // Stamped before the search runs, so an icon batch or a latest-version batch from an
    // older search that is still in flight when this page lands is recognized as stale and
    // drops its result instead of painting over these rows.
    let generation = shared.icon_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let latest_generation = shared.latest_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let bridge_for_icons = bridge.clone();
    let shared_for_icons = shared.clone();
    let bridge_for_latest = bridge.clone();
    let shared_for_latest = shared.clone();
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
            fetch_latest(
                &bridge_for_latest,
                &shared_for_latest,
                latest_generation,
                &found.hits,
            );
            shared_for_latest.set_last_hits(found.hits);
        },
    );
}

/// Adds one row, asking for a world first when the row is a data pack.
///
/// `kind` is the row's own kind, not the Type selector's: the selector can be changed after a
/// search, and the rows on screen keep the kind they were found with.
fn add(bridge: &Bridge, shared: &Shared, project_id: &str, kind: &str, world: Option<String>) {
    if world.is_none() && needs_world(kind) {
        pick_world(bridge, shared, project_id, kind, None);
        return;
    }
    install(
        bridge,
        shared,
        project_id,
        ContentKind::parse(kind),
        world,
        None,
    );
}

/// Installs one project into the chosen instance, into `world` when it is a data pack.
///
/// `version` pins the file Update sends: `Some(latest_id)`, straight from the row, so the
/// newest version resolved for the target is the one that replaces what is there. It is
/// `None` for a plain Add, which installs whatever `content::add` picks as newest on its
/// own. Either way, the row for `project_id` is refreshed on success, so its state catches
/// up with the file that just landed: an Add leaves the instance holding that project just
/// as much as an Update does, and a row still offering "Add" for a mod that is now
/// installed sends the next press into a no-op.
fn install(
    bridge: &Bridge,
    shared: &Shared,
    project_id: &str,
    kind: Option<ContentKind>,
    world: Option<String>,
    version: Option<String>,
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
    let request = add_request(source, project_id, kind, world, version);

    let refresh = (bridge.clone(), shared.clone(), project_id.to_string());
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
            let (bridge, shared, project_id) = refresh;
            refresh_row(&bridge, &shared, &project_id);
        },
    );
}

/// Asks which world a data pack goes into, then adds it. `version` is what the world
/// chooser's Accept pins for the install once a world is picked: `Some(latest_id)` when
/// Update opened the chooser, `None` for a plain Add.
///
/// Anything that is not a data pack is installed straight away, so the screen can send every
/// row through here without knowing the rule — Update included, so a data pack row's Update
/// asks for a world exactly the way its Add does.
fn pick_world(
    bridge: &Bridge,
    shared: &Shared,
    project_id: &str,
    kind: &str,
    version: Option<String>,
) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<BrowserState>();
    if !needs_world(kind) {
        install(
            bridge,
            shared,
            project_id,
            ContentKind::parse(kind),
            None,
            version,
        );
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
        version,
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

/// Builds the request one Add or Update sends.
///
/// `version` is `None` for a plain Add — the newest compatible version is `content::add`'s
/// own job to pick — and `Some(latest_id)` for Update, which pins the exact version the
/// latest-version job already resolved rather than asking `content::add` to pick again.
pub fn add_request(
    source: SourceId,
    project: &str,
    kind: Option<ContentKind>,
    world: Option<String>,
    version: Option<String>,
) -> AddRequest {
    AddRequest {
        source,
        project: project.to_string(),
        version,
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

/// What the Minecraft version ComboBox shows for "do not filter".
const ANY_MINECRAFT: &str = "any";

/// The Minecraft version ComboBox rows: `"any"` first, then every release in `entries`, in
/// the order the manifest already carries them (newest first). `target` is the version the
/// instance being searched for actually runs; when it names one `entries` does not (a
/// snapshot, or `entries` not loaded yet), it is inserted right after `"any"` so the row the
/// filters are prefilled with is always on the list.
pub fn minecraft_options(entries: &[ManifestEntry], target: Option<&str>) -> Vec<String> {
    let mut options = vec![ANY_MINECRAFT.to_string()];
    options.extend(
        entries
            .iter()
            .filter(|entry| entry.kind == VersionType::Release)
            .map(|entry| entry.id.clone()),
    );
    if let Some(version) = target
        && !version.is_empty()
        && !options.iter().any(|option| option == version)
    {
        options.insert(1, version.to_string());
    }
    options
}

/// The ComboBox row for one Minecraft version string (`""` is "any"). A version not on the
/// list — the manifest job has not answered yet, or the version has vanished from it — reads
/// as row 0, the same "any" fallback `loader_option_at` uses for an index outside its list.
pub fn minecraft_index_for(options: &[String], value: &str) -> i32 {
    if value.is_empty() {
        return 0;
    }
    options
        .iter()
        .position(|option| option == value)
        .map(|index| index as i32)
        .unwrap_or(0)
}

/// Makes sure the Minecraft ComboBox has a row for `version`, inserting it right after "any"
/// when it is missing, and answers with the row's index either way. A blank `version` ("any",
/// or a target with none set) is row 0 and never grows the list.
pub fn ensure_minecraft_option(options: &[String], version: &str) -> (Vec<String>, i32) {
    if version.is_empty() {
        return (options.to_vec(), 0);
    }
    if let Some(index) = options.iter().position(|option| option == version) {
        return (options.to_vec(), index as i32);
    }
    let mut updated = options.to_vec();
    let insert_at = if updated.is_empty() { 0 } else { 1 };
    updated.insert(insert_at, version.to_string());
    (updated, insert_at as i32)
}

/// The search string for one ComboBox row: `"any"` maps to `""`, the search filter's own
/// spelling of "do not filter"; every other row is its own version id.
pub fn minecraft_string_at(options: &[String], index: i32) -> String {
    usize::try_from(index)
        .ok()
        .and_then(|index| options.get(index))
        .map(|option| {
            if option == ANY_MINECRAFT {
                String::new()
            } else {
                option.clone()
            }
        })
        .unwrap_or_default()
}

/// Fills `minecraft_options` from the Mojang manifest, a `Bridge` job of its own so it never
/// holds up the search `open()` already ran with whatever the combo showed before it answers.
/// `target_version` is the target instance's own version, captured when `open()` ran, so the
/// snapshot-insertion rule above still applies once the full list is in.
fn load_minecraft_options(bridge: &Bridge, target_version: String) {
    bridge.run(
        "Load Minecraft versions",
        |launcher| launcher.list_versions(),
        move |window, manifest| {
            let state = window.global::<BrowserState>();
            let target = some_text(&target_version);
            let options = minecraft_options(&manifest.versions, target.as_deref());
            let labels: Vec<SharedString> = options
                .iter()
                .map(|option| option.as_str().into())
                .collect();
            // The model always lands first: `mc_combo`'s own `current-index <=>
            // BrowserState.minecraft_index` two-way binding runs `reset-current` on `changed
            // model`, so setting the index before the model would have it clamped against the
            // list this call is about to replace. And this reads `target_version` — the
            // version `open()` captured before this job was even started — not
            // `state.get_minecraft()`: the combo may have kept ticking through the debounce
            // window while the manifest job was in flight, and the job answering must not
            // stomp a pick the user already made.
            state.set_minecraft_options(ModelRc::new(VecModel::from(labels)));
            state.set_minecraft_index(minecraft_index_for(&options, &target_version));
        },
    );
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

/// The target ComboBox row `open()` should land on.
///
/// `current_slug` is `App.current_slug`, the instance the shell already has open — the one the
/// browser must default to when it is reached from the rail, not whichever row the ComboBox
/// happened to hold from a previous visit. Its position in `targets` wins whenever it names one
/// of them; an empty slug (nothing open) or a slug that has since vanished from the list falls
/// back to `previous`, clamped into range the same way every other ComboBox index is.
fn target_index_for(previous: i32, current_slug: &str, targets: &[Target]) -> i32 {
    if !current_slug.is_empty()
        && let Some(index) = targets
            .iter()
            .position(|target| target.slug == current_slug)
    {
        return index as i32;
    }
    clamp_index(previous.max(0), targets.len())
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
    set_minecraft_filter(state, &target.minecraft);
    set_loader_filter(state, &target.loader);
}

/// Writes the Minecraft filter and the ComboBox row that shows it, so the two never disagree.
///
/// `minecraft_options` only ever gained the version the target held when the browser first
/// opened: a target picked afterwards — through the ComboBox, or `open()` landing on a
/// different instance from the rail — can name a version the manifest job never inserted,
/// and the row that would show it does not exist yet. `ensure_minecraft_option` adds it, the
/// same way `minecraft_options` seeds the placeholder list in `open()`.
fn set_minecraft_filter(state: &BrowserState<'_>, version: &str) {
    let options: Vec<String> = state
        .get_minecraft_options()
        .iter()
        .map(|option| option.to_string())
        .collect();
    let (options, index) = ensure_minecraft_option(&options, version);
    state.set_minecraft_options(ModelRc::new(VecModel::from(
        options
            .iter()
            .map(|option| SharedString::from(option.as_str()))
            .collect::<Vec<_>>(),
    )));
    state.set_minecraft_index(index);
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
    // The target a stale page's rows were checked against goes with them; a new page's own
    // `fetch_latest` writes these fresh once it resolves.
    state.set_latest_minecraft("".into());
    state.set_latest_loader("".into());
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

/// The `VersionTarget` the latest-version job resolves against: the picked instance's own
/// Minecraft version and loader, read fresh through the launcher rather than trusted off
/// screen — the search filter fields can be edited after a target is picked, and this is
/// what an add would actually install for — or, with no instance picked, `fallback_minecraft`
/// and `fallback_loader`, which are the browser's own filters as they stood when the search
/// that found these hits ran.
fn resolve_target(
    launcher: &gcl_core::Launcher,
    target_slug: &str,
    fallback_minecraft: Option<String>,
    fallback_loader: Loader,
) -> VersionTarget {
    if !target_slug.is_empty()
        && let Ok(instance) = launcher.instances().get(target_slug)
    {
        return VersionTarget {
            minecraft: Some(instance.config.minecraft),
            loader: instance.config.loader,
        };
    }
    VersionTarget {
        minecraft: fallback_minecraft,
        loader: fallback_loader,
    }
}

/// Resolves every hit's newest version for the search target, then, when there is a target
/// instance, whether it is installed there and whether that copy is behind it. Paints each
/// row (found by `project_id`, so this also serves [`refresh_row`]'s one-row refresh) as its
/// own answer arrives, rather than holding the whole page back for the slowest hit.
///
/// Two threads, because one cannot do both halves. `latest_versions_each` calls back from
/// inside the launcher's runtime, and [`gcl_core::Launcher::install_state`] blocks on that
/// same runtime, so calling it from the callback would panic. The callback therefore only
/// sends the answer down a channel; the painter thread it is read on owns its own
/// `Arc<Launcher>`, asks for the install state there, and posts one
/// `upgrade_in_event_loop` per answer. The channel closes when `latest_versions_each`
/// returns and drops the callback, which is what ends the painter, and the job waits for it
/// so the "Latest versions" line is written once every row has been painted.
///
/// Every posted closure is guarded by `generation`, the same way `fetch_icons` is guarded by
/// `icon_generation`: a page that a newer search or a target change has since replaced must
/// not paint over it.
fn fetch_latest(bridge: &Bridge, shared: &Shared, generation: u64, hits: &[SearchHit]) {
    if hits.is_empty() {
        return;
    }
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<BrowserState>();
    let Some(source) = shared.source_at(state.get_source_index()) else {
        return;
    };
    let target_slug = state.get_target_slug().to_string();
    let fallback_minecraft = some_text(state.get_minecraft().as_str());
    let fallback_loader = parse_loader(state.get_loader().as_str()).unwrap_or(Loader::None);
    let hits = hits.to_vec();

    let painter_launcher = Arc::clone(bridge.launcher());
    let weak = bridge.weak().clone();
    let shared = shared.clone();
    let painter_slug = target_slug.clone();
    bridge.run(
        "Latest versions",
        move |launcher| {
            let target =
                resolve_target(launcher, &target_slug, fallback_minecraft, fallback_loader);
            // `target_label()` (`browser.slint`) reads what this job actually resolved
            // against, not the live filter fields, so a row's state line stays correct
            // even after the user keeps editing `minecraft`/`loader` post-search. Posted
            // once, ahead of the per-row answers below, and guarded by `generation` the
            // same way they are.
            {
                let weak = weak.clone();
                let shared = shared.clone();
                let target = target.clone();
                let _ = weak.upgrade_in_event_loop(move |window| {
                    if shared.latest_generation.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    let state = window.global::<BrowserState>();
                    state.set_latest_minecraft(target.minecraft.clone().unwrap_or_default().into());
                    state.set_latest_loader(loader_label(target.loader).into());
                });
            }
            let (answers, painted) = std::sync::mpsc::channel::<LatestVersion>();
            let painter = {
                let target = target.clone();
                std::thread::spawn(move || {
                    for answer in painted {
                        paint_answer(
                            &painter_launcher,
                            &weak,
                            &shared,
                            generation,
                            source,
                            &painter_slug,
                            &target,
                            answer,
                        );
                    }
                })
            };
            let resolved = launcher.latest_versions_each(source, &hits, &target, move |answer| {
                // A closed channel means the window went away; the answers are moot.
                let _ = answers.send(answer);
            });
            // The callback, and with it the sending half, is dropped by the call above, so
            // the painter's loop has ended or is about to.
            if painter.join().is_err() {
                tracing::warn!("the latest-version painter thread ended badly");
            }
            resolved
        },
        |_window, ()| {},
    );
}

/// The loader label `latest_minecraft`/`latest_loader` and `target()` (`browser.rs`) both
/// write: `Loader::None` reads as [`ANY_LOADER`], every other loader as its own `Display`.
fn loader_label(loader: Loader) -> String {
    match loader {
        Loader::None => ANY_LOADER.to_string(),
        loader => loader.to_string(),
    }
}

/// Asks for one answer's install state and paints it onto its row.
///
/// Runs on the painter thread, off both the UI thread and the launcher's runtime. Checked
/// against `generation` before the blocking lookup below, not only after it: a search or a
/// target change that has already moved on must not pay for an install-state lookup whose
/// answer would only be thrown away. The install state is only asked for when a target
/// instance is picked and the hit resolved to a version; with no version there is nothing to
/// compare against, and that lookup itself failing — the instance vanished, a network call
/// errored — reads the same way: both are
/// [`gcl_core::launcher::InstallState::Unknown`], which the row shows as "could not check"
/// rather than as "not installed", a claim this code never actually checked.
#[allow(clippy::too_many_arguments)]
fn paint_answer(
    launcher: &gcl_core::Launcher,
    weak: &slint::Weak<AppWindow>,
    shared: &Shared,
    generation: u64,
    source: SourceId,
    target_slug: &str,
    target: &VersionTarget,
    answer: LatestVersion,
) {
    if shared.latest_generation.load(Ordering::SeqCst) != generation {
        return;
    }
    let install_state = match (target_slug.is_empty(), answer.version.as_ref()) {
        (false, Some(version)) => Some(
            launcher
                .install_state(target_slug, source, &answer.project_id, target, version)
                .unwrap_or(InstallState::Unknown),
        ),
        _ => None,
    };
    let fields = latest_row_fields(&answer, install_state.as_ref());
    let project_id = answer.project_id;
    let shared = shared.clone();
    let _ = weak.upgrade_in_event_loop(move |window| {
        // A newer search or target change has replaced these rows; this answer is moot.
        if shared.latest_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        set_row_fields(&window, &project_id, fields);
    });
}

/// Writes one row's four latest-version fields in place, leaving every other row alone.
///
/// `set_row_data` on the model the screen is already showing, rather than a fresh model
/// through `set_rows`: a whole new model per answer would redraw and re-lay-out the list
/// twenty times a page, and the row's own icon, which a second job is filling in at the same
/// time, would be read from a copy taken before that job's last write.
fn set_row_fields(
    window: &AppWindow,
    project_id: &str,
    (number, id, installed_number, row_state): (String, String, String, String),
) {
    let rows = window.global::<BrowserState>().get_rows();
    let Some((index, mut row)) = (0..rows.row_count())
        .filter_map(|index| rows.row_data(index).map(|row| (index, row)))
        .find(|(_, row)| row.project_id == project_id)
    else {
        return;
    };
    row.latest_number = number.into();
    row.latest_id = id.into();
    row.installed_number = installed_number.into();
    row.state = row_state.into();
    rows.set_row_data(index, row);
}

/// Re-runs the latest-version job for the page already on screen: the hits stay the same,
/// but what each one's install state compares against does not, so this always bumps
/// `latest_generation` rather than reusing the one the last search stamped.
///
/// Every row goes back to "Checking…" first. What is on screen was answered about the
/// instance the user has just left — a row reading "Installed 0.6.0" for an instance that
/// has none of it is worse than a row saying it does not know yet — and the new target may
/// run another Minecraft version or another loader, so the latest version itself can change
/// too, not only the install state.
fn refresh_latest(bridge: &Bridge, shared: &Shared) {
    let hits = shared.last_hits();
    if hits.is_empty() {
        return;
    }
    if let Some(window) = bridge.weak().upgrade() {
        clear_row_states(&window);
    }
    let generation = shared.latest_generation.fetch_add(1, Ordering::SeqCst) + 1;
    fetch_latest(bridge, shared, generation, &hits);
}

/// Puts every row's four latest-version fields back to what `models::search_row` starts them
/// at, so the page reads "Checking…" again until the answers for the new target land.
fn clear_row_states(window: &AppWindow) {
    let rows = window.global::<BrowserState>().get_rows();
    for index in 0..rows.row_count() {
        let Some(mut row) = rows.row_data(index) else {
            continue;
        };
        row.latest_number = SharedString::new();
        row.latest_id = SharedString::new();
        row.installed_number = SharedString::new();
        row.state = "unknown".into();
        rows.set_row_data(index, row);
    }
}

/// Refreshes one row after Update has installed a new file for it. Runs on the generation
/// the page's own latest-version job already answered on: a search that has since moved on
/// guards this exactly the way it would guard that job's own late answer.
fn refresh_row(bridge: &Bridge, shared: &Shared, project_id: &str) {
    let hits = shared.last_hits();
    let Some(hit) = hits.into_iter().find(|hit| hit.project_id == project_id) else {
        return;
    };
    let generation = shared.latest_generation.load(Ordering::SeqCst);
    fetch_latest(bridge, shared, generation, std::slice::from_ref(&hit));
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
            // `set_row_data` on the model already on screen, one row at a time, rather than
            // a fresh model through `set_rows`: a whole new model here would redraw and
            // re-lay-out the whole page for every icon batch, and any row a concurrent
            // latest-version answer (`set_row_fields`) has already patched would be
            // overwritten by a copy of the rows taken before that write landed.
            let rows = window.global::<BrowserState>().get_rows();
            for decoded in decoded {
                let (Some(mut row), Some(icon)) = (rows.row_data(decoded.index), decoded.icon)
                else {
                    continue;
                };
                let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                    &icon.pixels,
                    icon.width,
                    icon.height,
                );
                row.icon = slint::Image::from_rgba8(buffer);
                rows.set_row_data(decoded.index, row);
            }
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
