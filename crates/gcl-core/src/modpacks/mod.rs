//! Modpack import: read a pack zip, turn it into a [`PackPlan`], and install that plan
//! as a new instance.
//!
//! Two formats land here. A Modrinth `.mrpack` names every file with a URL and a sha1,
//! so the launcher downloads them itself. A CurseForge pack names project and file ids,
//! so its files are resolved at the API first and placed by their class. Both then copy
//! their override folders over the game directory. Format detail is in the
//! `modpack-formats` skill.

pub mod curseforge;
pub mod mrpack;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::content::{ContentCtx, ManualDownload};
use crate::download::link_or_copy;
use crate::instances::model::{ContentEntry, ContentKind, Loader, PackSource};
use crate::instances::{Instance, Instances};
use crate::loaders::{LoaderCtx, LoaderEndpoints};
use crate::paths::safe_join;
use crate::sources::{SourceId, Version, VersionFile};

/// Name of the manifest inside a Modrinth `.mrpack`.
const MRPACK_INDEX: &str = "modrinth.index.json";

/// Name of the manifest inside a CurseForge modpack zip.
const CF_MANIFEST: &str = "manifest.json";

/// `manifestType` value that marks a CurseForge zip as a Minecraft modpack.
const CF_PACK_TYPE: &str = "minecraftModpack";

/// Largest manifest this module reads out of a pack archive, in bytes.
///
/// A zip entry's declared size is attacker-controlled, so the manifest is read through a
/// cap rather than into an unbounded `String`. 8 MiB is far above any real
/// `modrinth.index.json`, whose files list is the only part that grows.
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;

/// Errors from detecting, parsing, or importing a modpack.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The archive is neither a `.mrpack` nor a CurseForge modpack zip.
    #[error("this archive is not a Modrinth or CurseForge modpack")]
    UnknownFormat,
    /// A pack manifest is missing a field, or holds a value this launcher cannot use.
    #[error("could not parse {what}: {detail}")]
    Parse {
        /// Field or document that did not parse.
        what: &'static str,
        /// Why it did not parse.
        detail: String,
    },
    /// A pack file names a download host that is not on the allowlist.
    #[error("download host {0} is not allowed in a modpack")]
    DisallowedHost(String),
    /// A pack file or override entry names a path outside the game directory.
    #[error("unsafe path in modpack: {0}")]
    UnsafePath(String),
    /// Placing a file into the instance failed.
    #[error(transparent)]
    Content(#[from] crate::content::Error),
    /// Installing the pack's mod loader failed.
    #[error(transparent)]
    Loaders(#[from] crate::loaders::Error),
    /// Creating, saving, or deleting the instance failed.
    #[error(transparent)]
    Instance(#[from] crate::instances::Error),
    /// Fetching a pack file failed.
    #[error(transparent)]
    Download(#[from] crate::download::Error),
    /// A source lookup for a pack's files failed.
    #[error(transparent)]
    Sources(#[from] crate::sources::Error),
    /// The pack archive could not be read.
    #[error("could not read zip at {path}: {source}")]
    Zip {
        /// The archive being read.
        path: PathBuf,
        /// The underlying zip error.
        source: zip::result::ZipError,
    },
    /// A filesystem operation failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Which modpack format an archive holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackFormat {
    /// A Modrinth `.mrpack`, described by `modrinth.index.json`.
    Mrpack,
    /// A CurseForge modpack zip, described by `manifest.json`.
    CurseForge,
}

/// One file a pack installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    /// Path under `.minecraft/`, for a pack that names one. A CurseForge file has
    /// `None`: its folder comes from the project's class at the API.
    pub path: Option<String>,
    /// Direct download URL, when the pack publishes one.
    pub url: Option<String>,
    /// Expected sha1 as lowercase hex, when the pack publishes one.
    pub sha1: Option<String>,
    /// Expected size in bytes, when the pack publishes one.
    pub size: Option<u64>,
    /// Source, project id, and file or version id, for a file resolved at an API.
    pub source: Option<(SourceId, String, String)>,
    /// Whether the pack marks this file required. The MVP installs optional files too.
    pub required: bool,
}

/// Everything a pack manifest says, in the shape [`import`] needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackPlan {
    /// Pack display name, which becomes the instance name.
    pub name: String,
    /// Pack version string.
    pub version: String,
    /// Minecraft version the pack runs.
    pub minecraft: String,
    /// Mod loader the pack runs.
    pub loader: Loader,
    /// Version of that mod loader.
    pub loader_version: String,
    /// Files the pack installs.
    pub files: Vec<PackFile>,
    /// Directory prefixes inside the zip to copy over the game directory, in apply
    /// order. A later prefix overwrites what an earlier one wrote.
    pub overrides: Vec<String>,
}

/// One request to import a pack archive as a new instance.
#[derive(Debug, Clone)]
pub struct ImportRequest {
    /// The pack archive on disk.
    pub zip: PathBuf,
    /// Instance name. `None` takes the pack's own name.
    pub name: Option<String>,
    /// Keep the half-built instance when the import fails, for debugging.
    pub keep_partial: bool,
    /// Where the pack came from, recorded in `instance.toml`.
    pub pack_source: Option<PackSource>,
    /// Extra download hosts this one import may fetch `.mrpack` files from, on top of
    /// [`mrpack::ALLOWED_HOSTS`]. Normally empty; a test points it at its mock server.
    pub extra_hosts: Vec<String>,
}

/// What one [`import`] call produced.
#[derive(Debug, Clone)]
pub struct ImportOutcome {
    /// The instance as it stands after the import.
    pub instance: Instance,
    /// How many pack files were placed in the game directory.
    pub installed: usize,
    /// Files the user must fetch from a browser. They do not fail the import.
    pub manual: Vec<ManualDownload>,
}

/// Says which pack format `zip` holds, by looking at its central directory.
///
/// A `modrinth.index.json` at the root is a `.mrpack`. A root `manifest.json` whose
/// `manifestType` is `minecraftModpack` is a CurseForge pack. Anything else, including a
/// manifest nested in a subfolder, is [`Error::UnknownFormat`].
///
/// This blocks on file I/O; async callers wrap it in `tokio::task::spawn_blocking`.
#[tracing::instrument]
pub fn detect(zip: &Path) -> Result<PackFormat, Error> {
    let mut archive = open_zip(zip)?;
    if archive.by_name(MRPACK_INDEX).is_ok() {
        return Ok(PackFormat::Mrpack);
    }
    let text = match archive.by_name(CF_MANIFEST) {
        Ok(mut entry) => read_entry(&mut entry, zip, CF_MANIFEST)?,
        Err(_) => return Err(Error::UnknownFormat),
    };
    let manifest_type = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|doc| doc.get("manifestType")?.as_str().map(str::to_string));
    match manifest_type.as_deref() {
        Some(CF_PACK_TYPE) => Ok(PackFormat::CurseForge),
        _ => Err(Error::UnknownFormat),
    }
}

/// Detects `zip`'s format and parses its manifest into a [`PackPlan`].
///
/// `extra_hosts` adds to the `.mrpack` download allowlist; pass an empty slice for the
/// specification's list on its own. It is [`ImportRequest::extra_hosts`], so a plan read
/// ahead of an import and the import itself accept the same hosts.
///
/// This blocks on file I/O; async callers wrap it in `tokio::task::spawn_blocking`.
#[tracing::instrument]
pub fn read_plan(zip: &Path, extra_hosts: &[String]) -> Result<(PackFormat, PackPlan), Error> {
    let format = detect(zip)?;
    let name = match format {
        PackFormat::Mrpack => MRPACK_INDEX,
        PackFormat::CurseForge => CF_MANIFEST,
    };
    let mut archive = open_zip(zip)?;
    let mut entry = archive.by_name(name).map_err(zip_at(zip))?;
    let text = read_entry(&mut entry, zip, name)?;
    let plan = match format {
        PackFormat::Mrpack => mrpack::parse(&text, extra_hosts)?,
        PackFormat::CurseForge => curseforge::parse(&text)?,
    };
    Ok((format, plan))
}

/// Imports `req.zip` as a new instance: parse, create, install the loader, fetch the
/// files, apply the overrides.
///
/// The instance is created before anything is downloaded, so a failure part-way leaves a
/// half-built directory. That directory is deleted again unless
/// [`ImportRequest::keep_partial`] is set, and the original error is what the caller
/// sees. A file the author opted out of third-party distribution never fails the import;
/// it lands in [`ImportOutcome::manual`] for the user to fetch by hand.
#[tracing::instrument(skip(ctx, instances, loader_ctx, ep, game_defaults))]
pub async fn import(
    ctx: &ContentCtx<'_>,
    instances: &Instances,
    loader_ctx: &LoaderCtx<'_>,
    ep: &LoaderEndpoints,
    game_defaults: &BTreeMap<String, String>,
    req: ImportRequest,
) -> Result<ImportOutcome, Error> {
    let zip = req.zip.clone();
    let extra_hosts = req.extra_hosts.clone();
    let (format, plan) = tokio::task::spawn_blocking(move || read_plan(&zip, &extra_hosts))
        .await
        .map_err(join_at(&req.zip))??;
    ctx.log(format!(
        "parsed {} {} for Minecraft {} on {}",
        plan.name, plan.version, plan.minecraft, plan.loader
    ));

    let name = req.name.clone().unwrap_or_else(|| plan.name.clone());
    let mut instance = instances.create(
        &name,
        &plan.minecraft,
        plan.loader,
        Some(plan.loader_version.clone()),
        game_defaults,
    )?;
    ctx.log(format!("created instance {}", instance.slug));

    match install(ctx, &mut instance, loader_ctx, ep, &plan, format, &req).await {
        Ok((installed, manual)) => Ok(ImportOutcome {
            instance,
            installed,
            manual,
        }),
        Err(err) => {
            if req.keep_partial {
                ctx.log(format!("keeping the partial instance {}", instance.slug));
            } else if let Err(cleanup) = instances.delete(&instance.slug) {
                tracing::warn!(slug = %instance.slug, %cleanup, "could not delete the partial instance");
            }
            Err(err)
        }
    }
}

/// Everything [`import`] does after the instance exists, so one `match` can undo it all.
async fn install(
    ctx: &ContentCtx<'_>,
    instance: &mut Instance,
    loader_ctx: &LoaderCtx<'_>,
    ep: &LoaderEndpoints,
    plan: &PackPlan,
    format: PackFormat,
    req: &ImportRequest,
) -> Result<(usize, Vec<ManualDownload>), Error> {
    // Recording the pack belongs inside this function, not beside `create`: a failing
    // save is then rolled back like any other step, instead of leaving an instance that
    // does not know which pack it came from.
    instance.config.pack = req.pack_source.clone();
    instance.save()?;

    if plan.loader != Loader::None {
        ctx.log(format!(
            "installing {} {}",
            plan.loader, plan.loader_version
        ));
        crate::loaders::install(
            loader_ctx,
            ep,
            plan.loader,
            &plan.minecraft,
            &plan.loader_version,
        )
        .await?;
    }

    let (installed, manual) = match format {
        PackFormat::Mrpack => (
            install_mrpack_files(ctx, instance, &plan.files).await?,
            Vec::new(),
        ),
        PackFormat::CurseForge => install_curseforge_files(ctx, instance, &plan.files).await?,
    };

    let zip = req.zip.clone();
    let prefixes = plan.overrides.clone();
    let game_dir = instance.game_dir();
    ctx.log(format!("applying overrides from {}", prefixes.join(", ")));
    let written = tokio::task::spawn_blocking(move || apply_overrides(&zip, &prefixes, &game_dir))
        .await
        .map_err(join_at(&req.zip))??;
    ctx.log(format!("wrote {written} override files"));
    Ok((installed, manual))
}

/// Downloads and places every file a `.mrpack` names.
///
/// The path in the index decides where the file goes, so nothing here consults a source.
/// Only a file under `mods/`, `resourcepacks/`, or `shaderpacks/` is recorded in
/// `instance.toml`: those are the folders the content list manages. Everything else — a
/// config file, a script — is placed and left unrecorded, the same as an override.
async fn install_mrpack_files(
    ctx: &ContentCtx<'_>,
    instance: &mut Instance,
    files: &[PackFile],
) -> Result<usize, Error> {
    let game_dir = instance.game_dir();
    let total = files.len();
    let mut entries = Vec::new();
    let mut placed = 0usize;

    for (index, file) in files.iter().enumerate() {
        let Some(path) = file.path.as_deref() else {
            continue;
        };
        let dest = safe_join(&game_dir, path).map_err(|_| Error::UnsafePath(path.to_string()))?;
        let file_name = base_name(path).to_string();
        ctx.log(format!("files {}/{total}: {path}", index + 1));

        let spec = VersionFile {
            url: file.url.clone(),
            file_name: file_name.clone(),
            size: file.size,
            sha1: file.sha1.clone(),
            sha512: None,
            fingerprint: None,
            primary: true,
        };
        let (object, sha1) = crate::content::fetch_object(ctx, &spec, file_name.clone()).await?;
        link_into(&object, &dest).await?;
        placed += 1;

        if let Some(kind) = kind_of_path(path) {
            // A `.mrpack` file has no project or version id: it is only a URL and a
            // hash. The sha1 stands in for both, which keeps the entry unique and lets
            // an update check match the file by hash later.
            entries.push(ContentEntry {
                source: SourceId::Modrinth.to_string(),
                project_id: sha1.clone(),
                version_id: sha1.clone(),
                file_name,
                sha1: Some(sha1),
                fingerprint: None,
                kind,
                world: None,
                enabled: true,
            });
        }
    }

    if !entries.is_empty() {
        instance.config.content.extend(entries);
        instance.save()?;
    }
    Ok(placed)
}

/// Resolves a CurseForge pack's file ids and places what it can download.
///
/// The class of each file's project decides its folder, so the projects are fetched too.
/// A file whose `downloadUrl` is null is collected as a [`ManualDownload`] and does not
/// fail the import: the launcher never constructs a CDN URL to work around an author's
/// opt-out.
async fn install_curseforge_files(
    ctx: &ContentCtx<'_>,
    instance: &mut Instance,
    files: &[PackFile],
) -> Result<(usize, Vec<ManualDownload>), Error> {
    let client = ctx
        .sources
        .iter()
        .find_map(|source| source.as_curseforge())
        .ok_or_else(|| {
            Error::Sources(crate::sources::Error::Disabled {
                source_id: SourceId::CurseForge,
                reason: "no CurseForge source is configured".to_string(),
            })
        })?;

    let file_ids: Vec<u32> = files
        .iter()
        .filter_map(|f| f.source.as_ref())
        .filter(|(source, _, _)| *source == SourceId::CurseForge)
        .filter_map(|(_, _, file_id)| file_id.parse().ok())
        .collect();
    let versions = client.files_batch(&file_ids).await?;
    report_unresolved(ctx, &file_ids, &versions);

    let mut mod_ids: Vec<u32> = versions
        .iter()
        .filter_map(|v| v.project_id.parse().ok())
        .collect();
    mod_ids.sort_unstable();
    mod_ids.dedup();
    let projects = client.mods_batch(&mod_ids).await?;

    let total = versions.len();
    let mut manual = Vec::new();
    let mut placed = 0usize;

    for (index, version) in versions.iter().enumerate() {
        let Some(file) = crate::content::primary_file(version) else {
            continue;
        };
        let project = projects.iter().find(|p| p.id == version.project_id);
        // A project whose class did not map to a content kind — one `mods_batch`
        // dropped — is treated as a mod, which is what a pack's file list is made of.
        let kind = project.map_or(ContentKind::Mod, |p| p.kind);
        ctx.log(format!("files {}/{total}: {}", index + 1, file.file_name));

        if file.url.is_none() {
            // A project the batch did not return still gets a page under its own id,
            // which CurseForge redirects to the real slug. `pack_page_url` would be
            // wrong here: this is one file inside a project, not the pack itself.
            let project_page = match project {
                Some(project) => project.page_url.clone(),
                None => crate::sources::page_url(SourceId::CurseForge, kind, &version.project_id),
            };
            let page_url = crate::sources::curseforge::file_page_url(&project_page, &version.id);
            ctx.log(format!(
                "{} must be downloaded by hand from {page_url}",
                file.file_name
            ));
            manual.push(ManualDownload {
                source: SourceId::CurseForge,
                project_id: version.project_id.clone(),
                version_id: version.id.clone(),
                file_name: file.file_name.clone(),
                page_url,
                fingerprint: file.fingerprint,
                sha1: file.sha1.clone(),
            });
            continue;
        }

        let (object, sha1) =
            crate::content::fetch_object(ctx, file, file.file_name.clone()).await?;
        let entry = ContentEntry {
            source: SourceId::CurseForge.to_string(),
            project_id: version.project_id.clone(),
            version_id: version.id.clone(),
            file_name: file.file_name.clone(),
            sha1: Some(sha1),
            fingerprint: file.fingerprint,
            kind,
            world: None,
            enabled: true,
        };
        crate::content::place(instance, object, entry, kind).await?;
        placed += 1;
    }
    Ok((placed, manual))
}

/// Logs the file ids `files_batch` did not answer for.
///
/// CurseForge drops an id it does not know rather than failing, so a pack naming a
/// deleted file installs everything else and says nothing. This says it: those mods are
/// simply not in the instance, and the user has to know before they wonder why.
fn report_unresolved(ctx: &ContentCtx<'_>, asked: &[u32], got: &[Version]) {
    let missing: Vec<String> = asked
        .iter()
        .filter(|id| !got.iter().any(|v| v.id == id.to_string()))
        .map(u32::to_string)
        .collect();
    if !missing.is_empty() {
        ctx.log(format!(
            "CurseForge did not resolve {} of {} pack files: {}",
            missing.len(),
            asked.len(),
            missing.join(", ")
        ));
    }
}

/// Copies every override prefix over `game_dir`, in order, and returns the file count.
///
/// Each entry's path is checked with [`safe_join`], so an archive cannot write outside
/// the game directory. Directory entries are skipped; parents are created on demand. A
/// later prefix overwrites what an earlier one wrote, which is what the `.mrpack`
/// specification asks of `client-overrides/`.
///
/// This blocks on zip and file I/O; async callers wrap it in `tokio::task::spawn_blocking`.
fn apply_overrides(zip: &Path, prefixes: &[String], game_dir: &Path) -> Result<usize, Error> {
    let mut archive = open_zip(zip)?;
    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(zip_at(zip))?;
        names.push(entry.name().to_string());
    }

    let mut written = 0usize;
    for prefix in prefixes {
        for (index, name) in names.iter().enumerate() {
            let Some(rel) = name.strip_prefix(prefix.as_str()) else {
                continue;
            };
            if rel.is_empty() || rel.ends_with('/') {
                continue;
            }
            let dest = safe_join(game_dir, rel).map_err(|_| Error::UnsafePath(rel.to_string()))?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(io_at(parent))?;
            }
            let mut source = archive.by_index(index).map_err(zip_at(zip))?;
            let mut out = std::fs::File::create(&dest).map_err(io_at(&dest))?;
            std::io::copy(&mut source, &mut out).map_err(io_at(&dest))?;
            written += 1;
        }
    }
    Ok(written)
}

/// Downloads one modpack's own archive into the object store.
///
/// `project` is an id or a slug. `version` pins a version by id or number; without one
/// the newest release wins. Neither source exposes a modpack through
/// [`crate::sources::Source::project`] — a modpack is not a [`ContentKind`] — so this
/// goes through each client's pack endpoints instead. The returned [`PackSource`] is
/// what [`ImportRequest::pack_source`] records in `instance.toml`.
#[tracing::instrument(skip(ctx))]
pub async fn fetch_pack(
    ctx: &ContentCtx<'_>,
    source: SourceId,
    project: &str,
    version: Option<&str>,
) -> Result<(PathBuf, PackSource), Error> {
    let versions = match source {
        SourceId::Modrinth => {
            let client = ctx
                .sources
                .iter()
                .find_map(|s| s.as_modrinth())
                .ok_or_else(|| disabled(SourceId::Modrinth))?;
            // The versions endpoint takes an id or a slug, so a modpack needs no
            // project lookup first.
            client.pack_versions(project, None).await?
        }
        SourceId::CurseForge => {
            let client = ctx
                .sources
                .iter()
                .find_map(|s| s.as_curseforge())
                .ok_or_else(|| disabled(SourceId::CurseForge))?;
            let mod_id = client.resolve_pack_id(project).await?;
            client.pack_files(mod_id, None).await?
        }
    };

    let picked = pick_pack_version(&versions, version).ok_or_else(|| {
        Error::Sources(crate::sources::Error::NotFound {
            source_id: source,
            id: match version {
                Some(want) => format!("{project} {want}"),
                None => project.to_string(),
            },
        })
    })?;
    let file = crate::content::primary_file(picked).ok_or_else(|| {
        Error::Sources(crate::sources::Error::BadResponse {
            source_id: source,
            what: "pack version",
            detail: format!("{} has no file", picked.id),
        })
    })?;
    if file.url.is_none() {
        return Err(Error::Sources(crate::sources::Error::ManualDownload {
            page_url: crate::sources::pack_page_url(source, project),
            file_name: file.file_name.clone(),
            fingerprint: file.fingerprint,
            sha1: file.sha1.clone(),
        }));
    }

    ctx.log(format!("fetching pack {}", file.file_name));
    let (object, _) = crate::content::fetch_object(ctx, file, file.file_name.clone()).await?;
    Ok((
        object,
        PackSource {
            source: source.to_string(),
            project_id: picked.project_id.clone(),
            version_id: picked.id.clone(),
        },
    ))
}

/// The version `want` names, or the newest release when `want` is `None`.
///
/// Candidates sort by release channel first and publish time second, the same order
/// [`crate::content::pick_version`] uses. There is no loader or Minecraft filter here: a
/// modpack states both in its own manifest, not in the version's metadata.
fn pick_pack_version<'a>(versions: &'a [Version], want: Option<&str>) -> Option<&'a Version> {
    if let Some(want) = want {
        return versions.iter().find(|v| v.id == want || v.number == want);
    }
    versions.iter().max_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.published.cmp(&b.published))
    })
}

/// The content kind a `.mrpack` path's first segment records as, or `None` when the
/// content list does not manage that folder.
fn kind_of_path(path: &str) -> Option<ContentKind> {
    match path.split('/').next()? {
        "mods" => Some(ContentKind::Mod),
        "resourcepacks" => Some(ContentKind::ResourcePack),
        "shaderpacks" => Some(ContentKind::Shader),
        _ => None,
    }
}

/// The host part of an absolute URL, without userinfo or port.
///
/// This is a comparison helper for the allowlist, not a URL parser: it takes the
/// authority between `://` and the first `/`, `?`, or `#`, drops any `user:pass@`
/// prefix, and unwraps a bracketed IPv6 literal.
fn host_of(url: &str) -> Option<&str> {
    let authority = url.split_once("://")?.1;
    let authority = authority.split(['/', '?', '#']).next()?;
    let authority = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().filter(|h| !h.is_empty());
    }
    authority.split(':').next().filter(|h| !h.is_empty())
}

/// Whether a `.mrpack` may download from `host`.
///
/// The allowlist is [`mrpack::ALLOWED_HOSTS`] plus `extra`, compared without regard to
/// case. `extra` only ever adds: it is [`ImportRequest::extra_hosts`], which a caller
/// sets deliberately, and an empty slice leaves the specification's list exactly as it
/// is.
fn host_allowed(host: &str, extra: &[String]) -> bool {
    mrpack::ALLOWED_HOSTS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
        || extra
            .iter()
            .any(|allowed| allowed.trim().eq_ignore_ascii_case(host))
}

/// Rejects a pack path that would land outside the game directory.
///
/// The check runs at parse time, before an instance exists, so a pack with a `..` or an
/// absolute path never gets as far as creating one. [`safe_join`] decides; the base here
/// is a placeholder, because the answer does not depend on it.
fn check_rel(path: &str) -> Result<(), Error> {
    safe_join(Path::new("/pack"), path)
        .map(|_| ())
        .map_err(|_| Error::UnsafePath(path.to_string()))
}

/// The last `/`-separated segment of a pack path.
fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Hard-links, or copies, a cached object onto `dest`, creating the parent folder.
async fn link_into(object: &Path, dest: &Path) -> Result<(), Error> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(io_at(parent))?;
    }
    let (source, target) = (object.to_path_buf(), dest.to_path_buf());
    tokio::task::spawn_blocking(move || link_or_copy(&source, &target))
        .await
        .map_err(join_at(dest))?
        .map_err(io_at(dest))
}

/// Builds the "this source is not configured" error for a pack lookup.
fn disabled(source_id: SourceId) -> Error {
    Error::Sources(crate::sources::Error::Disabled {
        source_id,
        reason: format!("no {source_id} source is configured"),
    })
}

/// Opens a zip archive for reading.
fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, Error> {
    let file = std::fs::File::open(path).map_err(io_at(path))?;
    zip::ZipArchive::new(file).map_err(zip_at(path))
}

/// Reads one zip entry into a string, refusing anything over [`MAX_MANIFEST_BYTES`].
///
/// `what` names the entry in the error. The read takes one byte more than the cap, so an
/// oversized manifest is reported rather than silently truncated into a parse failure.
fn read_entry(
    entry: &mut impl std::io::Read,
    path: &Path,
    what: &'static str,
) -> Result<String, Error> {
    use std::io::Read as _;

    let mut text = String::new();
    entry
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(io_at(path))?;
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(Error::Parse {
            what,
            detail: "manifest too large".to_string(),
        });
    }
    Ok(text)
}

/// Builds an [`Error::Io`] mapper that carries `path`.
fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Io { path, source }
}

/// Builds an [`Error::Zip`] mapper that carries `path`.
fn zip_at(path: &Path) -> impl Fn(zip::result::ZipError) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Zip {
        path: path.clone(),
        source,
    }
}

/// Turns a `spawn_blocking` join failure into an [`Error::Io`] at `path`.
fn join_at(path: &Path) -> impl FnOnce(tokio::task::JoinError) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Io {
        path,
        source: std::io::Error::other(source),
    }
}
