//! Wires the project details screen: description, versions, and installing a chosen version.
//!
//! Every call into `gcl-core` runs off the UI thread through [`Bridge`]. The screen is pure
//! layout; this module owns the `ProjectState` global that feeds it.

use gcl_core::content::{AddRequest, compatible_loaders};
use gcl_core::instances::model::{ContentKind, Loader};
use gcl_core::launcher::ProjectDetails;
use gcl_core::sources::richtext::Block as CoreBlock;
use gcl_core::sources::{ReleaseKind, SourceId, Version, VersionFilter};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::bridge::Bridge;
use crate::models::short_time;
use crate::screens::browser::conflict_note;
use crate::{AppWindow, Block, ContentVersionRow, ProjectState, Screen};

/// Binds the `ProjectState` global to the launcher.
///
/// Nothing is loaded here: a screen that wants to show a project calls `ProjectState.open`
/// with the source, the project id, the instance a version installs into, and the screen to
/// return to. There is no "screen was shown" watcher, unlike the other screens, because this
/// one needs those four things before it can load anything.
pub fn wire(window: &AppWindow, bridge: &Bridge) {
    let state = window.global::<ProjectState>();

    {
        let bridge = bridge.clone();
        state.on_open(
            move |source, project_id, target_slug, return_to, downloads, author| {
                open(
                    &bridge,
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

    let Some(source_id) = SourceId::parse(&source) else {
        state.set_status(format!("Unknown content source: {source}").into());
        return;
    };

    state.set_loading(true);
    let show_all = false;
    run_reporting(bridge, "Project details", false, move |launcher| {
        load(launcher, source_id, &project_id, &target_slug, show_all)
    });
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
fn run_reporting(
    bridge: &Bridge,
    label: &'static str,
    preserve_status: bool,
    job: impl FnOnce(&gcl_core::Launcher) -> Result<Opened, gcl_core::Error> + Send + 'static,
) {
    bridge.run_with_error(label, job, move |window, result| {
        let state = window.global::<ProjectState>();
        state.set_loading(false);
        match result {
            Ok(opened) => apply(window, opened, preserve_status),
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

/// Fills every property `open` promised, from a successful full load.
fn apply(window: &AppWindow, opened: Opened, preserve_status: bool) {
    let state = window.global::<ProjectState>();
    let project = opened.details.project;
    let kind = project.kind;
    // The canonical id, so a later `reload_versions` or `install` matches the same
    // installed entry this open just did, even when the caller opened with a slug.
    state.set_project_id(project.id.as_str().into());
    state.set_title(project.title.as_str().into());
    state.set_kind(project.kind.to_string().into());
    state.set_page_url(project.page_url.as_str().into());

    let blocks: Vec<Block> = opened.details.blocks.iter().map(block_row).collect();
    state.set_blocks(ModelRc::new(VecModel::from(blocks)));

    let rows = build_version_rows(
        &opened.versions,
        opened.installed_version_id.as_deref(),
        opened.instance.as_ref(),
        kind,
    );
    state.set_versions(ModelRc::new(VecModel::from(rows)));

    if preserve_status {
        return;
    }
    if state.get_target_slug().is_empty() {
        state.set_status("Pick a target instance in the browser first".into());
    } else {
        state.set_status(format!("{} version(s)", opened.versions.len()).into());
    }
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
/// image block's `url` is filled here so `screens/project.rs`'s image-fetch job (Task 4) knows
/// what to fetch, but `image`/`image_state` are left empty: nothing has been fetched yet at the
/// point this pure conversion runs, and the per-open `url -> ImageState` side map merges the
/// fetched result in afterward, on the UI thread.
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
