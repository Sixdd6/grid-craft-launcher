//! Unit tests for the converters and formatters. No Slint instance and no display needed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gcl_core::auth::{Account, AccountKind};
use gcl_core::instances::Instance;
use gcl_core::instances::model::{ContentEntry, ContentKind, InstanceConfig, Loader};
use gcl_core::sources::{SearchHit, SourceId};

use super::{
    account_row, content_row, format_bytes, format_downloads, instance_row, search_row,
    setting_rows, short_time,
};

#[test]
fn format_bytes_uses_binary_units() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(1023), "1023 B");
    assert_eq!(format_bytes(1536), "1.5 KiB");
    assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
    assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
}

#[test]
fn format_downloads_shortens_large_counts() {
    assert_eq!(format_downloads(999), "999");
    assert_eq!(format_downloads(12_345), "12.3K");
    assert_eq!(format_downloads(4_500_000), "4.5M");
    assert_eq!(format_downloads(2_000_000_000), "2.0B");
}

#[test]
fn short_time_trims_an_rfc3339_stamp() {
    assert_eq!(short_time("2026-09-06T12:34:56Z"), "2026-09-06 12:34");
    assert_eq!(short_time("2026-01-02T03:04:05+00:00"), "2026-01-02 03:04");
}

#[test]
fn short_time_returns_unparsable_input_unchanged() {
    assert_eq!(short_time("never"), "never");
    assert_eq!(short_time(""), "");
}

#[test]
fn instance_row_carries_the_config_and_the_flags() {
    let instance = Instance {
        slug: "vanilla".into(),
        dir: PathBuf::from("/root/instances/vanilla"),
        config: InstanceConfig {
            name: "Vanilla".into(),
            minecraft: "1.20.1".into(),
            loader: Loader::Fabric,
            loader_version: Some("0.16.9".into()),
            last_launched: Some("2026-09-06T12:34:56Z".into()),
            ..InstanceConfig::default()
        },
    };
    let row = instance_row(&instance, true, false);
    assert_eq!(row.slug.as_str(), "vanilla");
    assert_eq!(row.name.as_str(), "Vanilla");
    assert_eq!(row.minecraft.as_str(), "1.20.1");
    assert_eq!(row.loader.as_str(), "fabric");
    assert_eq!(row.loader_version.as_str(), "0.16.9");
    assert_eq!(row.last_launched.as_str(), "2026-09-06 12:34");
    assert!(row.installed);
    assert!(!row.running);
}

#[test]
fn content_row_names_the_file_without_its_extension() {
    let entry = ContentEntry {
        source: "modrinth".into(),
        project_id: "AANobbMI".into(),
        version_id: "abc123".into(),
        file_name: "sodium-fabric-0.5.8.jar".into(),
        kind: ContentKind::Mod,
        enabled: true,
        ..ContentEntry::default()
    };
    let row = content_row(&entry, true);
    assert_eq!(row.name.as_str(), "sodium-fabric-0.5.8");
    assert_eq!(row.file_name.as_str(), "sodium-fabric-0.5.8.jar");
    assert_eq!(row.kind.as_str(), "mod");
    assert_eq!(row.version.as_str(), "abc123");
    assert!(row.enabled);
    assert!(row.update_available);
}

#[test]
fn search_row_formats_the_download_count() {
    let hit = SearchHit {
        source: SourceId::Modrinth,
        project_id: "AANobbMI".into(),
        slug: "sodium".into(),
        title: "Sodium".into(),
        description: "A rendering engine".into(),
        author: "jellysquid3".into(),
        kind: ContentKind::Mod,
        downloads: 12_345,
        icon_url: None,
        page_url: "https://modrinth.com/mod/sodium".into(),
    };
    let row = search_row(&hit);
    assert_eq!(row.source.as_str(), "modrinth");
    assert_eq!(row.downloads.as_str(), "12.3K");
    assert_eq!(row.kind.as_str(), "mod");
}

#[test]
fn account_row_shortens_the_expiry_and_marks_the_active_one() {
    let account = Account {
        id: "0123".into(),
        name: "Steve".into(),
        kind: AccountKind::Msa,
        mc_token: Some("secret".into()),
        mc_token_expires: Some("2026-09-06T12:34:56Z".into()),
        xuid: None,
        refresh_store: None,
    };
    let row = account_row(&account, true);
    assert_eq!(row.kind.as_str(), "msa");
    assert_eq!(row.expires.as_str(), "2026-09-06 12:34");
    assert!(row.active);
}

#[test]
fn setting_rows_follow_key_order() {
    let mut map = BTreeMap::new();
    map.insert("fov".to_string(), "90".to_string());
    map.insert("difficulty".to_string(), "hard".to_string());
    let rows = setting_rows(&map);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].key.as_str(), "difficulty");
    assert_eq!(rows[1].key.as_str(), "fov");
}
