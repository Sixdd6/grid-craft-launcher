//! Serde model for `instance.toml`.
//!
//! Field order here is the on-disk order. See the `instance-model` skill for the schema.

use std::collections::{BTreeMap, BTreeSet};
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
    /// The modpack this instance was created from, when it came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<PackSource>,
    /// Per-instance JVM overrides.
    pub jvm: InstanceJvm,
    /// `options.txt` keys rewritten on every launch.
    pub settings_overrides: BTreeMap<String, String>,
    /// Content installed into this instance.
    pub content: Vec<ContentEntry>,
}

/// The modpack an instance was installed from.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct PackSource {
    /// Where the pack came from: `modrinth`, `curseforge`, or `file`.
    pub source: String,
    /// Project id at the source.
    pub project_id: String,
    /// Version id at the source.
    pub version_id: String,
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
    /// Garbage collector preset. Absent in `instance.toml` means [`GcPreset::Default`].
    #[serde(default, skip_serializing_if = "GcPreset::is_default")]
    pub gc: GcPreset,
}

/// Garbage collector preset for an instance's JVM.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GcPreset {
    /// No collector flag: the JVM picks its own.
    #[default]
    Default,
    /// Serial collector.
    Serial,
    /// Parallel collector.
    Parallel,
    /// G1 collector.
    G1,
    /// ZGC, non-generational where the JVM offers both modes.
    Zgc,
    /// Generational ZGC.
    ZgcGenerational,
    /// Shenandoah collector.
    Shenandoah,
}

impl GcPreset {
    /// Every preset, in menu order.
    pub const fn all() -> &'static [GcPreset] {
        &[
            GcPreset::Default,
            GcPreset::Serial,
            GcPreset::Parallel,
            GcPreset::G1,
            GcPreset::Zgc,
            GcPreset::ZgcGenerational,
            GcPreset::Shenandoah,
        ]
    }

    /// Whether this is [`GcPreset::Default`], so a default preset writes no key.
    pub fn is_default(&self) -> bool {
        matches!(self, GcPreset::Default)
    }

    /// Display name for a menu.
    pub fn label(&self) -> &'static str {
        match self {
            GcPreset::Default => "Launcher default",
            GcPreset::Serial => "Serial",
            GcPreset::Parallel => "Parallel",
            GcPreset::G1 => "G1",
            GcPreset::Zgc => "ZGC",
            GcPreset::ZgcGenerational => "Generational ZGC",
            GcPreset::Shenandoah => "Shenandoah",
        }
    }

    /// One line saying what the preset trades for what.
    pub fn description(&self) -> &'static str {
        match self {
            GcPreset::Default => "Launcher default: no collector flag, the JVM chooses",
            GcPreset::Serial => "Serial: one thread, lowest overhead, for small heaps",
            GcPreset::Parallel => "Parallel: highest throughput, longer pauses",
            GcPreset::G1 => "G1: balanced pauses and throughput, the usual choice",
            GcPreset::Zgc => "ZGC: low pause times, needs more memory",
            GcPreset::ZgcGenerational => {
                "Generational ZGC: low pause times with less CPU on young objects"
            }
            GcPreset::Shenandoah => "Shenandoah: low pause times, works on smaller heaps than ZGC",
        }
    }

    /// The JVM flags this preset needs on a Java of the given major version.
    ///
    /// Java 21 and 22 ship both ZGC modes behind `ZGenerational`; 23 and later are
    /// generational only, so both ZGC presets pass `-XX:+UseZGC` alone. Below 21 the
    /// generational mode does not exist and the probe refuses the preset.
    pub fn flags(&self, major: u32) -> Vec<String> {
        let flags: &[&str] = match self {
            GcPreset::Default => &[],
            GcPreset::Serial => &["-XX:+UseSerialGC"],
            GcPreset::Parallel => &["-XX:+UseParallelGC"],
            GcPreset::G1 => &["-XX:+UseG1GC"],
            GcPreset::Zgc if (21..=22).contains(&major) => &["-XX:+UseZGC", "-XX:-ZGenerational"],
            GcPreset::Zgc => &["-XX:+UseZGC"],
            GcPreset::ZgcGenerational if (21..=22).contains(&major) => {
                &["-XX:+UseZGC", "-XX:+ZGenerational"]
            }
            GcPreset::ZgcGenerational => &["-XX:+UseZGC"],
            GcPreset::Shenandoah => &["-XX:+UseShenandoahGC"],
        };
        flags.iter().map(|f| (*f).to_string()).collect()
    }
}

/// The presets a JVM can run, from the major version and the flag names its dump listed.
///
/// [`GcPreset::Default`] is always there: it passes no collector flag at all. Java 23 dropped
/// `ZGenerational` and made ZGC generational, so `UseZGC` alone is enough for
/// [`GcPreset::ZgcGenerational`] from 23 on.
pub fn supported_presets(major: u32, flags: &BTreeSet<String>) -> Vec<GcPreset> {
    let has = |name: &str| flags.contains(name);
    GcPreset::all()
        .iter()
        .copied()
        .filter(|preset| match preset {
            GcPreset::Default => true,
            GcPreset::Serial => has("UseSerialGC"),
            GcPreset::Parallel => has("UseParallelGC"),
            GcPreset::G1 => has("UseG1GC"),
            GcPreset::Zgc => has("UseZGC"),
            GcPreset::ZgcGenerational => has("UseZGC") && (major >= 23 || has("ZGenerational")),
            GcPreset::Shenandoah => has("UseShenandoahGC"),
        })
        .collect()
}

impl std::fmt::Display for GcPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            GcPreset::Default => "default",
            GcPreset::Serial => "serial",
            GcPreset::Parallel => "parallel",
            GcPreset::G1 => "g1",
            GcPreset::Zgc => "zgc",
            GcPreset::ZgcGenerational => "zgc_generational",
            GcPreset::Shenandoah => "shenandoah",
        };
        f.write_str(s)
    }
}

/// A string that names no [`GcPreset`].
#[derive(Debug, thiserror::Error)]
#[error("unknown garbage collector preset: {0}")]
pub struct UnknownGcPreset(pub String);

impl std::str::FromStr for GcPreset {
    type Err = UnknownGcPreset;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        GcPreset::all()
            .iter()
            .copied()
            .find(|preset| preset.to_string() == s)
            .ok_or_else(|| UnknownGcPreset(s.to_string()))
    }
}

/// The default for [`ContentEntry::enabled`]: an entry with no `enabled` key is enabled.
fn default_true() -> bool {
    true
}

/// [`ContentEntry::source`] of a file that came from a pack archive, not from a source
/// API. No [`crate::sources::SourceId`] parses it, which is what keeps `content` from
/// asking a source about an entry it has no project id for.
pub const FILE_SOURCE: &str = "file";

/// One installed file: a mod, pack, shader, or world.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
    /// Project title at the source, for display. Absent in an `instance.toml` written
    /// before this key existed; `content::check_updates` backfills it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Lowercase hex sha1 of the file, when the source publishes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// CurseForge murmur2 fingerprint of the file, when the source publishes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<u32>,
    /// What kind of content this is.
    pub kind: ContentKind,
    /// Target world folder name, for a data pack installed into one world's `datapacks/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<String>,
    /// Whether the file is enabled. Disabled files are renamed with a `.disabled` suffix.
    /// Absent in `instance.toml` means enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for ContentEntry {
    fn default() -> Self {
        ContentEntry {
            source: String::new(),
            project_id: String::new(),
            version_id: String::new(),
            file_name: String::new(),
            title: None,
            sha1: None,
            fingerprint: None,
            kind: ContentKind::default(),
            world: None,
            enabled: true,
        }
    }
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

impl ContentKind {
    /// Parses the lowercase form used in `instance.toml` and source APIs.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mod" => Some(ContentKind::Mod),
            "resourcepack" => Some(ContentKind::ResourcePack),
            "shader" => Some(ContentKind::Shader),
            "datapack" => Some(ContentKind::DataPack),
            "world" => Some(ContentKind::World),
            _ => None,
        }
    }
}

impl std::fmt::Display for ContentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ContentKind::Mod => "mod",
            ContentKind::ResourcePack => "resourcepack",
            ContentKind::Shader => "shader",
            ContentKind::DataPack => "datapack",
            ContentKind::World => "world",
        };
        f.write_str(s)
    }
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
    fn an_entry_without_an_enabled_key_is_enabled() {
        let entry: ContentEntry = toml::from_str(
            "source = \"modrinth\"\nproject_id = \"p\"\nversion_id = \"v\"\n\
             file_name = \"sodium.jar\"\nsha1 = \"abc\"\nkind = \"mod\"\n",
        )
        .expect("parse");
        assert!(entry.enabled);
        assert!(ContentEntry::default().enabled);
    }

    #[test]
    fn content_entry_title_defaults_to_none_for_old_toml() {
        // An `instance.toml` written before the `title` key existed still parses.
        let entry: ContentEntry = toml::from_str(
            "source = \"modrinth\"\nproject_id = \"p\"\nversion_id = \"v\"\n\
             file_name = \"sodium.jar\"\nkind = \"mod\"\n",
        )
        .expect("parse");
        assert_eq!(entry.title, None);
        assert_eq!(ContentEntry::default().title, None);
        // A `None` title writes no key at all.
        let toml = toml::to_string(&entry).expect("serialize");
        assert!(!toml.contains("title"), "{toml}");
    }

    #[test]
    fn content_kind_display_and_parse_round_trip() {
        for kind in [
            ContentKind::Mod,
            ContentKind::ResourcePack,
            ContentKind::Shader,
            ContentKind::DataPack,
            ContentKind::World,
        ] {
            assert_eq!(ContentKind::parse(&kind.to_string()), Some(kind));
        }
        assert_eq!(ContentKind::parse("bogus"), None);
    }

    #[test]
    fn an_explicit_enabled_false_is_kept() {
        let entry: ContentEntry =
            toml::from_str("file_name = \"x.jar\"\nenabled = false\n").expect("parse");
        assert!(!entry.enabled);
    }

    #[test]
    fn a_config_with_a_pack_and_content_round_trips_and_snapshots() {
        let config = InstanceConfig {
            name: "My Pack".to_string(),
            minecraft: "1.20.1".to_string(),
            loader: Loader::Fabric,
            loader_version: Some("0.15.11".to_string()),
            created: "2026-09-06T12:00:00Z".to_string(),
            pack: Some(PackSource {
                source: "modrinth".to_string(),
                project_id: "AANobbMI".to_string(),
                version_id: "abc123".to_string(),
            }),
            content: vec![
                ContentEntry {
                    source: "modrinth".to_string(),
                    project_id: "AANobbMI".to_string(),
                    version_id: "abc123".to_string(),
                    file_name: "sodium.jar".to_string(),
                    sha1: Some("da39a3ee5e6b4b0d3255bfef95601890afd80709".to_string()),
                    ..ContentEntry::default()
                },
                ContentEntry {
                    source: "curseforge".to_string(),
                    project_id: "238222".to_string(),
                    version_id: "4567".to_string(),
                    file_name: "pack.zip".to_string(),
                    fingerprint: Some(1234567890),
                    kind: ContentKind::DataPack,
                    world: Some("New World".to_string()),
                    ..ContentEntry::default()
                },
            ],
            ..InstanceConfig::default()
        };
        let text = toml::to_string(&config).expect("serialize");
        assert_eq!(
            toml::from_str::<InstanceConfig>(&text).expect("parse"),
            config
        );
        insta::assert_snapshot!("instance_toml_full", text);
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

    #[test]
    fn a_jvm_table_without_gc_loads_as_default_and_is_not_written_back() {
        let jvm: InstanceJvm = toml::from_str("min_mib = 2048\nmax_mib = 6144\n").expect("parse");
        assert_eq!(jvm.gc, GcPreset::Default);
        let text = toml::to_string(&jvm).expect("serialize");
        assert!(!text.contains("gc"), "{text}");
    }

    #[test]
    fn a_non_default_gc_is_written_and_read_back() {
        let jvm = InstanceJvm {
            gc: GcPreset::ZgcGenerational,
            ..InstanceJvm::default()
        };
        let text = toml::to_string(&jvm).expect("serialize");
        assert!(text.contains("gc = \"zgc_generational\""), "{text}");
        assert_eq!(toml::from_str::<InstanceJvm>(&text).expect("parse"), jvm);
    }

    #[test]
    fn every_gc_token_round_trips_through_toml_and_from_str() {
        let expected = [
            "default",
            "serial",
            "parallel",
            "g1",
            "zgc",
            "zgc_generational",
            "shenandoah",
        ];
        let tokens: Vec<String> = GcPreset::all().iter().map(|p| p.to_string()).collect();
        assert_eq!(tokens, expected);
        for preset in GcPreset::all() {
            assert_eq!(
                preset.to_string().parse::<GcPreset>().expect("parse"),
                *preset
            );
            let jvm = InstanceJvm {
                gc: *preset,
                ..InstanceJvm::default()
            };
            let text = toml::to_string(&jvm).expect("serialize");
            let back: InstanceJvm = toml::from_str(&text).expect("parse");
            assert_eq!(back.gc, *preset, "{text}");
        }
        assert!("bogus".parse::<GcPreset>().is_err());
    }

    #[test]
    fn flags_match_the_preset_and_major_table() {
        let cases: &[(GcPreset, u32, &[&str])] = &[
            (GcPreset::Default, 21, &[]),
            (GcPreset::Serial, 17, &["-XX:+UseSerialGC"]),
            (GcPreset::Parallel, 17, &["-XX:+UseParallelGC"]),
            (GcPreset::G1, 8, &["-XX:+UseG1GC"]),
            (GcPreset::Shenandoah, 17, &["-XX:+UseShenandoahGC"]),
            (GcPreset::Zgc, 17, &["-XX:+UseZGC"]),
            (GcPreset::Zgc, 21, &["-XX:+UseZGC", "-XX:-ZGenerational"]),
            (GcPreset::Zgc, 22, &["-XX:+UseZGC", "-XX:-ZGenerational"]),
            (GcPreset::Zgc, 23, &["-XX:+UseZGC"]),
            (GcPreset::Zgc, 25, &["-XX:+UseZGC"]),
            (GcPreset::ZgcGenerational, 17, &["-XX:+UseZGC"]),
            (
                GcPreset::ZgcGenerational,
                21,
                &["-XX:+UseZGC", "-XX:+ZGenerational"],
            ),
            (
                GcPreset::ZgcGenerational,
                22,
                &["-XX:+UseZGC", "-XX:+ZGenerational"],
            ),
            (GcPreset::ZgcGenerational, 23, &["-XX:+UseZGC"]),
            (GcPreset::ZgcGenerational, 25, &["-XX:+UseZGC"]),
        ];
        for (preset, major, expected) in cases {
            let flags = preset.flags(*major);
            let got: Vec<&str> = flags.iter().map(String::as_str).collect();
            assert_eq!(got.as_slice(), *expected, "{preset} on java {major}");
        }
    }

    #[test]
    fn every_preset_has_a_one_line_label_and_description() {
        for preset in GcPreset::all() {
            assert!(!preset.label().is_empty(), "{preset}");
            let description = preset.description();
            assert!(!description.is_empty(), "{preset}");
            assert!(!description.contains('\n'), "{preset}");
        }
    }

    fn flag_set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn supported_presets_always_offers_the_default() {
        assert_eq!(
            supported_presets(17, &BTreeSet::new()),
            vec![GcPreset::Default]
        );
    }

    #[test]
    fn zgc_without_the_generational_switch_is_generational_only_from_23() {
        let flags = flag_set(&["UseZGC"]);
        let at_21 = supported_presets(21, &flags);
        assert!(at_21.contains(&GcPreset::Zgc));
        assert!(!at_21.contains(&GcPreset::ZgcGenerational));

        let at_23 = supported_presets(23, &flags);
        assert!(at_23.contains(&GcPreset::Zgc));
        assert!(at_23.contains(&GcPreset::ZgcGenerational));
    }

    #[test]
    fn the_generational_switch_offers_both_zgc_presets_on_21() {
        let flags = flag_set(&["UseZGC", "ZGenerational"]);
        let at_21 = supported_presets(21, &flags);
        assert!(at_21.contains(&GcPreset::Zgc));
        assert!(at_21.contains(&GcPreset::ZgcGenerational));
    }

    #[test]
    fn supported_presets_keeps_menu_order_and_reads_one_flag_each() {
        let flags = flag_set(&["UseSerialGC", "UseParallelGC", "UseG1GC", "UseShenandoahGC"]);
        assert_eq!(
            supported_presets(17, &flags),
            vec![
                GcPreset::Default,
                GcPreset::Serial,
                GcPreset::Parallel,
                GcPreset::G1,
                GcPreset::Shenandoah,
            ]
        );
    }
}
