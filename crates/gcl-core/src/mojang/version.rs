//! Serde types for a Mojang version JSON and for maven coordinates.
//!
//! Every field absent from old versions or from loader profiles is optional, so the same
//! types parse vanilla JSON and Fabric/Forge/NeoForge profiles.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Error;
use super::rules::Rule;

/// A parsed version JSON: vanilla, or a loader profile with `inheritsFrom`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    /// The version id this JSON describes.
    pub id: String,
    /// Parent version id a loader profile builds on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    /// Java class the game starts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
    /// Modern split game and JVM arguments (1.13+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Arguments>,
    /// Pre-1.13 single-string game arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_arguments: Option<String>,
    /// Libraries on the classpath, before rule filtering.
    #[serde(default)]
    pub libraries: Vec<Library>,
    /// Client and server jar downloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<Downloads>,
    /// Pointer to the asset index for this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<AssetIndexRef>,
    /// Asset index id, duplicated from `asset_index.id` by Mojang.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    /// Java runtime this version needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_version: Option<JavaVersion>,
    /// log4j configuration this version publishes, when it publishes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingConfig>,
    /// Release channel: `release`, `snapshot`, `old_beta`, `old_alpha`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// ISO 8601 release timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_time: Option<String>,
}

/// Modern argument lists, split into game and JVM sides.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct Arguments {
    /// Arguments passed to the game's main class.
    #[serde(default)]
    pub game: Vec<Argument>,
    /// Arguments passed to the JVM.
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

/// One argument entry: a plain string, or one guarded by rules.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Argument {
    /// An argument that is always present.
    Plain(String),
    /// An argument present only when its rules allow it.
    Conditional {
        /// Rules guarding the argument.
        rules: Vec<Rule>,
        /// One argument or several.
        value: ArgValue,
    },
}

/// The value of a conditional argument: one string or a list.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgValue {
    /// A single argument.
    One(String),
    /// Several arguments, kept in order.
    Many(Vec<String>),
}

impl ArgValue {
    /// The argument strings this value holds.
    pub fn as_slice(&self) -> &[String] {
        match self {
            ArgValue::One(s) => std::slice::from_ref(s),
            ArgValue::Many(v) => v,
        }
    }
}

/// One entry of `libraries[]`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Library {
    /// Maven coordinate, `group:artifact:version[:classifier][@ext]`.
    pub name: String,
    /// Artifact and classifier downloads, when the source publishes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibraryDownloads>,
    /// Maven repository base URL, used by loader profiles without a `downloads` block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Checksum published outside a `downloads` block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// Size published outside a `downloads` block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Rules deciding whether this library applies here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Old-style map from OS name to the natives classifier to extract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<BTreeMap<String, String>>,
    /// Paths to skip when extracting natives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extract>,
}

/// The `downloads` block of a library.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct LibraryDownloads {
    /// The main jar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    /// Classifier jars, keyed by classifier name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<BTreeMap<String, Artifact>>,
}

/// One downloadable file with its checksum.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Artifact {
    /// Path below the libraries directory. Absent for client and server jars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Lowercase hex sha1 of the file.
    pub sha1: String,
    /// Size in bytes.
    pub size: u64,
    /// Download URL.
    pub url: String,
}

/// Native extraction filter.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct Extract {
    /// Archive path prefixes to skip.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// The `downloads` block of a version JSON.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Downloads {
    /// The client jar.
    pub client: Artifact,
    /// The server jar, when published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Artifact>,
    /// Obfuscation mappings for the client, when published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_mappings: Option<Artifact>,
}

/// Pointer to the asset index for a version.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    /// Asset index id, used as its cache file name.
    pub id: String,
    /// Lowercase hex sha1 of the index JSON.
    pub sha1: String,
    /// Size of the index JSON in bytes.
    pub size: u64,
    /// Total size of every object the index names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_size: Option<u64>,
    /// Download URL of the index JSON.
    pub url: String,
}

/// The Java runtime a version asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    /// Mojang runtime component name, for example `java-runtime-gamma`.
    pub component: String,
    /// Major Java version, for example 17.
    pub major_version: u32,
}

/// The `logging` block of a version JSON. Every side is optional: old versions publish
/// none, and a loader profile can publish an empty object.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct LoggingConfig {
    /// Client-side log4j configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<LoggingClient>,
}

/// The client half of a [`LoggingConfig`]: the JVM argument and the file it points at.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LoggingClient {
    /// JVM argument template holding `${path}`, for example
    /// `-Dlog4j.configurationFile=${path}`.
    pub argument: String,
    /// The configuration file the argument points at.
    pub file: LoggingFile,
    /// Configuration format, for example `log4j2-xml`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// One downloadable log4j configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LoggingFile {
    /// File name the config is cached under, for example `client-1.12.xml`.
    pub id: String,
    /// Lowercase hex sha1 of the file.
    pub sha1: String,
    /// Size in bytes.
    pub size: u64,
    /// Where to download the file from.
    pub url: String,
}

/// A maven coordinate parsed from a library `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenCoord {
    /// Group id, dot separated.
    pub group: String,
    /// Artifact id.
    pub artifact: String,
    /// Version string.
    pub version: String,
    /// Classifier, when the coordinate names one.
    pub classifier: Option<String>,
    /// File extension, `jar` unless the coordinate ends with `@ext`.
    pub ext: String,
}

impl MavenCoord {
    /// Parses `group:artifact:version[:classifier][@ext]`.
    pub fn parse(name: &str) -> Result<Self, Error> {
        let bad = || Error::BadMavenCoord(name.to_string());
        let (body, ext) = match name.split_once('@') {
            Some((body, ext)) if !ext.is_empty() => (body, ext.to_string()),
            Some(_) => return Err(bad()),
            None => (name, "jar".to_string()),
        };
        let mut parts = body.split(':');
        let group = parts.next().filter(|s| !s.is_empty()).ok_or_else(bad)?;
        let artifact = parts.next().filter(|s| !s.is_empty()).ok_or_else(bad)?;
        let version = parts.next().filter(|s| !s.is_empty()).ok_or_else(bad)?;
        let classifier = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
        if parts.next().is_some() {
            return Err(bad());
        }
        Ok(MavenCoord {
            group: group.to_string(),
            artifact: artifact.to_string(),
            version: version.to_string(),
            classifier,
            ext,
        })
    }

    /// Repository-relative path, `group/with/slashes/artifact/version/artifact-version[-classifier].ext`.
    pub fn path(&self) -> String {
        let mut file = format!("{}-{}", self.artifact, self.version);
        if let Some(classifier) = &self.classifier {
            file.push('-');
            file.push_str(classifier);
        }
        format!(
            "{}/{}/{}/{}.{}",
            self.group.replace('.', "/"),
            self.artifact,
            self.version,
            file,
            self.ext
        )
    }

    /// Identity key: `group:artifact`, ignoring version, classifier, and extension.
    pub fn key(&self) -> String {
        format!("{}:{}", self.group, self.artifact)
    }

    /// Key used to merge libraries: `group:artifact:classifier`, empty when there is none.
    ///
    /// The plain jar and each natives classifier are separate classpath entries, so they
    /// must replace each other one for one rather than share a slot.
    pub fn merge_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.group,
            self.artifact,
            self.classifier.as_deref().unwrap_or("")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fixture(name: &str) -> VersionJson {
        let text = std::fs::read_to_string(format!("../../tests/fixtures/mojang/{name}.json"))
            .expect("fixture readable");
        serde_json::from_str(&text).expect("fixture parses")
    }

    #[test]
    fn parses_modern_version_json() {
        let v = parse_fixture("1.20.1");
        insta::assert_json_snapshot!(serde_json::json!({
            "id": v.id,
            "main_class": v.main_class,
            "libraries": v.libraries.len(),
            "java_version": v.java_version,
            "assets": v.assets,
            "game_args": v.arguments.as_ref().map(|a| a.game.len()),
            "jvm_args": v.arguments.as_ref().map(|a| a.jvm.len()),
            "minecraft_arguments": v.minecraft_arguments,
        }));
    }

    #[test]
    fn parses_the_logging_block_into_typed_fields() {
        let v = parse_fixture("1.20.1");
        let client = v
            .logging
            .as_ref()
            .and_then(|l| l.client.as_ref())
            .expect("1.20.1 publishes a client logging block");
        assert_eq!(client.argument, "-Dlog4j.configurationFile=${path}");
        assert_eq!(client.file.id, "client-1.12.xml");
        assert_eq!(client.file.sha1, "bd65e7d2e3c237be76cfbef4c2405033d7f91521");
        assert_eq!(client.file.size, 888);
        assert!(client.file.url.ends_with("client-1.12.xml"));
        assert_eq!(client.kind.as_deref(), Some("log4j2-xml"));
    }

    #[test]
    fn a_version_without_logging_parses() {
        let v: VersionJson = serde_json::from_str(r#"{"id":"x"}"#).expect("no logging parses");
        assert_eq!(v.logging, None);
    }

    #[test]
    fn an_empty_logging_object_parses_with_no_client() {
        let v: VersionJson =
            serde_json::from_str(r#"{"id":"x","logging":{}}"#).expect("empty logging parses");
        assert_eq!(v.logging, Some(LoggingConfig::default()));
    }

    #[test]
    fn parses_legacy_version_json() {
        let v = parse_fixture("1.8.9");
        assert!(v.arguments.is_none());
        assert!(v.minecraft_arguments.is_some());
        insta::assert_json_snapshot!(serde_json::json!({
            "id": v.id,
            "main_class": v.main_class,
            "libraries": v.libraries.len(),
            "java_version": v.java_version,
            "assets": v.assets,
        }));
    }

    #[test]
    fn legacy_libraries_carry_natives_and_extract() {
        let v = parse_fixture("1.8.9");
        let lwjgl = v
            .libraries
            .iter()
            .find(|l| l.name.starts_with("org.lwjgl.lwjgl:lwjgl-platform"))
            .expect("lwjgl-platform present");
        let natives = lwjgl.natives.as_ref().expect("natives map present");
        assert_eq!(
            natives.get("linux").map(String::as_str),
            Some("natives-linux")
        );
        let classifiers = lwjgl
            .downloads
            .as_ref()
            .and_then(|d| d.classifiers.as_ref())
            .expect("classifiers present");
        assert!(classifiers.contains_key("natives-linux"));
    }

    #[test]
    fn round_trips_through_serialize() {
        let v = parse_fixture("1.20.1");
        let text = serde_json::to_string(&v).expect("serializes");
        let again: VersionJson = serde_json::from_str(&text).expect("reparses");
        assert_eq!(v, again);
    }

    #[test]
    fn merge_key_separates_the_plain_jar_from_its_classifiers() {
        let plain = MavenCoord::parse("org.lwjgl:lwjgl-glfw:3.3.1").expect("parses");
        let natives =
            MavenCoord::parse("org.lwjgl:lwjgl-glfw:3.3.1:natives-linux").expect("parses");
        let other =
            MavenCoord::parse("org.lwjgl:lwjgl-glfw:3.3.1:natives-windows").expect("parses");
        assert_eq!(plain.key(), natives.key());
        assert_eq!(plain.merge_key(), "org.lwjgl:lwjgl-glfw:");
        assert_eq!(natives.merge_key(), "org.lwjgl:lwjgl-glfw:natives-linux");
        assert_ne!(plain.merge_key(), natives.merge_key());
        assert_ne!(natives.merge_key(), other.merge_key());
    }

    #[test]
    fn maven_coord_with_classifier_builds_a_path() {
        let c = MavenCoord::parse("org.lwjgl:lwjgl:3.3.1:natives-linux").expect("parses");
        assert_eq!(c.key(), "org.lwjgl:lwjgl");
        assert_eq!(c.classifier.as_deref(), Some("natives-linux"));
        assert_eq!(
            c.path(),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar"
        );
    }

    #[test]
    fn maven_coord_honours_an_ext_suffix() {
        let c = MavenCoord::parse("de.oceanlabs.mcp:mcp_config:1.20.1@zip").expect("parses");
        assert_eq!(c.ext, "zip");
        assert_eq!(
            c.path(),
            "de/oceanlabs/mcp/mcp_config/1.20.1/mcp_config-1.20.1.zip"
        );
    }

    #[test]
    fn maven_coord_rejects_a_short_name() {
        assert!(matches!(
            MavenCoord::parse("org.lwjgl:lwjgl"),
            Err(Error::BadMavenCoord(_))
        ));
    }
}
