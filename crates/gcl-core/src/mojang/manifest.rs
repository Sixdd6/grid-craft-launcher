//! Serde types for `version_manifest_v2.json`.

use serde::{Deserialize, Serialize};

/// The whole version manifest: the current release and snapshot, and every known version.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct VersionManifest {
    /// The ids Mojang currently points at.
    pub latest: Latest,
    /// Every published version, newest first.
    pub versions: Vec<ManifestEntry>,
}

impl VersionManifest {
    /// Finds the entry for a version id.
    pub fn entry(&self, id: &str) -> Option<&ManifestEntry> {
        self.versions.iter().find(|v| v.id == id)
    }
}

/// The current release and snapshot ids.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Latest {
    /// Id of the newest release.
    pub release: String,
    /// Id of the newest snapshot.
    pub snapshot: String,
}

/// One version's entry in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ManifestEntry {
    /// The version id.
    pub id: String,
    /// Release channel.
    #[serde(rename = "type")]
    pub kind: VersionType,
    /// URL of the version JSON.
    pub url: String,
    /// ISO 8601 timestamp of the last metadata change.
    pub time: String,
    /// ISO 8601 release timestamp.
    #[serde(rename = "releaseTime")]
    pub release_time: String,
    /// Lowercase hex sha1 of the version JSON at `url`.
    pub sha1: String,
    /// Mojang's compliance level. Absent on old entries.
    #[serde(rename = "complianceLevel", default)]
    pub compliance_level: u8,
}

/// Release channel of a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionType {
    /// A full release.
    Release,
    /// A snapshot or pre-release.
    Snapshot,
    /// A beta build from 2010-2011.
    OldBeta,
    /// An alpha build from 2010.
    OldAlpha,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_manifest_fixture() {
        let text = std::fs::read_to_string("../../tests/fixtures/mojang/version_manifest_v2.json")
            .expect("fixture readable");
        let m: VersionManifest = serde_json::from_str(&text).expect("manifest parses");
        assert_eq!(m.latest.release, "26.2");
        let entry = m.entry("1.20.1").expect("1.20.1 present");
        assert_eq!(entry.kind, VersionType::Release);
        assert_eq!(entry.sha1, "19f5ae58f9c31bd3b0923cb822e99e3162bd62ab");
        assert_eq!(entry.compliance_level, 1);
        assert!(m.entry("nope").is_none());
    }

    #[test]
    fn parses_every_version_type() {
        let json = r#"[{"id":"b1.7.3","type":"old_beta","url":"u","time":"t","releaseTime":"r","sha1":"s"}]"#;
        let entries: Vec<ManifestEntry> = serde_json::from_str(json).expect("parses");
        assert_eq!(entries[0].kind, VersionType::OldBeta);
        assert_eq!(entries[0].compliance_level, 0);
    }
}
