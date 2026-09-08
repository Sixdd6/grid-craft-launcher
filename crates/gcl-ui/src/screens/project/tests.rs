use gcl_core::instances::model::Loader;
use gcl_core::sources::richtext::Block as CoreBlock;
use gcl_core::sources::{ReleaseKind, SourceId, Version};

use super::{block_row, loader_filter, version_row};

fn version(id: &str) -> Version {
    Version {
        source: SourceId::Modrinth,
        project_id: "AANobbMI".to_string(),
        id: id.to_string(),
        name: "Sodium 0.6.13".to_string(),
        number: "0.6.13".to_string(),
        kind: ReleaseKind::Release,
        game_versions: vec!["1.21.1".to_string(), "1.21".to_string()],
        loaders: vec!["fabric".to_string(), "quilt".to_string()],
        published: "2026-08-01T10:00:00Z".to_string(),
        files: Vec::new(),
        dependencies: Vec::new(),
    }
}

#[test]
fn block_row_maps_heading_level_to_a_kind_string() {
    let row = block_row(&CoreBlock::Heading(2, "Features".to_string()));
    assert_eq!(row.kind, "heading2");
    assert_eq!(row.text, "Features");
}

#[test]
fn block_row_maps_every_other_kind() {
    assert_eq!(
        block_row(&CoreBlock::Paragraph("p".to_string())).kind,
        "paragraph"
    );
    assert_eq!(
        block_row(&CoreBlock::Bullet("b".to_string())).kind,
        "bullet"
    );
    assert_eq!(block_row(&CoreBlock::Code("c".to_string())).kind, "code");
}

#[test]
fn version_row_joins_game_versions_and_loaders() {
    let row = version_row(&version("abc"), None);
    assert_eq!(row.game_versions, "1.21.1, 1.21");
    assert_eq!(row.loaders, "fabric, quilt");
    assert_eq!(row.kind, "release");
    assert!(!row.installed);
}

#[test]
fn version_row_marks_the_installed_version() {
    let row = version_row(&version("abc"), Some("abc"));
    assert!(row.installed);
}

#[test]
fn version_row_does_not_mark_a_different_installed_version() {
    let row = version_row(&version("abc"), Some("def"));
    assert!(!row.installed);
}

#[test]
fn loader_filter_is_empty_for_no_loader() {
    assert!(loader_filter(Loader::None).is_empty());
}

#[test]
fn loader_filter_names_one_loader() {
    assert_eq!(loader_filter(Loader::Fabric), vec!["fabric".to_string()]);
}
