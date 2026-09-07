use gcl_core::instances::model::{ContentKind, Loader};
use gcl_core::sources::SourceId;

use super::{
    add_request, clamp_index, kinds_for, manual_note, page_bounds, parse_loader, result_status,
    source_status,
};

#[test]
fn kinds_for_modrinth_leaves_out_world() {
    let kinds = kinds_for(SourceId::Modrinth);
    assert_eq!(
        kinds,
        vec!["mod", "resourcepack", "shader", "datapack", "modpack"]
    );
    assert!(!kinds.contains(&"world"), "Modrinth has no world type");
}

#[test]
fn kinds_for_curseforge_offers_world() {
    let kinds = kinds_for(SourceId::CurseForge);
    assert_eq!(
        kinds,
        vec![
            "mod",
            "resourcepack",
            "shader",
            "datapack",
            "world",
            "modpack"
        ]
    );
}

#[test]
fn every_kind_but_modpack_parses_as_content() {
    for source in [SourceId::Modrinth, SourceId::CurseForge] {
        for kind in kinds_for(source) {
            let parsed = ContentKind::parse(kind);
            if kind == "modpack" {
                assert!(parsed.is_none(), "a modpack is not content");
            } else {
                assert!(parsed.is_some(), "{kind} should parse");
            }
        }
    }
}

#[test]
fn page_bounds_uses_the_total_when_the_source_reports_one() {
    // 20 of 57 shown on page 0: more behind it.
    assert!(page_bounds(0, 20, 20, 57));
    // 17 of 57 shown on page 2: that is the end.
    assert!(!page_bounds(2, 20, 17, 57));
    // A total that lands exactly on the page boundary has nothing after it.
    assert!(!page_bounds(1, 20, 20, 40));
}

#[test]
fn page_bounds_falls_back_to_a_full_page_without_a_total() {
    assert!(page_bounds(0, 20, 20, 0), "a full page probably has more");
    assert!(!page_bounds(0, 20, 6, 0), "a short page is the last one");
}

#[test]
fn page_bounds_survives_a_negative_page_and_a_zero_size() {
    assert!(!page_bounds(-3, 20, 0, 0));
    assert!(page_bounds(0, 0, 1, 0), "a zero page size counts as one");
}

#[test]
fn add_request_pins_no_version_and_carries_the_world() {
    let request = add_request(
        SourceId::Modrinth,
        "AANobbMI",
        Some(ContentKind::DataPack),
        Some("New World".to_string()),
    );
    assert_eq!(request.source, SourceId::Modrinth);
    assert_eq!(request.project, "AANobbMI");
    assert_eq!(request.version, None, "the newest version wins");
    assert_eq!(request.kind, Some(ContentKind::DataPack));
    assert_eq!(request.world.as_deref(), Some("New World"));
}

#[test]
fn add_request_without_a_kind_lets_the_project_decide() {
    let request = add_request(SourceId::CurseForge, "238222", None, None);
    assert_eq!(request.kind, None);
    assert_eq!(request.world, None);
}

#[test]
fn parse_loader_reads_the_four_loaders_and_nothing_else() {
    assert_eq!(parse_loader("fabric"), Some(Loader::Fabric));
    assert_eq!(parse_loader(" quilt "), Some(Loader::Quilt));
    assert_eq!(parse_loader("forge"), Some(Loader::Forge));
    assert_eq!(parse_loader("neoforge"), Some(Loader::NeoForge));
    assert_eq!(parse_loader("any"), None);
    assert_eq!(parse_loader(""), None);
    assert_eq!(parse_loader("none"), None);
}

#[test]
fn manual_note_names_the_tab_that_finishes_the_job() {
    let note = manual_note(2);
    assert!(note.starts_with("2 file(s)"), "got {note}");
    assert!(note.contains("Content tab"), "got {note}");
}

#[test]
fn result_status_reports_what_came_back() {
    assert_eq!(result_status(0, 0), "No results");
    assert_eq!(result_status(20, 0), "20 result(s)");
    assert_eq!(result_status(20, 57), "20 of 57 result(s)");
}

#[test]
fn source_status_names_the_missing_key() {
    assert_eq!(source_status(&[]), "No content source is enabled");
    let modrinth_only = source_status(&[SourceId::Modrinth]);
    assert!(
        modrinth_only.contains("CURSEFORGE_API_KEY"),
        "got {modrinth_only}"
    );
    assert_eq!(
        source_status(&[SourceId::Modrinth, SourceId::CurseForge]),
        "Ready to search"
    );
}

#[test]
fn clamp_index_holds_an_index_inside_the_list() {
    assert_eq!(clamp_index(3, 2), 1);
    assert_eq!(clamp_index(-1, 2), 0);
    assert_eq!(clamp_index(0, 0), -1, "an empty list has no index");
}
