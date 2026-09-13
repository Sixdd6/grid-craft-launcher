//! Wires the project details screen: description, versions, and installing a chosen version.
//!
//! Every call into `gcl-core` runs off the UI thread through [`Bridge`]. The screen is pure
//! layout; this module owns the `ProjectState` global that feeds it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gcl_core::content::{AddRequest, compatible_loaders};
use gcl_core::instances::model::{ContentKind, Loader};
use gcl_core::launcher::ProjectDetails;
use gcl_core::sources::richtext::Block as CoreBlock;
use gcl_core::sources::{GalleryImage, ReleaseKind, SourceId, Version, VersionFilter};
use slint::{ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::bridge::Bridge;
use crate::models::{decode_description_image, gallery_row, short_time};
use crate::screens::browser::conflict_note;
use crate::{AppWindow, Block, ContentVersionRow, GalleryImageRow, ProjectState, Screen};

/// How many images one description (or one changelog) fetches, in the order its blocks name
/// them. The rest keep the `[Image: alt]` failure text forever, per the description-rendering
/// spec: a pathological description is a degrade to accept, not a bug to chase further.
const MAX_IMAGES: usize = 20;

/// A block's (or a gallery row's) index and the url to fetch for it, in the order
/// [`prepare_images`]/[`prepare_gallery`] found them.
type ImagePairs = Vec<(usize, String)>;

/// Which block list an image-fetch job's completions land in.
///
/// The Description tab and the Notes modal load their images independently, each behind its
/// own generation counter (see [`Shared`]), so closing the modal and opening a different
/// version's notes cannot race stale pixels onto the one that replaced it, and neither can
/// navigating to a different project while the description's own fetch is still in flight.
#[derive(Clone, Copy)]
enum ImageTarget {
    Description,
    Notes,
}

impl ImageTarget {
    fn blocks(self, state: &ProjectState<'_>) -> ModelRc<Block> {
        match self {
            ImageTarget::Description => state.get_blocks(),
            ImageTarget::Notes => state.get_notes_blocks(),
        }
    }
}

/// What an `open` or `open_notes` call threads through to its image-fetch job.
///
/// Cloning shares the same counters, which is what lets `wire`'s closures and the job each
/// bump or read the generation that call started.
#[derive(Clone, Default)]
struct Shared {
    /// Bumped once per `open()`. A description image-fetch job checks this before it starts a
    /// fetch and again before it paints one, so a project opened after this one dropped its
    /// stale result instead of overwriting the new project's blocks.
    image_generation: Arc<AtomicU64>,
    /// The Notes modal's own counter, bumped once per `open_notes()`: a changelog's images load
    /// independently of the description's, so the two must not share one counter.
    notes_generation: Arc<AtomicU64>,
    /// Bumped once per `open()`, guarding the Gallery tab's own thumbnail fetch: it loads
    /// independently of the description's and the Notes modal's images, so a project opened
    /// after this one, or a thumbnail batch left over from it, cannot paint over the new
    /// project's grid.
    gallery_generation: Arc<AtomicU64>,
    /// Bumped once per `open_viewer()`/`close_viewer()`/`viewer_next()`/`viewer_prev()`: the
    /// viewer's own full-size fetch is a second, independent job from the thumbnail grid's,
    /// per the `slint-ui` skill's generation-counter pattern, so a fast Next/Prev discards a
    /// slow fetch behind it rather than painting it over the image the user has since moved
    /// to.
    viewer_generation: Arc<AtomicU64>,
    /// The full gallery this project's last `open()` loaded, kept so `open_viewer` (and
    /// `viewer_next`/`viewer_prev`) can fetch a full-size image by index without threading the
    /// whole list back out of `ProjectState.gallery`, which only carries the thumbnail-sized
    /// `GalleryImageRow` shape.
    gallery: Arc<std::sync::Mutex<Vec<GalleryImage>>>,
}

/// Binds the `ProjectState` global to the launcher.
///
/// Nothing is loaded here: a screen that wants to show a project calls `ProjectState.open`
/// with the source, the project id, the instance a version installs into, and the screen to
/// return to. There is no "screen was shown" watcher, unlike the other screens, because this
/// one needs those four things before it can load anything.
pub fn wire(window: &AppWindow, bridge: &Bridge) {
    let state = window.global::<ProjectState>();
    let shared = Shared::default();

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_open(
            move |source, project_id, target_slug, return_to, downloads, author| {
                open(
                    &bridge,
                    &shared,
                    source.to_string(),
                    project_id.to_string(),
                    target_slug.to_string(),
                    return_to,
                    downloads.to_string(),
                    author.to_string(),
                );
            },
        );
    }

    {
        let bridge = bridge.clone();
        state.on_back(move || {
            if let Some(window) = bridge.weak().upgrade() {
                let return_to = window.global::<ProjectState>().get_return_to();
                if return_to == Screen::Browser {
                    // The browser's own `open()` runs on every navigation to it; this says
                    // the page of hits it is about to see is the one the user searched for,
                    // not a stale one to clear.
                    window.global::<crate::BrowserState>().set_returning(true);
                }
                window.global::<crate::App>().set_screen(return_to);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_toggle_show_all(move || reload_versions(&bridge, false));
    }

    {
        let bridge = bridge.clone();
        state.on_install(move |version_id| install(&bridge, version_id.to_string()));
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_open_notes(move |version_id, number| {
            open_notes(&bridge, &shared, version_id.to_string(), number.to_string());
        });
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_close_notes(move || close_notes(&bridge, &shared));
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_open_viewer(move |index| open_viewer(&bridge, &shared, index as usize));
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_close_viewer(move || close_viewer(&bridge, &shared));
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_viewer_next(move || step_viewer(&bridge, &shared, 1));
    }

    {
        let bridge = bridge.clone();
        let shared = shared.clone();
        state.on_viewer_prev(move || step_viewer(&bridge, &shared, -1));
    }
}

/// The target instance's Minecraft version and loader, carried alongside a version list so a
/// row can be marked with whatever it would take to run it. `None` when there is no target.
type InstanceContext = (String, Loader);

/// Everything one `open` reads off disk, as the UI-thread half shows it.
struct Opened {
    details: ProjectDetails,
    versions: Vec<Version>,
    installed_version_id: Option<String>,
    instance: Option<InstanceContext>,
}

/// A versions-only reload's result: no project details, since the project itself has not
/// changed between a filter toggle or a just-finished install.
struct VersionsLoaded {
    versions: Vec<Version>,
    installed_version_id: Option<String>,
    instance: Option<InstanceContext>,
}

/// Loads a project's description and its versions, filtered by the target instance unless
/// the screen already asked to see them all.
#[allow(clippy::too_many_arguments)]
fn open(
    bridge: &Bridge,
    shared: &Shared,
    source: String,
    project_id: String,
    target_slug: String,
    return_to: Screen,
    downloads: String,
    author: String,
) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<ProjectState>();

    // Every property this screen owns is set here, the empty case included, so the screen
    // never shows the previous project while this one loads. `downloads` and `author` are
    // the exceptions carried in from the caller rather than reset to empty: `Project` has
    // no count and no author field of its own, so a search row's already-formatted values
    // are what the header shows, and `apply` below never overwrites them.
    state.set_source(source.as_str().into());
    state.set_project_id(project_id.as_str().into());
    state.set_target_slug(target_slug.as_str().into());
    state.set_return_to(return_to);
    state.set_tab(0);
    state.set_title("".into());
    state.set_author(author.as_str().into());
    state.set_kind("".into());
    state.set_downloads(downloads.as_str().into());
    state.set_page_url("".into());
    state.set_blocks(ModelRc::new(VecModel::from(Vec::<Block>::new())));
    state.set_versions(ModelRc::new(
        VecModel::from(Vec::<ContentVersionRow>::new()),
    ));
    state.set_show_all_versions(false);
    state.set_status("".into());
    // The Notes modal is about a version of whatever project was open before; a new project
    // has none of those versions loaded yet, so any modal left up from the last one is closed
    // rather than shown over the wrong project.
    state.set_notes_open(false);
    state.set_notes_title("".into());
    state.set_notes_blocks(ModelRc::new(VecModel::from(Vec::<Block>::new())));
    state.set_notes_loading(false);
    // The Gallery tab and its viewer are about this project's own images; a new project has
    // none of those loaded yet, and the viewer (if it was open) was showing a picture of
    // whatever project was open before.
    state.set_gallery(ModelRc::new(VecModel::from(Vec::<GalleryImageRow>::new())));
    state.set_viewer_open(false);
    state.set_viewer_index(-1);
    state.set_viewer_title("".into());
    state.set_viewer_description("".into());
    state.set_viewer_image(slint::Image::default());
    state.set_viewer_image_state("".into());
    *shared.gallery.lock().expect("gallery lock") = Vec::new();

    let Some(source_id) = SourceId::parse(&source) else {
        state.set_status(format!("Unknown content source: {source}").into());
        return;
    };

    state.set_loading(true);
    let show_all = false;
    // Stamped before the job runs, so an image batch from a project this open replaced is
    // recognized as stale and drops its result instead of painting over these blocks.
    let generation = shared.image_generation.fetch_add(1, Ordering::SeqCst) + 1;
    // The Notes modal's own counter is bumped too: closing it above does not race a fetch job
    // that was already in flight for the version it was showing.
    shared.notes_generation.fetch_add(1, Ordering::SeqCst);
    // The gallery grid's own counter, and the viewer's: a thumbnail batch or a full-size
    // fetch left over from the project this open replaced must not paint over the new
    // project's grid or a viewer the user has since closed.
    let gallery_generation = shared.gallery_generation.fetch_add(1, Ordering::SeqCst) + 1;
    shared.viewer_generation.fetch_add(1, Ordering::SeqCst);
    let bridge_for_images = bridge.clone();
    let bridge_for_gallery = bridge.clone();
    let shared_for_gallery = shared.clone();
    run_reporting(
        bridge,
        bridge_for_images,
        Arc::clone(&shared.image_generation),
        generation,
        bridge_for_gallery,
        shared_for_gallery,
        gallery_generation,
        "Project details",
        false,
        move |launcher| load(launcher, source_id, &project_id, &target_slug, show_all),
    );
}

/// Reloads only the versions tab, keeping the description on screen: neither a filter toggle
/// nor a just-finished install changes what the project is, only what its version list shows.
/// It never fetches project details again; `kind` comes from the description already on
/// screen, set the last time the project itself was loaded.
///
/// `preserve_status` keeps whatever status line is already up — the install path wrote
/// "Installed 0.6.13" or a conflict line, and a generic "N version(s)" would overwrite it.
/// The checkbox path has no such message to protect and clears `false`.
fn reload_versions(bridge: &Bridge, preserve_status: bool) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<ProjectState>();
    let source = state.get_source().to_string();
    let project_id = state.get_project_id().to_string();
    let target_slug = state.get_target_slug().to_string();
    let show_all = state.get_show_all_versions();
    let kind = ContentKind::parse(state.get_kind().as_str()).unwrap_or_default();
    let Some(source_id) = SourceId::parse(&source) else {
        return;
    };

    if !preserve_status {
        state.set_loading(true);
    }
    run_versions_reporting(
        bridge,
        "Project versions",
        preserve_status,
        move |launcher| {
            versions_and_installed(
                launcher,
                source_id,
                &project_id,
                &target_slug,
                kind,
                show_all,
            )
        },
    );
}

/// Fetches a project's details and its versions in one job.
fn load(
    launcher: &gcl_core::Launcher,
    source_id: SourceId,
    project_id: &str,
    target_slug: &str,
    show_all: bool,
) -> Result<Opened, gcl_core::Error> {
    let details = launcher.project_details(source_id, project_id)?;
    let kind = details.project.kind;
    // `project_id` is whatever the caller passed in — a search hit's id, or a slug typed by
    // hand — and a source may answer either one with its own canonical id. `details` just
    // resolved it, so the installed marker below is matched against that canonical id, the
    // same one `Launcher::add_content` records in `instance.toml`.
    let loaded = versions_and_installed(
        launcher,
        source_id,
        &details.project.id,
        target_slug,
        kind,
        show_all,
    )?;
    Ok(Opened {
        details,
        versions: loaded.versions,
        installed_version_id: loaded.installed_version_id,
        instance: loaded.instance,
    })
}

/// Lists a project's versions, filtered by the target instance's Minecraft version and, for
/// a mod, loader unless `show_all` is set or there is no target, and pairs them with that
/// instance's installed version for this project, if any.
fn versions_and_installed(
    launcher: &gcl_core::Launcher,
    source_id: SourceId,
    project_id: &str,
    target_slug: &str,
    kind: ContentKind,
    show_all: bool,
) -> Result<VersionsLoaded, gcl_core::Error> {
    if target_slug.is_empty() {
        let versions =
            launcher.project_versions(source_id, project_id, &VersionFilter::default())?;
        return Ok(VersionsLoaded {
            versions,
            installed_version_id: None,
            instance: None,
        });
    }

    let instance = launcher.instances().get(target_slug)?;
    let minecraft = instance.config.minecraft.clone();
    let loader = instance.config.loader;
    let filter = if show_all {
        VersionFilter::default()
    } else {
        let loaders = if kind == ContentKind::Mod {
            compatible_loaders(loader, &minecraft)
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            Vec::new()
        };
        VersionFilter {
            minecraft: Some(minecraft.clone()),
            loaders,
        }
    };
    let versions = launcher.project_versions(source_id, project_id, &filter)?;

    let source_label = source_id.to_string();
    let installed_version_id = launcher
        .list_content(target_slug)?
        .into_iter()
        .find(|entry| entry.source == source_label && entry.project_id == project_id)
        .map(|entry| entry.version_id);
    Ok(VersionsLoaded {
        versions,
        installed_version_id,
        instance: Some((minecraft, loader)),
    })
}

/// Installs one version into the target instance, then reloads the versions and, when that
/// instance's detail screen is the one on screen, its content list too.
fn install(bridge: &Bridge, version_id: String) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<ProjectState>();
    let target_slug = state.get_target_slug().to_string();
    if target_slug.is_empty() {
        state.set_status("Pick a target instance in the browser first".into());
        return;
    }
    let Some(source_id) = SourceId::parse(state.get_source().as_str()) else {
        state.set_status(format!("Unknown content source: {}", state.get_source()).into());
        return;
    };
    let project_id = state.get_project_id().to_string();

    // The version's own number is what the status line names once the install lands; the
    // response only carries the file it wrote, not the number a person reads.
    let number = state
        .get_versions()
        .iter()
        .find(|row| row.id.as_str() == version_id)
        .map(|row| row.number.to_string())
        .unwrap_or_else(|| version_id.clone());

    let request = AddRequest {
        source: source_id,
        project: project_id,
        version: Some(version_id),
        kind: None,
        world: None,
    };

    state.set_loading(true);
    state.set_status("Installing\u{2026}".into());
    let slug_for_job = target_slug.clone();
    let bridge_for_reload = bridge.clone();
    bridge.run_with_error(
        "Install version",
        move |launcher| launcher.add_content(&slug_for_job, request),
        move |window, result| {
            let state = window.global::<ProjectState>();
            // `loading` stays true across the reload below: clearing it here would let the
            // Install buttons flash enabled on rows the reload is about to mark installed or
            // incompatible again.
            match result {
                Ok(outcome) => {
                    if !outcome.conflicts.is_empty() {
                        state.set_status(conflict_note(&outcome.conflicts).into());
                    } else if !outcome.installed.is_empty() {
                        state.set_status(format!("Installed {number}").into());
                    } else {
                        state.set_status("No changes".into());
                    }
                    if !outcome.manual.is_empty() {
                        crate::bridge::warn(
                            window,
                            &format!(
                                "{} file(s) need a manual download; see the instance's Content tab",
                                outcome.manual.len()
                            ),
                        );
                    }
                    let app = window.global::<crate::App>();
                    if app.get_current_slug().as_str() == target_slug
                        && app.get_screen() == Screen::Instance
                    {
                        window
                            .global::<crate::InstanceState>()
                            .invoke_load(target_slug.as_str().into());
                    }
                    reload_versions(&bridge_for_reload, true);
                }
                Err(_) => {
                    // No reload follows on this path, so this is the one place left to
                    // clear it.
                    state.set_loading(false);
                    state.set_status("Install version failed".into());
                }
            }
        },
    );
}

/// Runs a launcher call and shows the outcome, clearing `loading` however it ends.
///
/// `preserve_status` leaves `ProjectState.status` as it is on success, for a reload that
/// follows an install and must not overwrite the line the install itself just wrote.
///
/// `images_bridge` and `image_generation`/`generation` are what starts the description's own
/// image-fetch job once `apply` has written the blocks it describes: `apply` returns the
/// (already capped) list of urls to fetch, in block order. `gallery_bridge`/`shared_for_gallery`/
/// `gallery_generation` do the same for the Gallery tab's thumbnails, a separate job behind its
/// own counter.
#[allow(clippy::too_many_arguments)]
fn run_reporting(
    bridge: &Bridge,
    images_bridge: Bridge,
    image_generation: Arc<AtomicU64>,
    generation: u64,
    gallery_bridge: Bridge,
    shared_for_gallery: Shared,
    gallery_generation: u64,
    label: &'static str,
    preserve_status: bool,
    job: impl FnOnce(&gcl_core::Launcher) -> Result<Opened, gcl_core::Error> + Send + 'static,
) {
    bridge.run_with_error(label, job, move |window, result| {
        // A project opened after this call started bumped `image_generation` past what this
        // job was stamped with; painting its result now would put a stale project's title and
        // blocks over the one the user has since opened.
        if image_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        let state = window.global::<ProjectState>();
        state.set_loading(false);
        match result {
            Ok(opened) => {
                let (urls, gallery_urls) =
                    apply(window, opened, preserve_status, &shared_for_gallery);
                fetch_description_images(
                    &images_bridge,
                    ImageTarget::Description,
                    image_generation,
                    generation,
                    urls,
                );
                fetch_gallery_images(
                    &gallery_bridge,
                    Arc::clone(&shared_for_gallery.gallery_generation),
                    gallery_generation,
                    gallery_urls,
                );
            }
            Err(_) => state.set_status(format!("{label} failed").into()),
        }
    });
}

/// Runs a versions-only reload and shows the outcome, the same way `run_reporting` does for
/// a full load, but through `apply_versions`, which touches only `versions` and `status`.
fn run_versions_reporting(
    bridge: &Bridge,
    label: &'static str,
    preserve_status: bool,
    job: impl FnOnce(&gcl_core::Launcher) -> Result<VersionsLoaded, gcl_core::Error> + Send + 'static,
) {
    bridge.run_with_error(label, job, move |window, result| {
        let state = window.global::<ProjectState>();
        state.set_loading(false);
        match result {
            Ok(loaded) => apply_versions(window, loaded, preserve_status),
            Err(_) => state.set_status(format!("{label} failed").into()),
        }
    });
}

/// Fills every property `open` promised, from a successful full load. Returns the block index
/// and url of every image the caller's image-fetch job should fetch, in block order, already
/// capped at [`MAX_IMAGES`] and marked "loading" on the blocks that were just written.
fn apply(
    window: &AppWindow,
    opened: Opened,
    preserve_status: bool,
    shared_for_gallery: &Shared,
) -> (ImagePairs, ImagePairs) {
    let state = window.global::<ProjectState>();
    let project = opened.details.project;
    let kind = project.kind;
    // The canonical id, so a later `reload_versions` or `install` matches the same
    // installed entry this open just did, even when the caller opened with a slug.
    state.set_project_id(project.id.as_str().into());
    state.set_title(project.title.as_str().into());
    state.set_kind(project.kind.to_string().into());
    state.set_page_url(project.page_url.as_str().into());

    let mut blocks: Vec<Block> = opened.details.blocks.iter().map(block_row).collect();
    let pairs = prepare_images(&mut blocks, MAX_IMAGES);
    state.set_blocks(ModelRc::new(VecModel::from(blocks)));

    let mut gallery_rows: Vec<GalleryImageRow> = project.gallery.iter().map(gallery_row).collect();
    let gallery_pairs = prepare_gallery(&mut gallery_rows, MAX_IMAGES);
    state.set_gallery(ModelRc::new(VecModel::from(gallery_rows)));
    // Kept so `open_viewer` (and `viewer_next`/`viewer_prev`) can fetch a full-size image by
    // index: `GalleryImageRow` only carries the thumbnail-sized shape, not the full `url`.
    *shared_for_gallery.gallery.lock().expect("gallery lock") = project.gallery;

    let rows = build_version_rows(
        &opened.versions,
        opened.installed_version_id.as_deref(),
        opened.instance.as_ref(),
        kind,
    );
    state.set_versions(ModelRc::new(VecModel::from(rows)));

    if !preserve_status {
        if state.get_target_slug().is_empty() {
            state.set_status("Pick a target instance in the browser first".into());
        } else {
            state.set_status(format!("{} version(s)", opened.versions.len()).into());
        }
    }
    (pairs, gallery_pairs)
}

/// Touches only `versions` and `status`, from a successful versions-only reload. `kind` comes
/// from `ProjectState.kind`, still on screen from the last full load.
fn apply_versions(window: &AppWindow, loaded: VersionsLoaded, preserve_status: bool) {
    let state = window.global::<ProjectState>();
    let kind = ContentKind::parse(state.get_kind().as_str()).unwrap_or_default();
    let rows = build_version_rows(
        &loaded.versions,
        loaded.installed_version_id.as_deref(),
        loaded.instance.as_ref(),
        kind,
    );
    state.set_versions(ModelRc::new(VecModel::from(rows)));

    if preserve_status {
        return;
    }
    if state.get_target_slug().is_empty() {
        state.set_status("Pick a target instance in the browser first".into());
    } else {
        state.set_status(format!("{} version(s)", loaded.versions.len()).into());
    }
}

/// Opens the Notes modal for one version and fetches its changelog.
///
/// Every property the modal owns is set here, the empty case included: `notes_blocks` starts
/// empty and `notes_loading` starts true, so the modal never shows the previous version's
/// notes while this one loads.
fn open_notes(bridge: &Bridge, shared: &Shared, version_id: String, number: String) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<ProjectState>();
    let Some(source_id) = SourceId::parse(state.get_source().as_str()) else {
        return;
    };
    let project_id = state.get_project_id().to_string();

    state.set_notes_title(number.as_str().into());
    state.set_notes_blocks(ModelRc::new(VecModel::from(Vec::<Block>::new())));
    state.set_notes_status("".into());
    state.set_notes_loading(true);
    state.set_notes_open(true);

    // Stamped before the job runs, the same way `open`'s own counter is: a changelog fetched
    // for a version the user has since closed, or replaced by opening another one, is dropped
    // rather than painted.
    let generation = shared.notes_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let bridge_for_images = bridge.clone();
    let notes_generation = Arc::clone(&shared.notes_generation);
    bridge.run_with_error(
        "Version notes",
        move |launcher| launcher.version_notes(source_id, &project_id, &version_id),
        move |window, result| {
            // A version whose Notes button was pressed after this one, or a modal since
            // closed, bumped `notes_generation` past this job's stamp; painting now would
            // put a stale changelog's title and blocks over the one open now.
            if notes_generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let state = window.global::<ProjectState>();
            state.set_notes_loading(false);
            match result {
                Ok(core_blocks) => {
                    let mut blocks: Vec<Block> = core_blocks.iter().map(block_row).collect();
                    let pairs = prepare_images(&mut blocks, MAX_IMAGES);
                    state.set_notes_blocks(ModelRc::new(VecModel::from(blocks)));
                    fetch_description_images(
                        &bridge_for_images,
                        ImageTarget::Notes,
                        notes_generation,
                        generation,
                        pairs,
                    );
                }
                Err(err) => {
                    state.set_notes_blocks(ModelRc::new(VecModel::from(Vec::<Block>::new())));
                    state.set_notes_status(
                        format!("Could not load notes: {}", crate::bridge::error_chain(&err))
                            .into(),
                    );
                }
            }
        },
    );
}

/// Closes the Notes modal. Bumps its generation counter too, so a changelog fetch still in
/// flight for the version it was showing paints nothing after this, and clears the blocks so
/// the next version opened never shows a frame of the one before it.
fn close_notes(bridge: &Bridge, shared: &Shared) {
    shared.notes_generation.fetch_add(1, Ordering::SeqCst);
    if let Some(window) = bridge.weak().upgrade() {
        let state = window.global::<ProjectState>();
        state.set_notes_open(false);
        state.set_notes_blocks(ModelRc::new(VecModel::from(Vec::<Block>::new())));
        state.set_notes_status("".into());
    }
}

/// Opens the viewer overlay on `gallery`'s row at `index` and fetches its full-size image.
///
/// Title and description come straight from the `GalleryImage` `open`'s last full load kept in
/// `shared.gallery`, so they show right away; only the pixels wait on the fetch. An `index`
/// past the end of that list (a stale click racing a project the user has since navigated away
/// from) is a no-op.
fn open_viewer(bridge: &Bridge, shared: &Shared, index: usize) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let Some((url, title, description)) = shared
        .gallery
        .lock()
        .expect("gallery lock")
        .get(index)
        .map(|item| {
            (
                item.url.clone(),
                item.title.clone().unwrap_or_default(),
                item.description.clone().unwrap_or_default(),
            )
        })
    else {
        return;
    };

    let state = window.global::<ProjectState>();
    state.set_viewer_open(true);
    state.set_viewer_index(index as i32);
    state.set_viewer_title(title.as_str().into());
    state.set_viewer_description(description.as_str().into());
    state.set_viewer_image(slint::Image::default());
    state.set_viewer_image_state("loading".into());

    fetch_viewer_image(bridge, shared, url);
}

/// Closes the viewer overlay. Bumps its generation counter too, so a full-size fetch still in
/// flight for the image it was showing paints nothing after this.
fn close_viewer(bridge: &Bridge, shared: &Shared) {
    shared.viewer_generation.fetch_add(1, Ordering::SeqCst);
    if let Some(window) = bridge.weak().upgrade() {
        let state = window.global::<ProjectState>();
        state.set_viewer_open(false);
        state.set_viewer_index(-1);
        state.set_viewer_title("".into());
        state.set_viewer_description("".into());
        state.set_viewer_image(slint::Image::default());
        state.set_viewer_image_state("".into());
    }
}

/// Moves the viewer by `delta` rows (`1` for Next, `-1` for Prev), clamped to `gallery`'s
/// bounds, and reopens on the row it lands on. A `delta` that would leave the current row (the
/// first row and Prev, or the last and Next) is a no-op: the screen already disables that
/// button, but a key press reaches here regardless of what is drawn.
fn step_viewer(bridge: &Bridge, shared: &Shared, delta: i32) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<ProjectState>();
    let len = shared.gallery.lock().expect("gallery lock").len() as i32;
    if len == 0 {
        return;
    }
    let current = state.get_viewer_index();
    let next = (current + delta).clamp(0, len - 1);
    if next == current {
        return;
    }
    open_viewer(bridge, shared, next as usize);
}

/// Fetches and decodes one gallery image's full-size bytes off the UI thread, and paints it
/// onto the viewer once it lands — unless `shared.viewer_generation` has moved on by then,
/// which means the user closed the viewer or stepped to another image while this fetch was in
/// flight, per the `slint-ui` skill's generation-counter pattern.
fn fetch_viewer_image(bridge: &Bridge, shared: &Shared, url: String) {
    let generation = shared.viewer_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let counter = Arc::clone(&shared.viewer_generation);
    bridge.run(
        "Gallery image",
        move |launcher| {
            Ok(launcher
                .fetch_image(&url)
                .ok()
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|bytes| decode_description_image(&bytes).ok())
                .map(DecodedImage::from))
        },
        move |window, decoded: Option<DecodedImage>| {
            if counter.load(Ordering::SeqCst) != generation {
                return;
            }
            let state = window.global::<ProjectState>();
            match decoded {
                Some(image) => {
                    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                        &image.pixels,
                        image.width,
                        image.height,
                    );
                    state.set_viewer_image(slint::Image::from_rgba8(buffer));
                    state.set_viewer_image_state("ready".into());
                }
                None => state.set_viewer_image_state("failed".into()),
            }
        },
    );
}

/// Marks every gallery row's initial load state: "loading" for the first `cap` rows with a
/// non-empty `url`, "failed" for any past that cap, the same degrade [`prepare_images`] gives a
/// pathological description. Returns each "loading" row's index and url, in order, for the
/// fetch job to work through.
fn prepare_gallery(rows: &mut [GalleryImageRow], cap: usize) -> ImagePairs {
    let mut pairs = Vec::new();
    for (index, row) in rows.iter_mut().enumerate() {
        if row.url.is_empty() {
            continue;
        }
        if pairs.len() < cap {
            row.image_state = "loading".into();
            pairs.push((index, row.url.to_string()));
        } else {
            row.image_state = "failed".into();
        }
    }
    pairs
}

/// Fetches and decodes up to [`MAX_IMAGES`] gallery thumbnails sequentially in one job, the
/// same shape [`fetch_description_images`] uses for a description: painting each one onto
/// `ProjectState.gallery` as it arrives, deduped by url, and checked against `counter` both
/// before a fetch starts and inside every posted closure.
fn fetch_gallery_images(
    bridge: &Bridge,
    counter: Arc<AtomicU64>,
    generation: u64,
    pairs: ImagePairs,
) {
    if pairs.is_empty() {
        return;
    }
    let mut order: Vec<String> = Vec::new();
    let mut indices_by_url: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, url) in pairs {
        indices_by_url
            .entry(url.clone())
            .or_insert_with(|| {
                order.push(url.clone());
                Vec::new()
            })
            .push(index);
    }

    let weak = bridge.weak().clone();
    bridge.run(
        "Gallery images",
        move |launcher| {
            for url in order {
                if counter.load(Ordering::SeqCst) != generation {
                    break;
                }
                let decoded = launcher
                    .fetch_image(&url)
                    .ok()
                    .and_then(|path| std::fs::read(path).ok())
                    .and_then(|bytes| decode_description_image(&bytes).ok())
                    .map(DecodedImage::from);
                let counter = Arc::clone(&counter);
                let indices = indices_by_url.get(&url).cloned().unwrap_or_default();
                let _ = weak.upgrade_in_event_loop(move |window| {
                    if counter.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    let state = window.global::<ProjectState>();
                    apply_gallery_image(&state, &indices, decoded);
                });
            }
            Ok(())
        },
        |_window, ()| {},
    );
}

/// Writes one decoded gallery thumbnail onto every row index in `indices`, the same in-place
/// update [`apply_image`] does for a description block.
fn apply_gallery_image(state: &ProjectState<'_>, indices: &[usize], decoded: Option<DecodedImage>) {
    let model = state.get_gallery();
    let image = decoded.as_ref().map(|image| {
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &image.pixels,
            image.width,
            image.height,
        );
        slint::Image::from_rgba8(buffer)
    });
    for &index in indices {
        let Some(mut row) = model.row_data(index) else {
            continue;
        };
        match &image {
            Some(image) => {
                row.image = image.clone();
                row.image_state = "ready".into();
            }
            None => row.image_state = "failed".into(),
        }
        model.set_row_data(index, row);
    }
}

/// Marks every image block's initial load state: "loading" for the first `cap` blocks with a
/// non-empty `url`, in the order they appear, and "failed" for any past that cap — the
/// permanent `[Image: alt]` degrade the description-rendering spec accepts for a pathological
/// description rather than a bug to chase further. Returns each "loading" block's index and
/// url, in that same order, for the fetch job to work through.
fn prepare_images(blocks: &mut [Block], cap: usize) -> ImagePairs {
    let mut pairs = Vec::new();
    for (index, block) in blocks.iter_mut().enumerate() {
        if block.kind.as_str() != "image" || block.url.is_empty() {
            continue;
        }
        if pairs.len() < cap {
            block.image_state = "loading".into();
            pairs.push((index, block.url.to_string()));
        } else {
            block.image_state = "failed".into();
        }
    }
    pairs
}

/// Fetches and decodes up to [`MAX_IMAGES`] description images sequentially in one job,
/// painting each one onto `target`'s block list as it arrives rather than waiting for the
/// whole batch — the first image a user sees does not wait on the last.
///
/// `pairs` is deduped by url before anything is fetched, so a changelog quoting the same
/// picture twice downloads and decodes it once; every block index that shared the url is
/// still updated, from the one decode.
///
/// This does not use [`Bridge::run`]'s own `done` callback, which only fires once after the
/// whole job returns: instead the job posts one `upgrade_in_event_loop` closure per image,
/// directly on `bridge`'s weak window handle, the same primitive `Bridge` itself is built on.
/// `generation` is checked both before a fetch starts and inside every posted closure, so a
/// project (or a Notes modal) the user has since navigated away from never has a stale image
/// painted over it.
fn fetch_description_images(
    bridge: &Bridge,
    target: ImageTarget,
    counter: Arc<AtomicU64>,
    generation: u64,
    pairs: ImagePairs,
) {
    if pairs.is_empty() {
        return;
    }
    // Group by url, keeping first-seen order, so the fetch loop below visits each url once
    // and knows every index to update once it lands.
    let mut order: Vec<String> = Vec::new();
    let mut indices_by_url: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, url) in pairs {
        indices_by_url
            .entry(url.clone())
            .or_insert_with(|| {
                order.push(url.clone());
                Vec::new()
            })
            .push(index);
    }

    let weak = bridge.weak().clone();
    bridge.run(
        "Description images",
        move |launcher| {
            for url in order {
                if counter.load(Ordering::SeqCst) != generation {
                    break;
                }
                let decoded = launcher
                    .fetch_image(&url)
                    .ok()
                    .and_then(|path| std::fs::read(path).ok())
                    .and_then(|bytes| decode_description_image(&bytes).ok())
                    .map(DecodedImage::from);
                let counter = Arc::clone(&counter);
                let indices = indices_by_url.get(&url).cloned().unwrap_or_default();
                let _ = weak.upgrade_in_event_loop(move |window| {
                    if counter.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    let state = window.global::<ProjectState>();
                    apply_image(&state, target, &indices, decoded);
                });
            }
            Ok(())
        },
        |_window, ()| {},
    );
}

/// Writes one decoded image onto every block index in `indices`, updating each row in place
/// with [`Model::set_row_data`] rather than rebuilding the whole list. The `slint::Image` is
/// built once from the decoded pixels and cloned per row — cheap, since it is a handle onto
/// shared pixel storage — rather than re-copied for every index that shares this url.
fn apply_image(
    state: &ProjectState<'_>,
    target: ImageTarget,
    indices: &[usize],
    decoded: Option<DecodedImage>,
) {
    let model = target.blocks(state);
    let image = decoded.as_ref().map(|image| {
        let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &image.pixels,
            image.width,
            image.height,
        );
        slint::Image::from_rgba8(buffer)
    });
    for &index in indices {
        let Some(mut block) = model.row_data(index) else {
            continue;
        };
        match &image {
            Some(image) => {
                block.image = image.clone();
                block.image_state = "ready".into();
            }
            None => block.image_state = "failed".into(),
        }
        model.set_row_data(index, block);
    }
}

/// Raw RGBA8 pixels and dimensions for one description image, decoded off the UI thread.
struct DecodedImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl From<(u32, u32, Vec<u8>)> for DecodedImage {
    fn from((width, height, pixels): (u32, u32, Vec<u8>)) -> Self {
        Self {
            width,
            height,
            pixels,
        }
    }
}

/// Builds every version row, in order, for the versions tab.
fn build_version_rows(
    versions: &[Version],
    installed: Option<&str>,
    instance: Option<&InstanceContext>,
    kind: ContentKind,
) -> Vec<ContentVersionRow> {
    versions
        .iter()
        .map(|v| {
            version_row(
                v,
                installed,
                instance.map(|(mc, l)| (mc.as_str(), *l)),
                kind,
            )
        })
        .collect()
}

/// Builds the description-tab (or Notes modal) row for one block.
///
/// Every field but the one or two a block's own kind uses is left at its type default — `0`,
/// `""`, `[]`, an empty `image` — per the doc comment on `Block` in `types.slint`. A table's
/// header and rows flatten into one row-major `cells` array, header included as row 0, with
/// `columns` saying how to slice it back apart; `BlockList` computes the row count itself. An
/// image block's `url` is filled here so the image-fetch job below knows what to fetch, but
/// `image`/`image_state` are left empty: nothing has been fetched yet at the point this pure
/// conversion runs. `prepare_images` marks each one "loading" right after, and `apply_image`
/// writes the fetched pixels onto that same block's row once the fetch lands.
pub fn block_row(block: &CoreBlock) -> Block {
    match block {
        CoreBlock::Heading(level, text) => Block {
            kind: format!("heading{}", (*level).clamp(1, 6)).into(),
            text: text.as_str().into(),
            ..Default::default()
        },
        CoreBlock::Paragraph(text) => Block {
            kind: "paragraph".into(),
            text: text.as_str().into(),
            ..Default::default()
        },
        CoreBlock::Bullet { depth, text } => Block {
            kind: "bullet".into(),
            text: text.as_str().into(),
            depth: i32::from(*depth),
            ..Default::default()
        },
        CoreBlock::Code(text) => Block {
            kind: "code".into(),
            text: text.as_str().into(),
            ..Default::default()
        },
        CoreBlock::Quote(text) => Block {
            kind: "quote".into(),
            text: text.as_str().into(),
            ..Default::default()
        },
        CoreBlock::Rule => Block {
            kind: "rule".into(),
            ..Default::default()
        },
        CoreBlock::Image { url, alt } => Block {
            kind: "image".into(),
            text: alt.as_str().into(),
            url: url.as_str().into(),
            ..Default::default()
        },
        CoreBlock::Table { header, rows } => {
            let columns = header.len();
            let cells: Vec<slint::SharedString> = header
                .iter()
                .chain(rows.iter().flatten())
                .map(|cell| cell.as_str().into())
                .collect();
            Block {
                kind: "table".into(),
                columns: columns as i32,
                cells: ModelRc::new(VecModel::from(cells)),
                ..Default::default()
            }
        }
    }
}

/// Builds the versions-tab row for one version. `installed` is the target instance's
/// installed version id for this project, if any. `instance` is the target's Minecraft
/// version and loader, `None` when there is no target; a row whose version cannot run on it
/// — no matching game version, or for a mod no compatible loader — comes back `compatible:
/// false`, which the screen shows as a disabled Install once "Show all versions" reveals it.
pub fn version_row(
    v: &Version,
    installed: Option<&str>,
    instance: Option<(&str, Loader)>,
    kind: ContentKind,
) -> ContentVersionRow {
    let is_installed = installed.is_some_and(|id| id == v.id);
    let compatible = match instance {
        Some((minecraft, loader)) => version_fits(v, minecraft, loader, kind),
        None => true,
    };
    let access_label = match instance {
        Some((minecraft, loader)) if !compatible => format!("Not for {minecraft} {loader}"),
        _ => format!(
            "{} {}",
            if is_installed { "Installed" } else { "Install" },
            v.number
        ),
    };
    ContentVersionRow {
        id: v.id.as_str().into(),
        number: v.number.as_str().into(),
        kind: release_kind_label(v.kind).into(),
        release_time: short_time(&v.published).into(),
        game_versions: v.game_versions.join(", ").into(),
        loaders: v.loaders.join(", ").into(),
        installed: is_installed,
        compatible,
        access_label: access_label.into(),
    }
}

/// Whether `version` can run on `minecraft` under `loader`: it must list `minecraft` among
/// its game versions, and a mod must also list a loader `compatible_loaders` names for it.
/// Every other content kind ignores loaders.
fn version_fits(version: &Version, minecraft: &str, loader: Loader, kind: ContentKind) -> bool {
    if !version.game_versions.iter().any(|g| g == minecraft) {
        return false;
    }
    if kind != ContentKind::Mod {
        return true;
    }
    let compatible = compatible_loaders(loader, minecraft);
    version
        .loaders
        .iter()
        .any(|l| compatible.iter().any(|c| c == l))
}

/// The lowercase label a release kind shows in the badge.
fn release_kind_label(kind: ReleaseKind) -> &'static str {
    match kind {
        ReleaseKind::Alpha => "alpha",
        ReleaseKind::Beta => "beta",
        ReleaseKind::Release => "release",
    }
}

#[cfg(test)]
mod tests;
