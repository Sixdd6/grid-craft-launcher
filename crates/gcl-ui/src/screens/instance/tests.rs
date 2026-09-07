//! Unit tests for the screen's pure helpers. No Slint instance and no display needed.

use gcl_core::content::{ManualDownload, UpdateCandidate};
use gcl_core::instances::model::{ContentEntry, ContentKind};
use gcl_core::sources::{ReleaseKind, SourceId, Version};

use super::{content_rows, instance_jvm, jvm_valid, pending_kind, tail_new_lines};

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

/// A pending manual download with the two fields the kind is guessed from.
fn pending(file_name: &str, world: Option<&str>) -> ManualDownload {
    ManualDownload {
        source: SourceId::CurseForge,
        project_id: "p1".to_string(),
        version_id: "v1".to_string(),
        file_name: file_name.to_string(),
        page_url: "https://example.invalid/file".to_string(),
        fingerprint: None,
        sha1: None,
        world: world.map(str::to_string),
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
fn tail_new_lines_returns_only_whole_lines() {
    let (lines, read) = tail_new_lines(0, "one\ntwo\nthr");
    assert_eq!(lines, vec!["one".to_string(), "two".to_string()]);
    assert_eq!(read, 8, "the partial third line is left for the next call");

    let (lines, read) = tail_new_lines(read, "one\ntwo\nthree\n");
    assert_eq!(lines, vec!["three".to_string()]);
    assert_eq!(read, 14);

    let (lines, read) = tail_new_lines(read, "one\ntwo\nthree\n");
    assert!(lines.is_empty(), "nothing was appended");
    assert_eq!(read, 14);
}

#[test]
fn tail_new_lines_strips_carriage_returns_and_restarts_on_a_truncated_file() {
    let (lines, _) = tail_new_lines(0, "one\r\ntwo\r\n");
    assert_eq!(lines, vec!["one".to_string(), "two".to_string()]);

    // The file was replaced by a shorter one, so the offset no longer means anything.
    let (lines, read) = tail_new_lines(500, "fresh\n");
    assert_eq!(lines, vec!["fresh".to_string()]);
    assert_eq!(read, 6);
}

#[test]
fn tail_new_lines_reads_from_the_start_when_the_offset_splits_a_character() {
    // "é" is two bytes, so an offset of 1 is inside it.
    let (lines, read) = tail_new_lines(1, "é\n");
    assert_eq!(lines, vec!["é".to_string()]);
    assert_eq!(read, 3);
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
fn pending_kind_reads_the_kind_off_the_pending_entry() {
    assert_eq!(pending_kind(&pending("mod.jar", None)), ContentKind::Mod);
    assert_eq!(pending_kind(&pending("MOD.JAR", None)), ContentKind::Mod);
    assert_eq!(
        pending_kind(&pending("pack.zip", None)),
        ContentKind::ResourcePack
    );
    assert_eq!(
        pending_kind(&pending("pack.zip", Some("New World"))),
        ContentKind::DataPack,
        "a named world makes it a data pack"
    );
}
