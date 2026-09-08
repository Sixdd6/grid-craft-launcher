//! Unit tests for the converters and formatters. No Slint instance and no display needed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gcl_core::auth::{Account, AccountKind};
use gcl_core::instances::Instance;
use gcl_core::instances::model::{ContentEntry, ContentKind, GcPreset, InstanceConfig, Loader};
use gcl_core::loaders::LoaderVersion;
use gcl_core::mojang::manifest::{ManifestEntry, VersionType};
use gcl_core::sources::{SearchHit, SourceId};

use super::{
    account_row, content_row, decode_description_image, decode_icon, format_bytes,
    format_downloads, gc_rows, instance_row, loader_version_row, search_row, setting_rows,
    short_time, version_row,
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
fn gc_rows_carries_the_token_label_and_description_of_each_preset_given() {
    let rows = gc_rows(&[GcPreset::Default, GcPreset::G1]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].token, "default");
    assert_eq!(rows[0].label, GcPreset::Default.label());
    assert_eq!(rows[0].description, GcPreset::Default.description());
    assert_eq!(rows[1].token, "g1");
    assert_eq!(rows[1].label, "G1");
    assert_eq!(rows[1].description, GcPreset::G1.description());
}

#[test]
fn gc_rows_of_an_empty_preset_list_is_empty() {
    assert!(gc_rows(&[]).is_empty());
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
fn decode_description_image_accepts_a_normal_png() {
    let image = image::RgbaImage::from_pixel(8, 6, image::Rgba([1, 2, 3, 255]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap_or_else(|err| panic!("encode: {err}"));
    let (width, height, pixels) =
        decode_description_image(&bytes).unwrap_or_else(|err| panic!("decode: {err}"));
    assert_eq!((width, height), (8, 6));
    assert_eq!(pixels.len(), (8 * 6 * 4) as usize);
}

#[test]
fn decode_description_image_refuses_one_wider_than_the_4096px_cap() {
    // A real 4097px-wide PNG would be a heavy fixture; `image::Limits` rejects an oversized
    // image by reading its header before decoding pixels, so a correctly-sized but
    // claims-to-be-huge PNG (built by hand, header only) exercises the same refusal path
    // without a multi-megabyte fixture. `image`'s decoder answers a `LimitError` before it
    // tries to allocate the claimed dimensions.
    let mut png = Vec::new();
    {
        // 5000x5000 8-bit RGBA PNG: signature, IHDR with the oversized dimensions, and an
        // otherwise-empty IDAT/IEND so the decoder gets far enough to see the size and stop.
        png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&5000u32.to_be_bytes());
        ihdr.extend_from_slice(&5000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        write_png_chunk(&mut png, b"IHDR", &ihdr);
        write_png_chunk(&mut png, b"IEND", &[]);
    }
    let err = decode_description_image(&png).expect_err("an oversized image must be refused");
    assert!(
        err.to_lowercase().contains("limit") || err.to_lowercase().contains("dimension"),
        "expected a limit-related error, got: {err}"
    );
}

/// Appends one PNG chunk (length, type, data, a placeholder CRC): the decoder only needs to
/// parse `IHDR` far enough to check `image::Limits` before the CRC would matter here.
fn write_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32(kind, data).to_be_bytes());
}

/// A minimal CRC-32 (the same polynomial PNG uses), so the hand-built chunk above passes the
/// decoder's checksum check on its way to `image::Limits` rejecting the declared size.
fn crc32(kind: &[u8], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in kind.iter().chain(data.iter()) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn decode_description_image_downscales_a_wide_banner_to_the_1600px_cap() {
    // 3000x100: well under the 4096 decode cap, but past the 1600px downscale cap on its
    // long side. A real 3000px-wide PNG compresses to almost nothing when every pixel is the
    // same color, so this is cheap to build and encode inline.
    let image = image::RgbaImage::from_pixel(3000, 100, image::Rgba([200, 50, 10, 255]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap_or_else(|err| panic!("encode: {err}"));

    let (width, height, pixels) =
        decode_description_image(&bytes).unwrap_or_else(|err| panic!("decode: {err}"));
    assert_eq!(width, 1600, "the long side is shrunk to the cap");
    assert_eq!(
        height, 53,
        "the short side keeps the same 30:1 aspect ratio, rounded"
    );
    assert_eq!(pixels.len(), (1600 * 53 * 4) as usize);
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
fn search_row_collapses_newlines_in_the_description() {
    let hit = SearchHit {
        source: SourceId::Modrinth,
        project_id: "AANobbMI".into(),
        slug: "sodium".into(),
        title: "Sodium".into(),
        description: "A rendering engine\nfor Fabric\r\nand Quilt.  Fast.".into(),
        author: "jellysquid3".into(),
        kind: ContentKind::Mod,
        is_pack: false,
        downloads: 12_345,
        icon_url: None,
        page_url: "https://modrinth.com/mod/sodium".into(),
    };
    let row = search_row(&hit);
    assert!(!row.description.contains('\n'));
    assert!(!row.description.contains('\r'));
    assert_eq!(
        row.description.as_str(),
        "A rendering engine for Fabric and Quilt. Fast."
    );
}

#[test]
fn search_row_collapses_every_kind_of_whitespace_and_trims_the_ends() {
    let hit = SearchHit {
        source: SourceId::Modrinth,
        project_id: "AANobbMI".into(),
        slug: "sodium".into(),
        title: "Sodium".into(),
        description: "  \tA rendering\tengine\u{a0}\u{a0}for  Fabric.  \n".into(),
        author: "jellysquid3".into(),
        kind: ContentKind::Mod,
        is_pack: false,
        downloads: 12_345,
        icon_url: None,
        page_url: "https://modrinth.com/mod/sodium".into(),
    };
    let row = search_row(&hit);
    assert_eq!(row.description.as_str(), "A rendering engine for Fabric.");
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
