//! Wires the project details screen: description, versions, and installing a chosen version.
//!
//! Every call into `gcl-core` runs off the UI thread through [`Bridge`]. The screen is pure
//! layout; this module owns the `ProjectState` global that feeds it.

use gcl_core::content::AddRequest;
use gcl_core::instances::model::Loader;
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
        state.on_open(move |source, project_id, target_slug, return_to| {
            open(
                &bridge,
                source.to_string(),
                project_id.to_string(),
                target_slug.to_string(),
                return_to,
            );
        });
    }

    {
        let bridge = bridge.clone();
        state.on_set_tab(move |tab| {
            if let Some(window) = bridge.weak().upgrade() {
                window.global::<ProjectState>().set_tab(tab);
            }
        });
    }

    {
        let bridge = bridge.clone();
        state.on_back(move || {
            if let Some(window) = bridge.weak().upgrade() {
                let return_to = window.global::<ProjectState>().get_return_to();
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

/// Everything one `open` reads off disk, as the UI-thread half shows it.
struct Opened {
    details: ProjectDetails,
    versions: Vec<Version>,
    installed_version_id: Option<String>,
}

/// Loads a project's description and its versions, filtered by the target instance unless
/// the screen already asked to see them all.
fn open(
    bridge: &Bridge,
    source: String,
    project_id: String,
    target_slug: String,
    return_to: Screen,
) {
    let Some(window) = bridge.weak().upgrade() else {
        return;
    };
    let state = window.global::<ProjectState>();

    // Every property this screen owns is set here, the empty case included, so the screen
    // never shows the previous project while this one loads.
    state.set_source(source.as_str().into());
    state.set_project_id(project_id.as_str().into());
    state.set_target_slug(target_slug.as_str().into());
    state.set_return_to(return_to);
    state.set_tab(0);
    state.set_title("".into());
    state.set_author("".into());
    state.set_kind("".into());
    state.set_downloads("".into());
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
    let Some(source_id) = SourceId::parse(&source) else {
        return;
    };

    if !preserve_status {
        state.set_loading(true);
    }
    run_reporting(
        bridge,
        "Project versions",
        preserve_status,
        move |launcher| {
            let (versions, installed_version_id) =
                versions_and_installed(launcher, source_id, &project_id, &target_slug, show_all)?;
            let details = launcher.project_details(source_id, &project_id)?;
            Ok(Opened {
                details,
                versions,
                installed_version_id,
            })
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
    let (versions, installed_version_id) =
        versions_and_installed(launcher, source_id, project_id, target_slug, show_all)?;
    Ok(Opened {
        details,
        versions,
        installed_version_id,
    })
}

/// Lists a project's versions, filtered by the target instance's Minecraft version and loader
/// unless `show_all` is set or there is no target, and pairs them with that instance's
/// installed version for this project, if any.
fn versions_and_installed(
    launcher: &gcl_core::Launcher,
    source_id: SourceId,
    project_id: &str,
    target_slug: &str,
    show_all: bool,
) -> Result<(Vec<Version>, Option<String>), gcl_core::Error> {
    if target_slug.is_empty() {
        let versions =
            launcher.project_versions(source_id, project_id, &VersionFilter::default())?;
        return Ok((versions, None));
    }

    let instance = launcher.instances().get(target_slug)?;
    let filter = if show_all {
        VersionFilter::default()
    } else {
        VersionFilter {
            minecraft: Some(instance.config.minecraft.clone()),
            loaders: loader_filter(instance.config.loader),
        }
    };
    let versions = launcher.project_versions(source_id, project_id, &filter)?;

    let source_label = source_id.to_string();
    let installed_version_id = launcher
        .list_content(target_slug)?
        .into_iter()
        .find(|entry| entry.source == source_label && entry.project_id == project_id)
        .map(|entry| entry.version_id);
    Ok((versions, installed_version_id))
}

/// The loader slugs a version filter asks for. `Loader::None` means no loader filter.
fn loader_filter(loader: Loader) -> Vec<String> {
    match loader {
        Loader::None => Vec::new(),
        other => vec![other.to_string()],
    }
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
            state.set_loading(false);
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
                Err(_) => state.set_status("Install version failed".into()),
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

/// Fills every property `open`/`reload_versions` promised, from a successful load.
fn apply(window: &AppWindow, opened: Opened, preserve_status: bool) {
    let state = window.global::<ProjectState>();
    let project = opened.details.project;
    state.set_title(project.title.as_str().into());
    state.set_kind(project.kind.to_string().into());
    state.set_page_url(project.page_url.as_str().into());

    let blocks: Vec<Block> = opened.details.blocks.iter().map(block_row).collect();
    state.set_blocks(ModelRc::new(VecModel::from(blocks)));

    let installed = opened.installed_version_id.as_deref();
    let versions: Vec<ContentVersionRow> = opened
        .versions
        .iter()
        .map(|v| version_row(v, installed))
        .collect();
    state.set_versions(ModelRc::new(VecModel::from(versions)));

    if preserve_status {
        return;
    }
    if state.get_target_slug().is_empty() {
        state.set_status("Pick a target instance in the browser first".into());
    } else {
        state.set_status(format!("{} version(s)", opened.versions.len()).into());
    }
}

/// Builds the description-tab row for one block.
pub fn block_row(block: &CoreBlock) -> Block {
    let (kind, text) = match block {
        CoreBlock::Heading(level, text) => (format!("heading{}", (*level).clamp(1, 6)), text),
        CoreBlock::Paragraph(text) => ("paragraph".to_string(), text),
        CoreBlock::Bullet(text) => ("bullet".to_string(), text),
        CoreBlock::Code(text) => ("code".to_string(), text),
    };
    Block {
        kind: kind.into(),
        text: text.as_str().into(),
    }
}

/// Builds the versions-tab row for one version. `installed` is the target instance's
/// installed version id for this project, if any.
pub fn version_row(v: &Version, installed: Option<&str>) -> ContentVersionRow {
    ContentVersionRow {
        id: v.id.as_str().into(),
        number: v.number.as_str().into(),
        kind: release_kind_label(v.kind).into(),
        release_time: short_time(&v.published).into(),
        game_versions: v.game_versions.join(", ").into(),
        loaders: v.loaders.join(", ").into(),
        installed: installed.is_some_and(|id| id == v.id),
    }
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
