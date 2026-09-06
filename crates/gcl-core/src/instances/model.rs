//! Serde model for `instance.toml`.
//!
//! Field order here is the on-disk order. See the `instance-model` skill for the schema.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Everything `instance.toml` holds for one instance.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct InstanceConfig {
    /// Display name, as the user typed it.
    pub name: String,
    /// Minecraft version id, e.g. `1.20.1`.
    pub minecraft: String,
    /// Mod loader this instance runs.
    pub loader: Loader,
    /// Loader version, when the loader needs one.
    pub loader_version: Option<String>,
    /// Creation time, RFC 3339 in UTC.
    pub created: String,
    /// Last launch time, RFC 3339 in UTC, if it has ever launched.
    pub last_launched: Option<String>,
    /// Per-instance JVM overrides.
    pub jvm: InstanceJvm,
    /// `options.txt` keys rewritten on every launch.
    pub settings_overrides: BTreeMap<String, String>,
    /// Content installed into this instance.
    pub content: Vec<ContentEntry>,
}

/// Mod loader for an instance.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    /// Vanilla, no loader.
    #[default]
    None,
    /// Fabric Loader.
    Fabric,
    /// Quilt Loader.
    Quilt,
    /// Forge.
    Forge,
    /// NeoForge.
    NeoForge,
}

impl std::fmt::Display for Loader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Loader::None => "none",
            Loader::Fabric => "fabric",
            Loader::Quilt => "quilt",
            Loader::Forge => "forge",
            Loader::NeoForge => "neoforge",
        };
        f.write_str(s)
    }
}

/// Per-instance JVM settings. Unset fields fall back to the global config.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct InstanceJvm {
    /// Minimum heap size in MiB.
    pub min_mib: Option<u32>,
    /// Maximum heap size in MiB.
    pub max_mib: Option<u32>,
    /// Path to a specific `java` executable.
    pub java_path: Option<PathBuf>,
    /// Extra JVM arguments appended after the launcher's own.
    pub extra_args: Vec<String>,
}

/// One installed file: a mod, pack, shader, or world.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct ContentEntry {
    /// Where the file came from: `modrinth`, `curseforge`, or `file`.
    pub source: String,
    /// Project id at the source.
    pub project_id: String,
    /// Version id at the source.
    pub version_id: String,
    /// File name inside the instance folder.
    pub file_name: String,
    /// Lowercase hex sha1 of the file.
    pub sha1: String,
    /// What kind of content this is.
    pub kind: ContentKind,
    /// Whether the file is enabled. Disabled files are renamed with a `.disabled` suffix.
    pub enabled: bool,
}

/// Kind of installed content, which decides the target folder.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    /// A mod jar, installed into `mods/`.
    #[default]
    Mod,
    /// A resource pack, installed into `resourcepacks/`.
    ResourcePack,
    /// A shader pack, installed into `shaderpacks/`.
    Shader,
    /// A data pack, installed into a world's `datapacks/`.
    DataPack,
    /// A world, unpacked into `saves/`.
    World,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_serializes_lowercase() {
        let toml = toml::to_string(&InstanceConfig {
            loader: Loader::NeoForge,
            ..InstanceConfig::default()
        })
        .expect("serialize");
        assert!(toml.contains("loader = \"neoforge\""), "{toml}");
    }

    #[test]
    fn content_kind_serializes_lowercase() {
        let entry = ContentEntry {
            kind: ContentKind::ResourcePack,
            ..ContentEntry::default()
        };
        let toml = toml::to_string(&entry).expect("serialize");
        assert!(toml.contains("kind = \"resourcepack\""), "{toml}");
    }

    #[test]
    fn empty_config_round_trips() {
        let config = InstanceConfig::default();
        let text = toml::to_string(&config).expect("serialize");
        assert_eq!(
            toml::from_str::<InstanceConfig>(&text).expect("parse"),
            config
        );
    }
}
