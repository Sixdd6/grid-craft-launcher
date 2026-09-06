//! Parsing `modrinth.index.json`, the manifest inside a Modrinth `.mrpack`.
//!
//! The index names every file with a URL, a sha1, and a path under `.minecraft/`, so a
//! plan built here needs no API call to install. Everything the parser refuses — a
//! download host off the allowlist, a path that escapes the game directory, a file with
//! no sha1 — is refused before an instance exists, so a bad pack never half-installs.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{Error, PackFile, PackPlan, check_rel, host_allowed, host_of};
use crate::instances::model::Loader;

/// Hosts a `.mrpack` may name in a file's `downloads`.
///
/// The list is from the mrpack specification. A pack that points anywhere else is
/// [`Error::DisallowedHost`]: an arbitrary host would turn a pack into a way to make the
/// launcher fetch and run any file at all.
pub const ALLOWED_HOSTS: [&str; 4] = [
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];

/// Override directories, in the order they are copied over the game directory.
const OVERRIDES: [&str; 2] = ["overrides/", "client-overrides/"];

/// The `dependencies` key each loader uses, and the loader it means.
const LOADER_KEYS: [(&str, Loader); 4] = [
    ("fabric-loader", Loader::Fabric),
    ("quilt-loader", Loader::Quilt),
    ("forge", Loader::Forge),
    ("neoforge", Loader::NeoForge),
];

/// Value of `env.client` for a file the client must not install.
const UNSUPPORTED: &str = "unsupported";

/// Parses `modrinth.index.json` into a [`PackPlan`].
///
/// `dependencies` must name `minecraft` and exactly one loader: a modpack with no
/// loader is not something this MVP installs, so it is a parse error rather than a
/// vanilla instance. Files whose `env.client` is `unsupported` are dropped; `optional`
/// files are kept, which is what the launcher's MVP promises.
pub fn parse(json: &str) -> Result<PackPlan, Error> {
    let raw: RawIndex = serde_json::from_str(json).map_err(|source| Error::Parse {
        what: "modrinth.index.json",
        detail: source.to_string(),
    })?;
    if raw.name.is_empty() {
        return Err(Error::Parse {
            what: "name",
            detail: "missing".to_string(),
        });
    }
    let minecraft = raw
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| deps("no minecraft"))?;
    let (loader, loader_version) = loader_of(&raw.dependencies)?;

    let mut files = Vec::with_capacity(raw.files.len());
    for file in raw.files {
        if file.env.client.as_deref() == Some(UNSUPPORTED) {
            continue;
        }
        check_rel(&file.path)?;
        let url = file.downloads.into_iter().next().ok_or(Error::Parse {
            what: "downloads",
            detail: file.path.clone(),
        })?;
        let host = host_of(&url).ok_or_else(|| Error::DisallowedHost(url.clone()))?;
        if !host_allowed(host) {
            return Err(Error::DisallowedHost(host.to_string()));
        }
        let sha1 = file.hashes.sha1.ok_or(Error::Parse {
            what: "hashes",
            detail: file.path.clone(),
        })?;
        files.push(PackFile {
            path: Some(file.path),
            url: Some(url),
            sha1: Some(sha1),
            size: file.file_size,
            source: None,
            required: file.env.client.as_deref() != Some("optional"),
        });
    }

    Ok(PackPlan {
        name: raw.name,
        version: raw.version_id,
        minecraft,
        loader,
        loader_version,
        files,
        overrides: OVERRIDES.iter().map(|s| (*s).to_string()).collect(),
    })
}

/// The one loader `dependencies` names, or a [`Error::Parse`] when it names none or two.
fn loader_of(dependencies: &BTreeMap<String, String>) -> Result<(Loader, String), Error> {
    let mut found: Vec<(Loader, String)> = Vec::new();
    for (key, loader) in LOADER_KEYS {
        if let Some(version) = dependencies.get(key) {
            found.push((loader, version.clone()));
        }
    }
    match found.len() {
        0 => Err(deps("no loader")),
        1 => Ok(found.remove(0)),
        n => Err(deps(&format!("{n} loaders"))),
    }
}

/// Builds a `dependencies` parse error.
fn deps(detail: &str) -> Error {
    Error::Parse {
        what: "dependencies",
        detail: detail.to_string(),
    }
}

/// The whole `modrinth.index.json` document, as far as the launcher reads it.
#[derive(Debug, Deserialize)]
struct RawIndex {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "versionId")]
    version_id: String,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    files: Vec<RawFile>,
}

/// One `files[]` entry.
#[derive(Debug, Deserialize)]
struct RawFile {
    path: String,
    #[serde(default)]
    hashes: RawHashes,
    #[serde(default)]
    env: RawEnv,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default, rename = "fileSize")]
    file_size: Option<u64>,
}

/// The hashes block. Only sha1 is used; the object store is sha1-addressed.
#[derive(Debug, Default, Deserialize)]
struct RawHashes {
    #[serde(default)]
    sha1: Option<String>,
}

/// The `env` block. A file with no `env` is required on both sides.
#[derive(Debug, Default, Deserialize)]
struct RawEnv {
    #[serde(default)]
    client: Option<String>,
}
