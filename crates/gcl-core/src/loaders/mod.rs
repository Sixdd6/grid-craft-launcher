//! Mod loader version discovery and headless installation into the shared version cache.
//!
//! Every loader writes `cache/versions/<id>.json` with `inheritsFrom` set to the Minecraft
//! version, so a launch resolves loader and vanilla through the same Mojang merge.

pub mod fabric;
pub mod fabriclike;
pub mod forge;
pub mod forgelike;
pub mod neoforge;
pub mod processors;
pub mod quilt;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use crate::instances::model::Loader;
pub use forgelike::{
    DataEntry, DataMap, InstallProfile, InstallerJar, Processor, build_data_map, library_specs,
    substitute,
};
pub use processors::{JavaRunner, ProcessRunner, outputs_current, run_processors};

use crate::download::DownloadCtx;
use crate::http::HttpClient;
use crate::java::JavaInstall;
use crate::paths::Root;

/// Errors from listing or installing a mod loader.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A request to a loader's metadata or maven host failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// Downloading a loader library or installer failed.
    #[error(transparent)]
    Download(#[from] crate::download::Error),
    /// Reading or merging Mojang metadata failed.
    #[error(transparent)]
    Mojang(#[from] crate::mojang::Error),
    /// A path in the app root layout could not be built.
    #[error(transparent)]
    Paths(#[from] crate::paths::Error),
    /// Reading or writing a cache file failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being read or written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A document was not the JSON we expect.
    #[error("{url}: invalid JSON: {source}")]
    Json {
        /// The URL or file the JSON came from.
        url: String,
        /// The underlying parse error.
        source: serde_json::Error,
    },
    /// The loader publishes no such build for this Minecraft version.
    #[error("no {loader} version {version} for Minecraft {mc}")]
    NoSuchVersion {
        /// The loader that was asked.
        loader: Loader,
        /// The Minecraft version.
        mc: String,
        /// The loader version that was asked for.
        version: String,
    },
    /// The loader has no builds at all for this Minecraft version.
    #[error("Minecraft {0} has no {1} builds")]
    Unsupported(String, Loader),
    /// A pre-1.13 Forge installer, which has no processors to run.
    #[error("legacy Forge installer (pre-1.13) is not supported")]
    LegacyInstaller,
    /// An installer processor exited non-zero.
    #[error("processor {main} failed with exit {code}: see {log}")]
    ProcessorFailed {
        /// Main class of the processor.
        main: String,
        /// Exit code the JVM returned.
        code: i32,
        /// Log file holding the processor's output.
        log: PathBuf,
    },
    /// A processor ran but did not produce the file it promised.
    #[error("processor output {path} is missing")]
    ProcessorOutput {
        /// The output the processor never wrote.
        path: PathBuf,
    },
    /// A processor output has the wrong sha1 and its archive does not read back.
    #[error(
        "processor output {path} hash mismatch and the archive is damaged \
         (expected {expected}, got {actual}): {source}"
    )]
    ProcessorOutputDamaged {
        /// The output that is wrong.
        path: PathBuf,
        /// The sha1 the install profile names.
        expected: String,
        /// The sha1 the file on disk has.
        actual: String,
        /// Why the archive did not read back.
        source: std::io::Error,
    },
    /// A processor output has the wrong sha1 and is not an archive to check further.
    #[error("processor output {path} hash mismatch (expected {expected}, got {actual})")]
    ProcessorOutputHash {
        /// The output that is wrong.
        path: PathBuf,
        /// The sha1 the install profile names.
        expected: String,
        /// The sha1 the file on disk has.
        actual: String,
    },
    /// A Forge-like install needs a JVM to run its processors.
    #[error("java runtime required to install {0}")]
    JavaRequired(Loader),
    /// Reading an installer archive failed.
    #[error("zip error in {path}: {source}")]
    Zip {
        /// The archive being read.
        path: PathBuf,
        /// The underlying zip error.
        source: zip::result::ZipError,
    },
    /// An installer named a path that escapes the directory it is written into.
    #[error("unsafe path in installer: {0}")]
    UnsafePath(String),
    /// A processor argument used a `{KEY}` the installer's data map does not define.
    #[error("installer data map has no key {0}")]
    UnknownDataKey(String),
    /// A processor jar's manifest declares no `Main-Class`.
    #[error("processor jar {0} declares no Main-Class")]
    ProcessorMain(String),
}

/// One installable loader build for a given Minecraft version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoaderVersion {
    /// Loader version string, e.g. `0.19.5`.
    pub version: String,
    /// True when the loader publishes this build as stable.
    pub stable: bool,
    /// True for the single build the launcher offers first.
    pub recommended: bool,
}

/// Everything a loader install borrows: HTTP, the app root, downloads, and a JVM.
#[derive(Clone, Copy)]
pub struct LoaderCtx<'a> {
    /// Shared HTTP client.
    pub http: &'a HttpClient,
    /// App root, which owns the version and library cache.
    pub root: &'a Root,
    /// Download context for library and installer fetches.
    pub dl: &'a DownloadCtx<'a>,
    /// Java to run installer processors with. Required for Forge and NeoForge.
    pub java: Option<&'a JavaInstall>,
    /// How installer processors are run. Required for Forge and NeoForge.
    pub runner: Option<&'a dyn ProcessRunner>,
    /// Mojang client used to install the vanilla version a Forge-like build patches.
    /// `None` builds one against the production endpoints.
    pub mojang: Option<&'a crate::mojang::Mojang>,
}

/// Base URLs for every loader's metadata and maven hosts. Tests override them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderEndpoints {
    /// Fabric meta base, without the `/v2` path.
    pub fabric: String,
    /// Quilt meta base, without the `/v3` path.
    pub quilt: String,
    /// Forge metadata host, which serves the version list and the promotions.
    pub forge_meta: String,
    /// Forge maven host, which serves versions and installers.
    pub forge_maven: String,
    /// NeoForge maven host, which serves versions and installers.
    pub neoforge: String,
}

impl Default for LoaderEndpoints {
    fn default() -> Self {
        LoaderEndpoints {
            fabric: fabric::BASE.to_string(),
            quilt: quilt::BASE.to_string(),
            forge_meta: forge::META.to_string(),
            forge_maven: forge::MAVEN.to_string(),
            neoforge: neoforge::MAVEN.to_string(),
        }
    }
}

/// Orders loader builds newest first by their version string.
///
/// Loader hosts publish their lists in upstream order, which is not always sorted: NeoForge's
/// API returns whole release lines out of order. Sorting here makes "newest" mean the same
/// thing for every loader.
pub(crate) fn sort_newest_first(versions: &mut [LoaderVersion]) {
    versions.sort_by(|a, b| compare_versions(&b.version, &a.version));
}

/// One component of a version string: a number, or a word such as a `beta` suffix.
#[derive(Debug, PartialEq, Eq)]
enum Part {
    /// A numeric component, compared by value rather than by text.
    Num(u64),
    /// A non-numeric component, which marks a prerelease.
    Text(String),
}

/// Splits a version on `.` and `-` into comparable components.
fn parts(version: &str) -> Vec<Part> {
    version
        .split(['.', '-', '+', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| match p.parse::<u64>() {
            Ok(n) => Part::Num(n),
            Err(_) => Part::Text(p.to_ascii_lowercase()),
        })
        .collect()
}

/// Compares two version strings component by component. A prerelease sorts below its release.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let (a, b) = (parts(a), parts(b));
    for index in 0..a.len().max(b.len()) {
        let ordering = match (a.get(index), b.get(index)) {
            (Some(Part::Num(x)), Some(Part::Num(y))) => x.cmp(y),
            (Some(Part::Text(x)), Some(Part::Text(y))) => x.cmp(y),
            // A number outranks a word, so `1.0` beats `1.0-beta` on the same component.
            (Some(Part::Num(_)), Some(Part::Text(_))) => Ordering::Greater,
            (Some(Part::Text(_)), Some(Part::Num(_))) => Ordering::Less,
            // A longer version wins only when the extra component is numeric.
            (Some(Part::Num(_)), None) | (None, Some(Part::Text(_))) => Ordering::Greater,
            (Some(Part::Text(_)), None) | (None, Some(Part::Num(_))) => Ordering::Less,
            (None, None) => Ordering::Equal,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

/// Builds the cached version id one loader build installs under.
pub fn version_id(loader: Loader, mc: &str, loader_version: &str) -> String {
    match loader {
        Loader::None => mc.to_string(),
        Loader::Fabric => format!("fabric-loader-{loader_version}-{mc}"),
        Loader::Quilt => format!("quilt-loader-{loader_version}-{mc}"),
        Loader::Forge => format!("{mc}-forge-{loader_version}"),
        Loader::NeoForge => format!("neoforge-{loader_version}"),
    }
}

/// True when a loader's profile needs both its own and vanilla's copy of a library.
pub fn keep_both_libraries(loader: Loader) -> bool {
    matches!(loader, Loader::Forge | Loader::NeoForge)
}

/// Lists the loader builds available for one Minecraft version, newest first.
#[tracing::instrument(skip(ctx, ep))]
pub async fn list_versions(
    ctx: &LoaderCtx<'_>,
    ep: &LoaderEndpoints,
    loader: Loader,
    mc: &str,
) -> Result<Vec<LoaderVersion>, Error> {
    match loader {
        Loader::Fabric => fabric::list(ctx, &ep.fabric, mc).await,
        Loader::Quilt => quilt::list(ctx, &ep.quilt, mc).await,
        Loader::Forge => forge::list(ctx, &ep.forge_meta, mc).await,
        Loader::NeoForge => neoforge::list(ctx, &ep.neoforge, mc).await,
        Loader::None => Err(Error::Unsupported(mc.to_string(), loader)),
    }
}

/// Installs one loader build into the version cache and returns its version id.
#[tracing::instrument(skip(ctx, ep))]
pub async fn install(
    ctx: &LoaderCtx<'_>,
    ep: &LoaderEndpoints,
    loader: Loader,
    mc: &str,
    loader_version: &str,
) -> Result<String, Error> {
    match loader {
        Loader::Fabric => fabric::install(ctx, &ep.fabric, mc, loader_version).await,
        Loader::Quilt => quilt::install(ctx, &ep.quilt, mc, loader_version).await,
        Loader::Forge => {
            let url = forge::installer_url(&ep.forge_maven, mc, loader_version);
            forgelike::install(ctx, loader, mc, loader_version, url).await
        }
        Loader::NeoForge => {
            let url = neoforge::installer_url(&ep.neoforge, loader_version);
            forgelike::install(ctx, loader, mc, loader_version, url).await
        }
        Loader::None => Err(Error::Unsupported(mc.to_string(), loader)),
    }
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
