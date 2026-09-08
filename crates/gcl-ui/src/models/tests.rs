//! Unit tests for the converters and formatters. No Slint instance and no display needed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gcl_core::auth::{Account, AccountKind};
use gcl_core::instances::Instance;
use gcl_core::instances::model::{ContentEntry, ContentKind, InstanceConfig, Loader};
use gcl_core::loaders::LoaderVersion;
use gcl_core::mojang::manifest::{ManifestEntry, VersionType};
use gcl_core::sources::{SearchHit, SourceId};

use super::{
    account_row, content_row, decode_icon, format_bytes, format_downloads, instance_row,
    loader_version_row, search_row, setting_rows, short_time, version_row,
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
fn content_row_shows_title_over_file_stem() {
    let entry = ContentEntry {
        source: "modrinth".into(),
        project_id: "AANobbMI".into(),
        version_id: "abc123".into(),
        file_name: "sodium-fabric-0.5.8.jar".into(),
        title: Some("Sodium".into()),
        kind: ContentKind::Mod,
        enabled: true,
        ..ContentEntry::default()
    };
    let row = content_row(&entry, false);
    assert_eq!(row.name.as_str(), "Sodium");
    assert_eq!(
        row.file_name.as_str(),
        "sodium-fabric-0.5.8.jar",
        "the file name stays on the row for the row's second line"
    );
}

#[test]
fn content_row_falls_back_to_the_file_stem_with_no_title() {
    let entry = ContentEntry {
        file_name: "sodium-fabric-0.5.8.jar".into(),
        title: None,
        ..ContentEntry::default()
    };
    assert_eq!(
        content_row(&entry, false).name.as_str(),
        "sodium-fabric-0.5.8"
    );
}

#[test]
fn search_row_carries_the_icon_url_with_no_decoded_icon_yet() {
    let hit = SearchHit {
        source: SourceId::Modrinth,
        project_id: "AANobbMI".into(),
        slug: "sodium".into(),
        title: "Sodium".into(),
        description: "A rendering engine".into(),
        author: "jellysquid3".into(),
        kind: ContentKind::Mod,
        is_pack: false,
        downloads: 12_345,
        icon_url: Some("https://cdn.modrinth.com/icon.png".into()),
        page_url: "https://modrinth.com/mod/sodium".into(),
    };
    let row = search_row(&hit);
    assert_eq!(row.icon_url.as_str(), "https://cdn.modrinth.com/icon.png");
    assert_eq!(row.icon.size().width, 0, "nothing has been fetched yet");
}

#[test]
fn decode_icon_reads_png_webp_gif_and_jpeg_bytes() {
    let image = image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 255]));
    let dynamic = image::DynamicImage::ImageRgba8(image);

    for format in [
        image::ImageFormat::Png,
        image::ImageFormat::WebP,
        image::ImageFormat::Gif,
        image::ImageFormat::Jpeg,
    ] {
        let mut bytes = Vec::new();
        dynamic
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap_or_else(|err| panic!("encode {format:?}: {err}"));
        let (width, height, pixels) =
            decode_icon(&bytes).unwrap_or_else(|err| panic!("decode {format:?}: {err}"));
        assert_eq!((width, height), (4, 3), "{format:?}");
        assert_eq!(pixels.len(), (4 * 3 * 4) as usize, "{format:?}");
    }
}

#[test]
fn decode_icon_rejects_garbage() {
    assert!(decode_icon(b"not an image").is_err());
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
        is_pack: false,
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

#[test]
fn version_row_lowercases_the_channel_and_shortens_the_date() {
    let entry = ManifestEntry {
        id: "1.21".into(),
        kind: VersionType::Snapshot,
        url: "https://example.invalid/1.21.json".into(),
        time: "2026-06-13T13:12:00+00:00".into(),
        release_time: "2026-06-13T13:12:00+00:00".into(),
        sha1: "0".repeat(40),
        compliance_level: 1,
    };
    let row = version_row(&entry);
    assert_eq!(row.id.as_str(), "1.21");
    assert_eq!(row.kind.as_str(), "snapshot");
    assert_eq!(row.release_time.as_str(), "2026-06-13 13:12");
}

#[test]
fn version_row_names_the_old_channels_in_one_word() {
    let entry = ManifestEntry {
        id: "b1.7.3".into(),
        kind: VersionType::OldBeta,
        url: String::new(),
        time: String::new(),
        release_time: "not a date".into(),
        sha1: String::new(),
        compliance_level: 0,
    };
    let row = version_row(&entry);
    assert_eq!(row.kind.as_str(), "oldbeta");
    assert_eq!(row.release_time.as_str(), "not a date");
}

#[test]
fn loader_version_row_carries_both_flags() {
    let version = LoaderVersion {
        version: "0.16.9".into(),
        stable: true,
        recommended: true,
    };
    let row = loader_version_row(&version);
    assert_eq!(row.version.as_str(), "0.16.9");
    assert!(row.stable);
    assert!(row.recommended);
}
