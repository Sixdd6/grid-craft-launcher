//! Converters from `gcl-core` types to the Slint structs declared in `ui/types.slint`.
//!
//! Nothing here touches the UI thread: these are plain value conversions, so they are
//! unit-testable without a Slint instance.

use std::collections::BTreeMap;

use gcl_core::auth::{Account, AccountKind};
use gcl_core::content::ManualDownload;
use gcl_core::instances::Instance;
use gcl_core::instances::model::{ContentEntry, GcPreset};
use gcl_core::launcher::{InstallState, LatestVersion, VersionTarget};
use gcl_core::loaders::LoaderVersion;
use gcl_core::mojang::manifest::ManifestEntry;
use gcl_core::sources::SearchHit;

use crate::{
    AccountRow, ContentRow, GcPresetRow, InstanceRow, LoaderVersionRow, PendingRow, SearchRow,
    SettingRow, VersionRow,
};

pub mod settings;

/// Builds the instances-list row for one instance.
pub fn instance_row(i: &Instance, installed: bool, running: bool) -> InstanceRow {
    InstanceRow {
        slug: i.slug.as_str().into(),
        name: i.config.name.as_str().into(),
        minecraft: i.config.minecraft.as_str().into(),
        loader: i.config.loader.to_string().into(),
        loader_version: i.config.loader_version.clone().unwrap_or_default().into(),
        last_launched: i
            .config
            .last_launched
            .as_deref()
            .map(short_time)
            .unwrap_or_default()
            .into(),
        installed,
        running,
    }
}

/// Builds the content row for one installed file. `update` marks a newer version at the source.
///
/// `name` is the project's title when the entry carries one, falling back to the file's stem
/// for an entry installed before `title` existed. `content_rows` (in `screens/instance.rs`)
/// sorts the built rows by this same name, case-insensitively.
pub fn content_row(e: &ContentEntry, update: bool) -> ContentRow {
    ContentRow {
        project_id: e.project_id.as_str().into(),
        source: e.source.as_str().into(),
        name: e
            .title
            .as_deref()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| file_stem(&e.file_name))
            .into(),
        version: e.version_id.as_str().into(),
        kind: e.kind.to_string().into(),
        enabled: e.enabled,
        update_available: update,
        file_name: e.file_name.as_str().into(),
    }
}

/// Builds the browser row for one search hit. `icon` starts empty: `src/screens/browser.rs`
/// fills it in once the row's icon has been fetched and decoded, off the UI thread.
///
/// The description collapses `\n`/`\r` to a single space and squeezes repeated spaces down to
/// one, so a short description with embedded line breaks renders as one line the row's own
/// `word-wrap` reflows, rather than as the literal blank lines a Slint `Text` would otherwise
/// show.
pub fn search_row(h: &SearchHit) -> SearchRow {
    SearchRow {
        source: h.source.to_string().into(),
        project_id: h.project_id.as_str().into(),
        slug: h.slug.as_str().into(),
        title: h.title.as_str().into(),
        description: collapse_whitespace(&h.description).into(),
        author: h.author.as_str().into(),
        kind: h.kind.to_string().into(),
        downloads: format_downloads(h.downloads).into(),
        page_url: h.page_url.as_str().into(),
        icon_url: h.icon_url.as_deref().unwrap_or_default().into(),
        icon: Default::default(),
        latest_number: "".into(),
        latest_id: "".into(),
        installed_number: "".into(),
        state: "unknown".into(),
    }
}

/// The four fields `fetch_latest` (`src/screens/browser.rs`) writes onto one row once its
/// latest version and, when there is a target instance, its install state have answered.
///
/// `latest.version` being `None` (nothing at the source fits the target) is the only way
/// `state` comes back `"none"`; `install_state` being `None` (no target instance to compare
/// against) or [`InstallState::Unknown`] (its own lookup failed) both read as
/// `"not_installed"` and `"unknown"` respectively — the first because there is nothing
/// installed to compare against, the second because [`InstallState::Unknown`]'s own doc
/// comment is exactly "leave the line blank until a state arrives", the same as a row that
/// has not been checked at all. `target` is not read here: the display text a row shows
/// (built in `browser.slint`, alongside `BrowserState.minecraft`/`loader`) is the only thing
/// that needs it, and this converter only ever answers with a version this call already
/// resolved for that target.
pub fn latest_row_fields(
    latest: &LatestVersion,
    install_state: Option<&InstallState>,
    _target: &VersionTarget,
) -> (String, String, String, String) {
    let Some(version) = &latest.version else {
        return (
            String::new(),
            String::new(),
            String::new(),
            "none".to_string(),
        );
    };
    let latest_number = version.number.clone();
    let latest_id = version.id.clone();
    match install_state {
        None | Some(InstallState::NotInstalled) => (
            latest_number,
            latest_id,
            String::new(),
            "not_installed".to_string(),
        ),
        Some(InstallState::Unknown) => (
            latest_number,
            latest_id,
            String::new(),
            "unknown".to_string(),
        ),
        Some(InstallState::Installed { number, .. }) => (
            latest_number,
            latest_id,
            number.clone(),
            "installed".to_string(),
        ),
        Some(InstallState::Older { installed_number }) => (
            latest_number,
            latest_id,
            installed_number.clone(),
            "older".to_string(),
        ),
    }
}

/// Replaces every run of whitespace — `\n`, `\r`, `\t`, and any other `char::is_whitespace`,
/// not only a space — with a single space, and trims the result, so a description with
/// embedded line breaks or tabs becomes a single line a `Text`'s own `word-wrap` reflows
/// instead of showing as literal blank lines or runs of visible gaps.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Decodes a fetched icon's bytes into raw RGBA8 pixels and its dimensions.
///
/// Pure and windowless, so it is unit-tested directly: the `slint::Image` it feeds is built
/// only in the event-loop closure that calls this, per the `slint-ui` skill's rule that a
/// `slint::Image` is UI-thread only. `image::load_from_memory` sniffs the format from the
/// bytes, so a `cache/icons/*.bin` file with no real extension still decodes.
pub fn decode_icon(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory(bytes).map_err(|err| err.to_string())?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((width, height, rgba.into_raw()))
}

/// The long side a decoded description image is shrunk to, in pixels. A changelog banner or a
/// screenshot renders in a column a few hundred pixels wide; anything bigger only costs memory
/// and upload-sized pixel buffers `slint::Image::from_rgba8` has to copy onto the GPU.
const MAX_DESCRIPTION_IMAGE_SIDE: u32 = 1600;

/// Decodes a fetched description image's bytes into raw RGBA8 pixels and its dimensions, under
/// a 4096x4096 pixel decode cap, then downscales it so its long side is at most
/// [`MAX_DESCRIPTION_IMAGE_SIDE`].
///
/// Unlike [`decode_icon`], a description image comes from any `https://` host (no CDN
/// allowlist, per `download::images::ImageCache`), so the cap here guards against a
/// pathologically large image using this decode step to exhaust memory, the same reason the
/// core-side [`gcl_core`] design doc gives for capping the fetch itself at 5 MiB. `image`'s
/// [`image::Limits`] rejects an oversized image before it is fully decoded rather than after.
/// The downscale runs after that check, on the already-decoded image: `DynamicImage::thumbnail`
/// only ever shrinks, so an image already under the cap is returned as is.
pub fn decode_description_image(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);

    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|err| err.to_string())?;
    reader.limits(limits);
    let img = reader.decode().map_err(|err| err.to_string())?;
    // `DynamicImage::thumbnail` always resizes to fit the given bounds, scaling up as
    // readily as down, so an image already under the cap is left alone rather than
    // needlessly resampled or, worse, enlarged.
    let img =
        if img.width() > MAX_DESCRIPTION_IMAGE_SIDE || img.height() > MAX_DESCRIPTION_IMAGE_SIDE {
            img.thumbnail(MAX_DESCRIPTION_IMAGE_SIDE, MAX_DESCRIPTION_IMAGE_SIDE)
        } else {
            img
        };
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((width, height, rgba.into_raw()))
}

/// Builds the accounts row for one saved account. `active` marks the launch default.
pub fn account_row(a: &Account, active: bool) -> AccountRow {
    AccountRow {
        id: a.id.as_str().into(),
        name: a.name.as_str().into(),
        kind: match a.kind {
            AccountKind::Offline => "offline",
            AccountKind::Msa => "msa",
        }
        .into(),
        expires: a
            .mc_token_expires
            .as_deref()
            .map(short_time)
            .unwrap_or_default()
            .into(),
        active,
    }
}

/// Builds the settings rows for a key/value map, in key order.
pub fn setting_rows(map: &BTreeMap<String, String>) -> Vec<SettingRow> {
    map.iter()
        .map(|(key, value)| SettingRow {
            key: key.as_str().into(),
            value: value.as_str().into(),
        })
        .collect()
}

/// Builds the loader-version row for one loader build.
pub fn loader_version_row(v: &LoaderVersion) -> LoaderVersionRow {
    LoaderVersionRow {
        version: v.version.as_str().into(),
        stable: v.stable,
        recommended: v.recommended,
    }
}

/// Builds the Minecraft-version row for one manifest entry.
pub fn version_row(e: &ManifestEntry) -> VersionRow {
    VersionRow {
        id: e.id.as_str().into(),
        kind: format!("{:?}", e.kind).to_lowercase().into(),
        release_time: short_time(&e.release_time).into(),
    }
}

/// Builds the row for one download the user has to fetch by hand.
///
/// The kind comes off the pending entry itself: it is the kind the add resolved, and the
/// same one the import will install as.
pub fn pending_row(m: &ManualDownload) -> PendingRow {
    PendingRow {
        project_id: m.project_id.as_str().into(),
        file_name: m.file_name.as_str().into(),
        page_url: m.page_url.as_str().into(),
        kind: m.kind.to_string().into(),
    }
}

/// Formats a byte count with binary units, e.g. `1536` as `1.5 KiB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = UNITS[0];
    for next in &UNITS[1..] {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

/// Formats a download count short, e.g. `12345` as `12.3K`.
pub fn format_downloads(count: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1_000_000_000, "B"), (1_000_000, "M"), (1_000, "K")];
    for (scale, suffix) in UNITS {
        if count >= scale {
            return format!("{:.1}{suffix}", count as f64 / scale as f64);
        }
    }
    count.to_string()
}

/// Shortens an RFC 3339 timestamp to `YYYY-MM-DD HH:MM`. Unparsable input is returned as is.
pub fn short_time(rfc3339: &str) -> String {
    let format = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]");
    match time::OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339) {
        Ok(t) => t.format(&format).unwrap_or_else(|_| rfc3339.to_string()),
        Err(_) => rfc3339.to_string(),
    }
}

/// Builds the JVM tab's GC combo rows, one per supported preset, in the order given.
///
/// `label` and `description` come straight from `GcPreset::label()`/`GcPreset::description()`
/// in gcl-core, never hand-written here, so the UI's copy cannot drift from the CLI's.
pub fn gc_rows(presets: &[GcPreset]) -> Vec<GcPresetRow> {
    presets
        .iter()
        .map(|preset| GcPresetRow {
            token: preset.to_string().into(),
            label: preset.label().into(),
            description: preset.description().into(),
        })
        .collect()
}

/// The file name without its last extension.
fn file_stem(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    }
}

#[cfg(test)]
mod tests;
