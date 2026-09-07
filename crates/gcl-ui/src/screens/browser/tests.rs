use gcl_core::instances::model::{ContentKind, Loader};
use gcl_core::sources::SourceId;

use gcl_core::content::DependencyConflict;

use super::{
    add_request, clamp_index, conflict_note, kinds_for, loader_option_at, loader_option_index,
    manual_note, needs_world, pack_name, page_bounds, parse_loader, result_status, source_status,
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
fn a_loader_row_and_its_slug_map_both_ways() {
    for (index, slug) in ["any", "fabric", "quilt", "forge", "neoforge"]
        .into_iter()
        .enumerate()
    {
        let index = index as i32;
        assert_eq!(loader_option_at(index), slug);
        assert_eq!(loader_option_index(slug), index);
    }
    // A row outside the list, and a slug that is not one, both mean "do not filter".
    assert_eq!(loader_option_at(-1), "any");
    assert_eq!(loader_option_at(99), "any");
    assert_eq!(loader_option_index("liteloader"), 0);
    assert_eq!(parse_loader(loader_option_at(0)), None);
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
fn conflict_note_names_one_conflict_and_counts_more() {
    let conflict = DependencyConflict {
        project_id: "sodium".to_string(),
        title: "Sodium".to_string(),
        installed_version_id: "sv-new".to_string(),
        wanted_version_id: "sv-old".to_string(),
        wanted_by: "Iris".to_string(),
    };
    assert_eq!(
        conflict_note(std::slice::from_ref(&conflict)),
        "Kept Sodium at sv-new; Iris wanted sv-old"
    );
    let note = conflict_note(&[conflict.clone(), conflict]);
    assert!(note.contains("2 mod(s)"), "got {note}");
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

#[test]
fn needs_world_reads_the_row_kind_not_the_selector() {
    // The Type selector is not an argument, which is the point: a data pack row found before
    // the selector moved still needs a world, and a mod row still does not.
    assert!(needs_world("datapack"));
    assert!(!needs_world("mod"));
    assert!(!needs_world("resourcepack"));
    assert!(!needs_world("shader"));
    assert!(!needs_world("world"));
    assert!(
        !needs_world("modpack"),
        "a modpack is not added to an instance"
    );
    assert!(!needs_world(""), "an unknown kind lets the project decide");
}

#[test]
fn a_datapack_row_keeps_its_kind_in_the_request() {
    // What the stale-row fix protects: the kind comes from the row, so the request still
    // installs as a data pack into the world the chooser named.
    let kind = ContentKind::parse("datapack");
    let request = add_request(
        SourceId::Modrinth,
        "abc",
        kind,
        Some("New World".to_string()),
    );
    assert_eq!(request.kind, Some(ContentKind::DataPack));
    assert_eq!(request.world.as_deref(), Some("New World"));
}

#[test]
fn pack_name_suggests_the_archives_own_name() {
    assert_eq!(
        pack_name(std::path::Path::new(
            "/home/steve/Fabulously Optimized.mrpack"
        )),
        "Fabulously Optimized"
    );
    assert_eq!(pack_name(std::path::Path::new("packs/atm9.zip")), "atm9");
    assert_eq!(
        pack_name(std::path::Path::new("/tmp/")),
        "tmp",
        "a directory still names itself"
    );
    assert_eq!(pack_name(std::path::Path::new("")), "");
}
