//! Unit tests for the screen's pure helpers. No Slint instance and no display needed.

use gcl_core::instances::model::Loader;

use super::{can_create, filter_rows, loader_from_slug, loader_version_label};
use crate::InstanceRow;

/// Builds a row with only the two fields the filter reads.
fn row(slug: &str, name: &str) -> InstanceRow {
    InstanceRow {
        slug: slug.into(),
        name: name.into(),
        ..InstanceRow::default()
    }
}

fn sample() -> Vec<InstanceRow> {
    vec![
        row("vanilla", "Vanilla 1.21"),
        row("fabric-perf", "Fabric Performance"),
        row("create-above", "Create: Above and Beyond"),
    ]
}

#[test]
fn filter_rows_keeps_everything_for_an_empty_filter() {
    assert_eq!(filter_rows(&sample(), "").len(), 3);
    assert_eq!(filter_rows(&sample(), "   ").len(), 3);
}

#[test]
fn filter_rows_matches_the_name_ignoring_case() {
    let kept = filter_rows(&sample(), "PERFORM");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].slug.as_str(), "fabric-perf");
}

#[test]
fn filter_rows_matches_the_slug() {
    let kept = filter_rows(&sample(), "create-");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].name.as_str(), "Create: Above and Beyond");
}

#[test]
fn filter_rows_returns_nothing_when_no_row_matches() {
    assert!(filter_rows(&sample(), "quilt").is_empty());
}

#[test]
fn can_create_needs_a_name_and_a_version() {
    assert!(can_create("My pack", true, 0, false));
    assert!(!can_create("", true, 0, false));
    assert!(!can_create("   ", true, 0, false));
    assert!(!can_create("My pack", false, 0, false));
}

#[test]
fn can_create_needs_a_build_once_a_loader_is_chosen() {
    assert!(!can_create("My pack", true, 1, false));
    assert!(can_create("My pack", true, 1, true));
    assert!(can_create("My pack", true, 4, true));
}

#[test]
fn loader_from_slug_reads_every_display_name() {
    assert_eq!(loader_from_slug("none"), Loader::None);
    assert_eq!(loader_from_slug("fabric"), Loader::Fabric);
    assert_eq!(loader_from_slug("quilt"), Loader::Quilt);
    assert_eq!(loader_from_slug("forge"), Loader::Forge);
    assert_eq!(loader_from_slug("neoforge"), Loader::NeoForge);
    assert_eq!(loader_from_slug("nonsense"), Loader::None);
}

#[test]
fn loader_from_slug_round_trips_the_display_impl() {
    for loader in [
        Loader::None,
        Loader::Fabric,
        Loader::Quilt,
        Loader::Forge,
        Loader::NeoForge,
    ] {
        assert_eq!(loader_from_slug(&loader.to_string()), loader);
    }
}

#[test]
fn loader_version_label_marks_the_recommended_and_the_unstable() {
    assert_eq!(
        loader_version_label("0.16.9", true, true),
        "0.16.9 (recommended)"
    );
    assert_eq!(loader_version_label("0.16.9", true, false), "0.16.9");
    assert_eq!(
        loader_version_label("0.17.0", false, false),
        "0.17.0 (unstable)"
    );
}
