//! Content orchestration: resolve a project at a source, pick a compatible version,
//! download its file into the object store, and place it in an instance.
//!
//! This module is the seam between [`crate::sources`] (what exists at Modrinth and
//! CurseForge) and [`crate::instances::content`] (where a file goes on disk). It adds
//! the rules that neither side owns: loader compatibility, version choice, required
//! dependencies, update checks, and importing a file the user had to fetch by hand.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::download::hash::sha1_file;
use crate::download::{DownloadCtx, DownloadSpec, download_one};
use crate::events::{Event, EventSink, LogLevel};
use crate::instances::Instance;
use crate::instances::content::{installed, place_file, place_world};
use crate::instances::model::{ContentEntry, ContentKind, Loader};
use crate::paths::Root;
use crate::sources::fingerprint::fingerprint_file;
use crate::sources::{
    BoxSource, DependencyKind, Project, SourceId, Version, VersionFile, VersionFilter, curseforge,
};

#[cfg(test)]
mod tests;

/// How many dependency hops `add` follows before it gives up.
const MAX_DEPENDENCY_DEPTH: usize = 10;

/// Loader names that mean "this file is a mod", used to tell a data pack apart from one.
const MOD_LOADERS: [&str; 4] = ["fabric", "quilt", "forge", "neoforge"];

/// The one Minecraft version where NeoForge still loads Forge mods.
const NEOFORGE_FORGE_COMPAT_MC: &str = "1.20.1";

/// Errors from adding, updating, or importing content.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A source lookup, search, or resolve failed.
    #[error(transparent)]
    Sources(#[from] crate::sources::Error),
    /// Fetching a file into the object store failed.
    #[error(transparent)]
    Download(#[from] crate::download::Error),
    /// Placing a file into the instance, or saving `instance.toml`, failed.
    #[error(transparent)]
    Instance(#[from] crate::instances::Error),
    /// A path under the app root could not be built.
    #[error(transparent)]
    Paths(#[from] crate::paths::Error),
    /// No source with that id is configured, usually a missing CurseForge API key.
    #[error("no content source configured for {0}")]
    SourceUnavailable(SourceId),
    /// The project has no version for this instance's Minecraft version and loader.
    #[error("{project} has no version for Minecraft {minecraft} on loader {loader}")]
    NoCompatibleVersion {
        /// Project id or slug that was asked for.
        project: String,
        /// Minecraft version the instance runs.
        minecraft: String,
        /// Loader the instance runs.
        loader: Loader,
    },
    /// A hand-downloaded file does not hash to what the source published.
    #[error("{} does not match: expected {expected}, got {actual}", file.display())]
    VerificationFailed {
        /// File the user supplied.
        file: PathBuf,
        /// Hash or fingerprint the source published.
        expected: String,
        /// Hash or fingerprint the file actually has.
        actual: String,
    },
    /// The dependency chain is more than 10 hops deep.
    #[error("dependency chain is more than 10 levels deep at {0}")]
    DependencyDepth(String),
    /// A filesystem operation on an object or a dropped file failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Everything a content operation borrows: the configured sources, the download cache,
/// the app root, and the event sink.
///
/// `Source` is a trait object with no `Debug` bound, so this type has none either.
#[derive(Clone, Copy)]
pub struct ContentCtx<'a> {
    /// Sources the user has configured, in preference order.
    pub sources: &'a [BoxSource],
    /// Shared download context, which owns the object store and the HTTP client.
    pub dl: &'a DownloadCtx<'a>,
    /// App root, for object paths.
    pub root: &'a Root,
    /// Where progress and log events go.
    pub sink: &'a EventSink,
}

impl ContentCtx<'_> {
    /// The configured source with this id, or [`Error::SourceUnavailable`].
    fn source(&self, id: SourceId) -> Result<&BoxSource, Error> {
        self.sources
            .iter()
            .find(|s| s.id() == id)
            .ok_or(Error::SourceUnavailable(id))
    }

    /// Emits an [`Event::Log`] line at info level.
    pub(crate) fn log(&self, message: impl Into<String>) {
        let _ = self.sink.send(Event::Log {
            level: LogLevel::Info,
            message: message.into(),
        });
    }
}

/// One request to install a project into an instance.
#[derive(Debug, Clone)]
pub struct AddRequest {
    /// Source to resolve the project at.
    pub source: SourceId,
    /// Project id or slug.
    pub project: String,
    /// Version id or number to pin. `None` picks the newest compatible version.
    pub version: Option<String>,
    /// Content kind override. `None` takes the kind from the project.
    pub kind: Option<ContentKind>,
    /// World a data pack installs into.
    pub world: Option<String>,
}

/// What one [`add`] call did.
#[derive(Debug, Clone, Default)]
pub struct AddOutcome {
    /// Entries written to `instance.toml`, in install order.
    pub installed: Vec<ContentEntry>,
    /// Project ids that were already installed and were left alone.
    pub skipped: Vec<String>,
    /// Files the user must fetch from a browser, because the author opted out of
    /// third-party distribution.
    pub manual: Vec<ManualDownload>,
}

/// A file the launcher may not download: the user fetches it and imports it with
/// [`import_manual`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualDownload {
    /// Source the file belongs to. [`import_manual`] records it in the entry.
    pub source: SourceId,
    /// Project id at the source.
    pub project_id: String,
    /// Version id at the source.
    pub version_id: String,
    /// File name the user should end up with.
    pub file_name: String,
    /// Browser page to fetch the file from.
    pub page_url: String,
    /// Expected CurseForge fingerprint, when the source publishes one.
    pub fingerprint: Option<u32>,
    /// Expected sha1, when the source publishes one.
    pub sha1: Option<String>,
    /// World a data pack installs into, carried over from the request that produced it.
    pub world: Option<String>,
}

/// An installed entry that has a newer compatible version at its source.
#[derive(Debug, Clone)]
pub struct UpdateCandidate {
    /// The entry as `instance.toml` records it now.
    pub entry: ContentEntry,
    /// The newer version [`pick_version`] chose.
    pub new: Version,
}

/// Loader names whose files run on `loader` at Minecraft `minecraft`.
///
/// Quilt loads Fabric mods. NeoForge loads Forge mods only at 1.20.1, where the two
/// forks had not diverged yet. A loader-less instance runs no mods at all, so the list
/// is empty and [`add`] refuses a [`ContentKind::Mod`].
pub fn compatible_loaders(loader: Loader, minecraft: &str) -> Vec<&'static str> {
    match loader {
        Loader::Fabric => vec!["fabric"],
        Loader::Quilt => vec!["quilt", "fabric"],
        Loader::Forge => vec!["forge"],
        Loader::NeoForge if minecraft == NEOFORGE_FORGE_COMPAT_MC => vec!["neoforge", "forge"],
        Loader::NeoForge => vec!["neoforge"],
        Loader::None => Vec::new(),
    }
}

/// Chooses the version to install from `versions`.
///
/// Every candidate must list `minecraft` in its game versions. A [`ContentKind::Mod`]
/// must also list a loader from [`compatible_loaders`]. A [`ContentKind::DataPack`] must
/// either be tagged `datapack` or carry no mod loader at all, because Modrinth serves
/// data packs both ways. Other kinds ignore loaders entirely.
///
/// `want` matches a version id or a version number exactly and wins when it is
/// compatible. Otherwise the newest release wins: candidates sort by release channel
/// first, publish time second.
pub fn pick_version(
    versions: &[Version],
    kind: ContentKind,
    minecraft: &str,
    loader: Loader,
    want: Option<&str>,
) -> Option<Version> {
    let compatible = compatible_loaders(loader, minecraft);
    let mut usable: Vec<&Version> = versions
        .iter()
        .filter(|v| v.game_versions.iter().any(|g| g == minecraft))
        .filter(|v| loader_matches(v, kind, &compatible))
        .collect();

    if let Some(want) = want {
        return usable
            .into_iter()
            .find(|v| v.id == want || v.number == want)
            .cloned();
    }

    usable.sort_by(|a, b| {
        b.kind
            .cmp(&a.kind)
            .then_with(|| b.published.cmp(&a.published))
    });
    usable.first().map(|v| (*v).clone())
}

/// Whether `version` can run under the loaders in `compatible`, for this content kind.
fn loader_matches(version: &Version, kind: ContentKind, compatible: &[&str]) -> bool {
    match kind {
        ContentKind::Mod => version
            .loaders
            .iter()
            .any(|l| compatible.iter().any(|c| c == l)),
        ContentKind::DataPack => {
            version.loaders.iter().any(|l| l == "datapack")
                || !version.loaders.iter().any(|l| MOD_LOADERS.contains(&&**l))
        }
        _ => true,
    }
}

/// One item waiting to be resolved and installed.
#[derive(Debug, Clone)]
struct Pending {
    /// Project id or slug to resolve.
    project: String,
    /// Version id or number to pin, when the caller asked for one.
    version: Option<String>,
    /// Content kind override, when the caller asked for one.
    kind: Option<ContentKind>,
    /// World a data pack installs into.
    world: Option<String>,
    /// How many dependency hops away from the original request this is.
    depth: usize,
}

/// Installs a project and its required dependencies into `instance`.
///
/// The queue is breadth-first from `req`: each item resolves its project, picks a
/// compatible version, downloads the version's primary file into the object store, and
/// places it in the instance. A project already installed from the same source is
/// skipped unless the caller pinned a different version. A file the author opted out of
/// third-party distribution has no URL; it is reported in [`AddOutcome::manual`] and
/// never fails the call.
///
/// `instance` is replaced with the saved copy after every placement, so the caller's
/// value stays in step with `instance.toml`.
#[tracing::instrument(skip(ctx), fields(slug = %instance.slug))]
pub async fn add(
    ctx: &ContentCtx<'_>,
    instance: &mut Instance,
    req: AddRequest,
) -> Result<AddOutcome, Error> {
    let source = ctx.source(req.source)?;
    let minecraft = instance.config.minecraft.clone();
    let loader = instance.config.loader;

    let mut outcome = AddOutcome::default();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<Pending> = VecDeque::new();
    queue.push_back(Pending {
        project: req.project.clone(),
        version: req.version.clone(),
        kind: req.kind,
        world: req.world.clone(),
        depth: 0,
    });

    while let Some(item) = queue.pop_front() {
        if item.depth > MAX_DEPENDENCY_DEPTH {
            return Err(Error::DependencyDepth(item.project));
        }
        if seen.contains(&item.project) {
            continue;
        }
        let project = source.project(&item.project).await?;
        if !seen.insert(project.id.clone()) {
            continue;
        }
        seen.insert(item.project.clone());

        if let Some(entry) = installed(instance, req.source, &project.id)
            && item
                .version
                .as_ref()
                .is_none_or(|want| *want == entry.version_id)
        {
            ctx.log(format!(
                "{} is already installed at {}",
                project.title, entry.version_id
            ));
            outcome.skipped.push(project.id.clone());
            let version_id = entry.version_id.clone();
            queue_installed_dependencies(ctx, source, &version_id, &item, &mut queue).await;
            continue;
        }

        let kind = item.kind.unwrap_or(project.kind);
        if kind == ContentKind::Mod && loader == Loader::None {
            return Err(Error::NoCompatibleVersion {
                project: project.id,
                minecraft,
                loader,
            });
        }
        let filter = VersionFilter {
            minecraft: Some(minecraft.clone()),
            loaders: if kind == ContentKind::Mod {
                compatible_loaders(loader, &minecraft)
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            } else {
                Vec::new()
            },
        };
        let versions = source.versions(&project.id, &filter).await?;
        let version = pick_version(&versions, kind, &minecraft, loader, item.version.as_deref())
            .ok_or_else(|| Error::NoCompatibleVersion {
                project: project.id.clone(),
                minecraft: minecraft.clone(),
                loader,
            })?;

        match primary_file(&version) {
            Some(file) if file.url.is_some() => {
                let entry =
                    install_file(ctx, instance, &project, &version, file, kind, &item).await?;
                outcome.installed.push(entry);
            }
            Some(file) => {
                let page_url = manual_page_url(&project, &version);
                ctx.log(format!(
                    "{} must be downloaded by hand from {page_url}",
                    file.file_name
                ));
                outcome.manual.push(ManualDownload {
                    source: req.source,
                    project_id: project.id.clone(),
                    version_id: version.id.clone(),
                    file_name: file.file_name.clone(),
                    page_url,
                    fingerprint: file.fingerprint,
                    sha1: file.sha1.clone(),
                    world: item.world.clone(),
                });
            }
            None => {
                return Err(Error::NoCompatibleVersion {
                    project: project.id.clone(),
                    minecraft: minecraft.clone(),
                    loader,
                });
            }
        }

        queue_dependencies(&mut queue, &version.dependencies, &item);
    }

    Ok(outcome)
}

/// Pushes every required dependency of `deps` onto the queue, one hop deeper.
///
/// Dependencies inherit the parent's world, so a data pack's dependency lands in the
/// same world folder. A dependency with no project id cannot be resolved and is dropped.
fn queue_dependencies(
    queue: &mut VecDeque<Pending>,
    deps: &[crate::sources::Dependency],
    parent: &Pending,
) {
    for dep in deps {
        if dep.kind != DependencyKind::Required {
            continue;
        }
        let Some(project_id) = dep.project_id.clone() else {
            continue;
        };
        queue.push_back(Pending {
            project: project_id,
            version: dep.version_id.clone(),
            kind: None,
            world: parent.world.clone(),
            depth: parent.depth + 1,
        });
    }
}

/// Queues the dependencies of a version that is already installed.
///
/// `instance.toml` records no dependency edges, so the version has to be fetched again
/// to walk past an entry that is already there. A source that cannot answer only costs
/// this branch of the walk: it warns and stops.
async fn queue_installed_dependencies(
    ctx: &ContentCtx<'_>,
    source: &BoxSource,
    version_id: &str,
    parent: &Pending,
    queue: &mut VecDeque<Pending>,
) {
    match source.version(version_id).await {
        Ok(version) => queue_dependencies(queue, &version.dependencies, parent),
        Err(err) => warn(ctx, version_id, &err.to_string()),
    }
}

/// The file to install from a version: the primary one, or the first when none is marked.
pub(crate) fn primary_file(version: &Version) -> Option<&VersionFile> {
    version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
}

/// The browser page a manual download points at.
///
/// CurseForge has a per-file page under the project; Modrinth sends the user to the
/// project page, where every version is listed.
fn manual_page_url(project: &Project, version: &Version) -> String {
    match project.source {
        SourceId::CurseForge => curseforge::file_page_url(&project.page_url, &version.id),
        SourceId::Modrinth => project.page_url.clone(),
    }
}

/// Downloads one version file into the object store and places it in the instance.
async fn install_file(
    ctx: &ContentCtx<'_>,
    instance: &mut Instance,
    project: &Project,
    version: &Version,
    file: &VersionFile,
    kind: ContentKind,
    item: &Pending,
) -> Result<ContentEntry, Error> {
    let label = format!("{} {}", project.title, file.file_name);
    let (object, sha1) = fetch_object(ctx, file, label).await?;
    let fingerprint = match project.source {
        SourceId::CurseForge => Some(match file.fingerprint {
            Some(fp) => fp,
            None => fingerprint_of(&object).await?,
        }),
        SourceId::Modrinth => file.fingerprint,
    };

    let entry = ContentEntry {
        source: project.source.to_string(),
        project_id: project.id.clone(),
        version_id: version.id.clone(),
        file_name: file.file_name.clone(),
        sha1: Some(sha1),
        fingerprint,
        kind,
        world: item.world.clone(),
        enabled: true,
    };
    place(instance, object, entry, kind).await
}

/// Fetches `file` into `cache/objects/` and returns its object path and sha1.
///
/// The download always lands on a staging path under `cache/objects/tmp/`, named
/// `<uuid>.part` so [`crate::download::cleanup_partials`] sweeps an abandoned one, never
/// on the object path itself: [`download_one`] stores the object first and then links it to the
/// destination, so naming the object as the destination would unlink the object it just
/// wrote. A published sha1 goes into the spec, so a jar both sources serve is verified
/// on arrival and fetched only once. With no published sha1 the staged bytes are hashed
/// here instead. Objects are written once: the staged copy is dropped when the object is
/// already in the store.
pub(crate) async fn fetch_object(
    ctx: &ContentCtx<'_>,
    file: &VersionFile,
    label: String,
) -> Result<(PathBuf, String), Error> {
    let known = file
        .sha1
        .as_deref()
        .filter(|sha1| ctx.root.object_path(sha1).is_ok())
        .map(str::to_string);
    // The `.part` suffix is what `download::cleanup_partials` sweeps, so a staged file
    // abandoned by a crash is removed on the next start.
    let staging = ctx
        .root
        .objects_dir()
        .join("tmp")
        .join(format!("{}.part", uuid::Uuid::new_v4()));
    create_parent(&staging).await?;

    let spec = DownloadSpec {
        url: file.url.clone().unwrap_or_default(),
        sha1: known.clone(),
        size: file.size,
        dest: staging.clone(),
        label,
    };
    let staged = download_one(ctx.dl, &spec).await?;

    let sha1 = match known {
        Some(sha1) => sha1,
        None => hash_of(&staged).await?,
    };
    let object = ctx.root.object_path(&sha1)?;
    create_parent(&object).await?;
    if tokio::fs::metadata(&object).await.is_ok() {
        remove_quietly(&staged).await;
    } else if tokio::fs::rename(&staged, &object).await.is_err() {
        tokio::fs::copy(&staged, &object)
            .await
            .map_err(io_at(&object))?;
        remove_quietly(&staged).await;
    }
    Ok((object, sha1))
}

/// Places `object` in the instance and swaps in the saved copy of the instance.
///
/// [`place_file`] and [`place_world`] both mutate an [`Instance`] and write
/// `instance.toml`, and both block on file I/O. The instance moves into the blocking
/// task and comes back out, so the caller's value carries the saved content list.
pub(crate) async fn place(
    instance: &mut Instance,
    object: PathBuf,
    entry: ContentEntry,
    kind: ContentKind,
) -> Result<ContentEntry, Error> {
    let taken = instance.clone();
    let placed = tokio::task::spawn_blocking(move || {
        let mut owned = taken;
        let result = if kind == ContentKind::World {
            place_world(&mut owned, &object, entry)
        } else {
            place_file(&mut owned, &object, entry)
        };
        result.map(|placed| (owned, placed))
    })
    .await
    .map_err(|source| Error::Io {
        path: instance.config_path(),
        source: std::io::Error::other(source),
    })?;
    let (saved, placed) = placed?;
    *instance = saved;
    Ok(placed.entry)
}

/// Lists the installed entries that have a newer compatible version at their source.
///
/// An entry whose source is not configured, or whose source answers with an error, is
/// skipped with an [`Event::Warning`]: one dead source must not hide the updates the
/// others found. An entry whose `source` names no known source — `file`, written by a
/// `.mrpack` import — is skipped silently: it has no project to check.
#[tracing::instrument(skip(ctx), fields(slug = %instance.slug))]
pub async fn check_updates(
    ctx: &ContentCtx<'_>,
    instance: &Instance,
) -> Result<Vec<UpdateCandidate>, Error> {
    let minecraft = &instance.config.minecraft;
    let loader = instance.config.loader;
    let mut candidates = Vec::new();

    for entry in &instance.config.content {
        // A source that does not parse is skipped without a word. `file` is the one
        // that happens in practice: a `.mrpack` file records no project at either
        // source, so there is nothing to ask for a newer version.
        let Some(source_id) = SourceId::parse(&entry.source) else {
            continue;
        };
        let source = match ctx.source(source_id) {
            Ok(source) => source,
            Err(err) => {
                warn(ctx, &entry.file_name, &err.to_string());
                continue;
            }
        };
        let filter = VersionFilter {
            minecraft: Some(minecraft.clone()),
            loaders: if entry.kind == ContentKind::Mod {
                compatible_loaders(loader, minecraft)
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            } else {
                Vec::new()
            },
        };
        let versions = match source.versions(&entry.project_id, &filter).await {
            Ok(versions) => versions,
            Err(err) => {
                warn(ctx, &entry.file_name, &err.to_string());
                continue;
            }
        };
        let Some(newest) = pick_version(&versions, entry.kind, minecraft, loader, None) else {
            continue;
        };
        if newest.id != entry.version_id {
            candidates.push(UpdateCandidate {
                entry: entry.clone(),
                new: newest,
            });
        }
    }
    Ok(candidates)
}

/// Installs the newer version a [`check_updates`] candidate names.
///
/// This is [`add`] pinned to the candidate's version id. Placing replaces the old file
/// and rewrites the entry where it sits in the content list.
#[tracing::instrument(skip(ctx), fields(slug = %instance.slug))]
pub async fn apply_update(
    ctx: &ContentCtx<'_>,
    instance: &mut Instance,
    candidate: &UpdateCandidate,
) -> Result<AddOutcome, Error> {
    let req = AddRequest {
        source: candidate.new.source,
        project: candidate.entry.project_id.clone(),
        version: Some(candidate.new.id.clone()),
        kind: Some(candidate.entry.kind),
        world: candidate.entry.world.clone(),
    };
    add(ctx, instance, req).await
}

/// Verifies a file the user downloaded by hand, stores it, and places it.
///
/// The CurseForge fingerprint is checked when the source published one; otherwise the
/// sha1 is. A pending download with neither is accepted as-is, because there is nothing
/// to check it against. The file is copied, never moved: it is the user's own file.
///
/// [`ManualDownload::world`] decides the world a [`ContentKind::DataPack`] lands in, so a
/// data pack the user fetched by hand goes to the same `saves/<world>/datapacks/` the
/// original request asked for.
#[tracing::instrument(skip(ctx), fields(slug = %instance.slug, file = %file.display()))]
pub async fn import_manual(
    ctx: &ContentCtx<'_>,
    instance: &mut Instance,
    pending: &ManualDownload,
    file: &Path,
    kind: ContentKind,
) -> Result<ContentEntry, Error> {
    let sha1 = hash_of(file).await?;
    let fingerprint = fingerprint_of(file).await?;

    if let Some(expected) = pending.fingerprint {
        if expected != fingerprint {
            return Err(Error::VerificationFailed {
                file: file.to_path_buf(),
                expected: expected.to_string(),
                actual: fingerprint.to_string(),
            });
        }
    } else if let Some(expected) = pending.sha1.as_deref()
        && !expected.eq_ignore_ascii_case(&sha1)
    {
        return Err(Error::VerificationFailed {
            file: file.to_path_buf(),
            expected: expected.to_string(),
            actual: sha1,
        });
    }

    let object = ctx.root.object_path(&sha1)?;
    create_parent(&object).await?;
    if tokio::fs::metadata(&object).await.is_err() {
        tokio::fs::copy(file, &object)
            .await
            .map_err(io_at(&object))?;
    }

    let entry = ContentEntry {
        source: pending.source.to_string(),
        project_id: pending.project_id.clone(),
        version_id: pending.version_id.clone(),
        file_name: pending.file_name.clone(),
        sha1: Some(sha1),
        fingerprint: match pending.source {
            SourceId::CurseForge => Some(fingerprint),
            SourceId::Modrinth => pending.fingerprint,
        },
        kind,
        world: pending.world.clone(),
        enabled: true,
    };
    ctx.log(format!("imported {} by hand", entry.file_name));
    place(instance, object, entry, kind).await
}

/// Sends an [`Event::Warning`] naming the entry it concerns.
fn warn(ctx: &ContentCtx<'_>, what: &str, detail: &str) {
    let _ = ctx.sink.send(Event::Warning(format!("{what}: {detail}")));
}

/// Hashes a file on a blocking thread.
async fn hash_of(path: &Path) -> Result<String, Error> {
    let owned = path.to_path_buf();
    blocking(path, move || sha1_file(&owned)).await
}

/// Computes a CurseForge fingerprint on a blocking thread.
async fn fingerprint_of(path: &Path) -> Result<u32, Error> {
    let owned = path.to_path_buf();
    blocking(path, move || fingerprint_file(&owned)).await
}

/// Runs a blocking file operation and maps both the join and the I/O error to `path`.
async fn blocking<T, F>(path: &Path, work: F) -> Result<T, Error>
where
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other(source),
        })?
        .map_err(io_at(path))
}

/// Creates the parent directory of `path`.
async fn create_parent(path: &Path) -> Result<(), Error> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(io_at(parent))
}

/// Deletes a file, ignoring a failure: it is staging waste, not user data.
async fn remove_quietly(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

/// Builds an [`Error::Io`] mapper that carries `path`.
fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Io { path, source }
}
