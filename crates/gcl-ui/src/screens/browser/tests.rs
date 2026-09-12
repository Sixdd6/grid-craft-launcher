use gcl_core::instances::model::{ContentKind, Loader};
use gcl_core::mojang::manifest::{ManifestEntry, VersionType};
use gcl_core::sources::SourceId;

use gcl_core::content::DependencyConflict;

use super::{
    Target, add_request, clamp_index, conflict_note, ensure_minecraft_option, kinds_for,
    loader_option_at, loader_option_index, manual_note, minecraft_index_for, minecraft_options,
    minecraft_string_at, needs_world, pack_name, page_bounds, parse_loader, result_status,
    source_status, target_index_for,
};

/// A manifest entry with only the fields `minecraft_options` reads.
fn entry(id: &str, kind: VersionType) -> ManifestEntry {
    ManifestEntry {
        id: id.to_string(),
        kind,
        url: String::new(),
        time: String::new(),
        release_time: String::new(),
        sha1: String::new(),
        compliance_level: 0,
    }
}

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
        None,
    );
    assert_eq!(request.source, SourceId::Modrinth);
    assert_eq!(request.project, "AANobbMI");
    assert_eq!(request.version, None, "the newest version wins");
    assert_eq!(request.kind, Some(ContentKind::DataPack));
    assert_eq!(request.world.as_deref(), Some("New World"));
}

#[test]
fn add_request_without_a_kind_lets_the_project_decide() {
    let request = add_request(SourceId::CurseForge, "238222", None, None, None);
    assert_eq!(request.kind, None);
    assert_eq!(request.world, None);
}

#[test]
fn add_request_pins_update_s_version() {
    let request = add_request(
        SourceId::Modrinth,
        "AANobbMI",
        Some(ContentKind::Mod),
        None,
        Some("v-latest".to_string()),
    );
    assert_eq!(request.version.as_deref(), Some("v-latest"));
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
        None,
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

#[test]
fn minecraft_options_lists_any_first_then_releases_newest_first() {
    let entries = vec![
        entry("24w10a", VersionType::Snapshot),
        entry("1.20.2", VersionType::Release),
        entry("1.20.1", VersionType::Release),
        entry("b1.7.3", VersionType::OldBeta),
    ];
    assert_eq!(
        minecraft_options(&entries, None),
        vec!["any", "1.20.2", "1.20.1"],
        "snapshots and old channels are left out, releases keep the manifest's own order"
    );
}

#[test]
fn minecraft_options_inserts_a_snapshot_target_after_any() {
    let entries = vec![
        entry("24w10a", VersionType::Snapshot),
        entry("1.20.1", VersionType::Release),
    ];
    assert_eq!(
        minecraft_options(&entries, Some("24w10a")),
        vec!["any", "24w10a", "1.20.1"],
        "a target the release list does not carry is inserted right after `any`"
    );
}

#[test]
fn minecraft_options_does_not_duplicate_a_target_already_listed() {
    let entries = vec![entry("1.20.1", VersionType::Release)];
    assert_eq!(
        minecraft_options(&entries, Some("1.20.1")),
        vec!["any", "1.20.1"],
        "a release target is already on the list, so nothing is inserted"
    );
}

#[test]
fn minecraft_options_ignores_an_empty_target() {
    let entries = vec![entry("1.20.1", VersionType::Release)];
    assert_eq!(minecraft_options(&entries, Some("")), vec!["any", "1.20.1"]);
    assert_eq!(minecraft_options(&entries, None), vec!["any", "1.20.1"]);
}

#[test]
fn minecraft_options_with_no_manifest_yet_still_carries_the_target() {
    assert_eq!(
        minecraft_options(&[], Some("1.20.1")),
        vec!["any", "1.20.1"],
        "the placeholder list `open()` shows before the manifest job answers"
    );
    assert_eq!(minecraft_options(&[], None), vec!["any"]);
}

#[test]
fn minecraft_index_for_maps_any_and_a_known_version() {
    let options = vec![
        "any".to_string(),
        "1.20.1".to_string(),
        "1.19.4".to_string(),
    ];
    assert_eq!(minecraft_index_for(&options, ""), 0, "empty is any");
    assert_eq!(minecraft_index_for(&options, "1.20.1"), 1);
    assert_eq!(minecraft_index_for(&options, "1.19.4"), 2);
}

#[test]
fn minecraft_index_for_falls_back_to_any_when_the_version_is_not_listed() {
    let options = vec!["any".to_string(), "1.20.1".to_string()];
    assert_eq!(
        minecraft_index_for(&options, "1.18.2"),
        0,
        "a version the list has not loaded yet reads as any, not out of range"
    );
}

#[test]
fn minecraft_string_at_maps_any_to_empty_and_everything_else_to_itself() {
    let options = vec!["any".to_string(), "1.20.1".to_string()];
    assert_eq!(minecraft_string_at(&options, 0), "");
    assert_eq!(minecraft_string_at(&options, 1), "1.20.1");
    assert_eq!(
        minecraft_string_at(&options, 99),
        "",
        "an index outside the list reads as any"
    );
}

/// A target with only the fields `target_index_for` reads.
fn target(slug: &str, name: &str) -> Target {
    Target {
        slug: slug.to_string(),
        name: name.to_string(),
        minecraft: String::new(),
        loader: "any".to_string(),
    }
}

#[test]
fn target_index_for_finds_the_active_instance_by_slug() {
    let targets = vec![target("alpha", "Alpha"), target("beta", "Beta")];
    assert_eq!(
        target_index_for(0, "beta", &targets),
        1,
        "the active instance wins even when the rail left a different row picked"
    );
}

#[test]
fn target_index_for_keeps_the_previous_row_when_the_slug_is_empty() {
    let targets = vec![target("alpha", "Alpha"), target("beta", "Beta")];
    assert_eq!(
        target_index_for(1, "", &targets),
        1,
        "opened from the rail with no current instance: keep whatever was picked before"
    );
    assert_eq!(
        target_index_for(-1, "", &targets),
        0,
        "no previous pick and a non-empty list: the first row, not -1"
    );
}

#[test]
fn target_index_for_keeps_the_previous_row_when_the_slug_is_not_a_target() {
    let targets = vec![target("alpha", "Alpha"), target("beta", "Beta")];
    assert_eq!(
        target_index_for(1, "vanished", &targets),
        1,
        "an active instance that is no longer in the list keeps the previous pick"
    );
}

#[test]
fn target_index_for_gives_no_index_for_an_empty_list() {
    assert_eq!(target_index_for(0, "alpha", &[]), -1);
    assert_eq!(target_index_for(-1, "", &[]), -1);
}

#[test]
fn ensure_minecraft_option_finds_a_version_already_on_the_list() {
    let options = vec!["any".to_string(), "1.20.1".to_string()];
    assert_eq!(
        ensure_minecraft_option(&options, "1.20.1"),
        (options.clone(), 1),
        "already listed: the row is reused, not duplicated"
    );
}

#[test]
fn ensure_minecraft_option_inserts_a_missing_version_after_any() {
    let options = vec!["any".to_string(), "1.20.1".to_string()];
    assert_eq!(
        ensure_minecraft_option(&options, "1.19.4"),
        (
            vec![
                "any".to_string(),
                "1.19.4".to_string(),
                "1.20.1".to_string()
            ],
            1
        ),
        "a target the manifest job never inserted still gets a row of its own, right after any"
    );
}

#[test]
fn ensure_minecraft_option_reads_a_blank_version_as_any_and_leaves_the_list_alone() {
    let options = vec!["any".to_string(), "1.20.1".to_string()];
    assert_eq!(
        ensure_minecraft_option(&options, ""),
        (options, 0),
        "no version is exactly what row 0 already means"
    );
}

#[test]
fn ensure_minecraft_option_inserts_at_the_front_of_an_empty_list() {
    assert_eq!(
        ensure_minecraft_option(&[], "1.20.1"),
        (vec!["1.20.1".to_string()], 0),
        "the placeholder list before `open()` has even run has no `any` row to insert after"
    );
}
