//! Unit tests for the screen's pure helpers. No Slint instance and no display needed.

use gcl_core::content::{ManualDownload, UpdateCandidate};
use gcl_core::instances::model::{ContentEntry, ContentKind};
use gcl_core::sources::{ReleaseKind, SourceId, Version};

use super::{content_rows, instance_jvm, jvm_valid, pending_rows};

/// An installed entry with only the fields these helpers read.
fn entry(project_id: &str, file_name: &str) -> ContentEntry {
    ContentEntry {
        source: "modrinth".to_string(),
        project_id: project_id.to_string(),
        version_id: "v1".to_string(),
        file_name: file_name.to_string(),
        ..ContentEntry::default()
    }
}

/// An update candidate for one entry. The new version is filler: nothing here reads it.
fn candidate(entry: ContentEntry) -> UpdateCandidate {
    let new = Version {
        source: SourceId::Modrinth,
        project_id: entry.project_id.clone(),
        id: "v2".to_string(),
        name: "v2".to_string(),
        number: "2.0.0".to_string(),
        kind: ReleaseKind::Release,
        game_versions: vec!["1.20.1".to_string()],
        loaders: vec!["fabric".to_string()],
        published: "2026-01-01T00:00:00Z".to_string(),
        files: Vec::new(),
        dependencies: Vec::new(),
    };
    UpdateCandidate { entry, new }
}

/// A pending manual download, with the kind the add that produced it resolved.
fn pending(file_name: &str, kind: ContentKind) -> ManualDownload {
    ManualDownload {
        source: SourceId::CurseForge,
        kind,
        project_id: "p1".to_string(),
        version_id: "v1".to_string(),
        file_name: file_name.to_string(),
        page_url: "https://example.invalid/file".to_string(),
        fingerprint: None,
        sha1: None,
        world: None,
    }
}

#[test]
fn jvm_valid_accepts_a_range_inside_the_offered_bounds() {
    assert!(jvm_valid(512, 512));
    assert!(jvm_valid(2048, 4096));
    assert!(jvm_valid(512, 65536));
}

#[test]
fn jvm_valid_rejects_an_inverted_or_out_of_range_pair() {
    assert!(!jvm_valid(4096, 2048), "the minimum is above the maximum");
    assert!(!jvm_valid(256, 4096), "below the smallest heap offered");
    assert!(!jvm_valid(1024, 131072), "above the largest heap offered");
    assert!(!jvm_valid(-1, 2048), "a negative heap is not a heap");
}

#[test]
fn instance_jvm_splits_the_arguments_and_drops_an_empty_java_path() {
    let jvm = instance_jvm(1024, 4096, "  -XX:+UseG1GC  -Dfoo=bar ", "   ");
    assert_eq!(jvm.min_mib, Some(1024));
    assert_eq!(jvm.max_mib, Some(4096));
    assert_eq!(
        jvm.extra_args,
        vec!["-XX:+UseG1GC".to_string(), "-Dfoo=bar".to_string()]
    );
    assert_eq!(jvm.java_path, None, "a blank path lets the launcher choose");

    let jvm = instance_jvm(1024, 4096, "", " /usr/bin/java ");
    assert_eq!(
        jvm.java_path,
        Some(std::path::PathBuf::from("/usr/bin/java")),
        "a typed path is trimmed, not dropped"
    );
    assert!(jvm.extra_args.is_empty());
}

#[test]
fn content_rows_marks_only_the_entries_a_candidate_names() {
    let entries = vec![
        entry("sodium", "sodium-0.5.8.jar"),
        entry("lithium", "l.jar"),
    ];
    let rows = content_rows(&entries, &[candidate(entries[0].clone())]);
    assert_eq!(rows.len(), 2);
    assert!(rows[0].update_available, "sodium has a newer version");
    assert!(!rows[1].update_available, "lithium does not");
    assert_eq!(
        rows[0].name.as_str(),
        "sodium-0.5.8",
        "the extension is cut"
    );
}

#[test]
fn content_rows_marks_nothing_without_candidates() {
    let entries = vec![entry("sodium", "sodium-0.5.8.jar")];
    let rows = content_rows(&entries, &[]);
    assert!(!rows[0].update_available);
}

#[test]
fn pending_rows_carry_the_kind_the_entry_records() {
    let rows = pending_rows(&[
        pending("mod.jar", ContentKind::Mod),
        pending("pack.zip", ContentKind::ResourcePack),
        pending("data.zip", ContentKind::DataPack),
    ]);
    let kinds: Vec<&str> = rows.iter().map(|row| row.kind.as_str()).collect();
    assert_eq!(kinds, vec!["mod", "resourcepack", "datapack"]);
}
