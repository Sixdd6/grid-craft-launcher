use gcl_core::instances::model::{ContentKind, Loader};
use gcl_core::sources::richtext::Block as CoreBlock;
use gcl_core::sources::{ReleaseKind, SourceId, Version};
use slint::Model;

use super::{block_row, version_row};

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
        changelog: None,
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
        block_row(&CoreBlock::Bullet {
            depth: 1,
            text: "b".to_string()
        })
        .kind,
        "bullet"
    );
    assert_eq!(block_row(&CoreBlock::Code("c".to_string())).kind, "code");
}

#[test]
fn block_row_carries_a_bullets_depth() {
    let row = block_row(&CoreBlock::Bullet {
        depth: 2,
        text: "nested".to_string(),
    });
    assert_eq!(row.kind, "bullet");
    assert_eq!(row.text, "nested");
    assert_eq!(row.depth, 2);
}

#[test]
fn block_row_maps_a_rule_with_no_text() {
    let row = block_row(&CoreBlock::Rule);
    assert_eq!(row.kind, "rule");
    assert_eq!(row.text, "");
}

#[test]
fn block_row_maps_a_quote() {
    let row = block_row(&CoreBlock::Quote("be careful".to_string()));
    assert_eq!(row.kind, "quote");
    assert_eq!(row.text, "be careful");
}

#[test]
fn block_row_maps_an_image_to_its_url_and_alt_text_with_no_fetch_state_yet() {
    let row = block_row(&CoreBlock::Image {
        url: "https://example.invalid/screenshot.png".to_string(),
        alt: "A screenshot".to_string(),
    });
    assert_eq!(row.kind, "image");
    assert_eq!(row.text, "A screenshot");
    assert_eq!(row.url, "https://example.invalid/screenshot.png");
    assert_eq!(row.image_state, "");
    assert_eq!(row.image.size().width, 0);
}

#[test]
fn block_row_flattens_a_table_row_major_with_the_header_as_row_zero() {
    let row = block_row(&CoreBlock::Table {
        header: vec!["Minecraft".to_string(), "Loader".to_string()],
        rows: vec![
            vec!["1.21.1".to_string(), "Fabric".to_string()],
            vec!["1.20.1".to_string(), "Quilt".to_string()],
        ],
    });
    assert_eq!(row.kind, "table");
    assert_eq!(row.columns, 2);
    let cells: Vec<String> = row.cells.iter().map(|c| c.to_string()).collect();
    assert_eq!(
        cells,
        vec!["Minecraft", "Loader", "1.21.1", "Fabric", "1.20.1", "Quilt"]
    );
}

#[test]
fn version_row_joins_game_versions_and_loaders() {
    let row = version_row(&version("abc"), None, None, ContentKind::Mod);
    assert_eq!(row.game_versions, "1.21.1, 1.21");
    assert_eq!(row.loaders, "fabric, quilt");
    assert_eq!(row.kind, "release");
    assert!(!row.installed);
}

#[test]
fn version_row_marks_the_installed_version() {
    let row = version_row(&version("abc"), Some("abc"), None, ContentKind::Mod);
    assert!(row.installed);
}

#[test]
fn version_row_does_not_mark_a_different_installed_version() {
    let row = version_row(&version("abc"), Some("def"), None, ContentKind::Mod);
    assert!(!row.installed);
}

#[test]
fn version_row_with_no_target_is_always_compatible() {
    let row = version_row(&version("abc"), None, None, ContentKind::Mod);
    assert!(row.compatible);
    assert_eq!(row.access_label, "Install 0.6.13");
}

#[test]
fn version_row_marks_installed_access_label() {
    let row = version_row(
        &version("abc"),
        Some("abc"),
        Some(("1.21.1", Loader::Fabric)),
        ContentKind::Mod,
    );
    assert!(row.compatible);
    assert_eq!(row.access_label, "Installed 0.6.13");
}

#[test]
fn version_row_is_incompatible_with_a_different_minecraft_version() {
    let row = version_row(
        &version("abc"),
        None,
        Some(("1.20.1", Loader::Fabric)),
        ContentKind::Mod,
    );
    assert!(!row.compatible);
    assert_eq!(row.access_label, "Not for 1.20.1 fabric");
}

#[test]
fn version_row_is_incompatible_with_a_loader_the_mod_does_not_list() {
    // The version only lists fabric and quilt loaders; Forge cannot run it even though
    // the Minecraft version matches.
    let row = version_row(
        &version("abc"),
        None,
        Some(("1.21.1", Loader::Forge)),
        ContentKind::Mod,
    );
    assert!(!row.compatible);
    assert_eq!(row.access_label, "Not for 1.21.1 forge");
}

#[test]
fn version_row_ignores_loader_for_a_non_mod_kind() {
    // Loader::Forge is not among the version's own "loaders" list, but a resource pack
    // does not care: only the Minecraft version has to match.
    let row = version_row(
        &version("abc"),
        None,
        Some(("1.21.1", Loader::Forge)),
        ContentKind::ResourcePack,
    );
    assert!(row.compatible);
}
