//! Parsing `manifest.json`, the manifest inside a CurseForge modpack zip.
//!
//! A CurseForge manifest names project and file ids, not URLs: every file is resolved at
//! the API before it can be downloaded, and its folder comes from the project's class.
//! So a [`PackFile`] from here carries a `source` triple and no `path`.

use serde::Deserialize;

use super::{Error, PackFile, PackPlan};
use crate::instances::model::Loader;
use crate::sources::SourceId;

/// The only `manifestType` this parser accepts.
const PACK_TYPE: &str = "minecraftModpack";

/// Override folder used when the manifest names none.
const DEFAULT_OVERRIDES: &str = "overrides";

/// Loader names a `modLoaders[].id` may start with.
const LOADER_NAMES: [(&str, Loader); 4] = [
    ("forge", Loader::Forge),
    ("neoforge", Loader::NeoForge),
    ("fabric", Loader::Fabric),
    ("quilt", Loader::Quilt),
];

/// Parses `manifest.json` into a [`PackPlan`].
///
/// The loader is the entry marked `primary`, or the first one when none is marked. Its
/// `id` is `<loader>-<version>`, for example `forge-47.2.0`.
pub fn parse(json: &str) -> Result<PackPlan, Error> {
    let raw: RawManifest = serde_json::from_str(json).map_err(|source| Error::Parse {
        what: "manifest.json",
        detail: source.to_string(),
    })?;
    if raw.manifest_type != PACK_TYPE {
        return Err(Error::Parse {
            what: "manifestType",
            detail: raw.manifest_type,
        });
    }
    if raw.minecraft.version.is_empty() {
        return Err(Error::Parse {
            what: "minecraft",
            detail: "no version".to_string(),
        });
    }
    let primary = raw
        .minecraft
        .mod_loaders
        .iter()
        .find(|l| l.primary)
        .or_else(|| raw.minecraft.mod_loaders.first())
        .ok_or(Error::Parse {
            what: "modLoaders",
            detail: "empty".to_string(),
        })?;
    let (loader, loader_version) = split_loader_id(&primary.id)?;

    let files = raw
        .files
        .into_iter()
        .map(|f| PackFile {
            path: None,
            url: None,
            sha1: None,
            size: None,
            source: Some((
                SourceId::CurseForge,
                f.project_id.to_string(),
                f.file_id.to_string(),
            )),
            required: f.required,
        })
        .collect();

    let overrides = match raw.overrides.trim_end_matches('/') {
        "" => DEFAULT_OVERRIDES.to_string(),
        name => name.to_string(),
    };

    Ok(PackPlan {
        name: raw.name,
        version: raw.version,
        minecraft: raw.minecraft.version,
        loader,
        loader_version,
        files,
        overrides: vec![format!("{overrides}/")],
    })
}

/// Splits `<loader>-<version>` into a [`Loader`] and its version string.
fn split_loader_id(id: &str) -> Result<(Loader, String), Error> {
    let bad = || Error::Parse {
        what: "modLoaders",
        detail: id.to_string(),
    };
    let (name, version) = id.split_once('-').ok_or_else(bad)?;
    if version.is_empty() {
        return Err(bad());
    }
    let loader = LOADER_NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, l)| *l)
        .ok_or_else(bad)?;
    Ok((loader, version.to_string()))
}

/// The whole `manifest.json` document, as far as the launcher reads it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawManifest {
    #[serde(default)]
    manifest_type: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    overrides: String,
    #[serde(default)]
    minecraft: RawMinecraft,
    #[serde(default)]
    files: Vec<RawFile>,
}

/// The `minecraft` block: the game version and the loaders the pack runs.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMinecraft {
    #[serde(default)]
    version: String,
    #[serde(default)]
    mod_loaders: Vec<RawLoader>,
}

/// One `modLoaders[]` entry.
#[derive(Debug, Deserialize)]
struct RawLoader {
    #[serde(default)]
    id: String,
    #[serde(default)]
    primary: bool,
}

/// One `files[]` entry. The two id keys are capitalized in the format, not camelCase.
#[derive(Debug, Deserialize)]
struct RawFile {
    #[serde(rename = "projectID")]
    project_id: u64,
    #[serde(rename = "fileID")]
    file_id: u64,
    #[serde(default)]
    required: bool,
}
