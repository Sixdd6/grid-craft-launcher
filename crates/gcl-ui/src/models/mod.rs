//! Converters from `gcl-core` types to the Slint structs declared in `ui/types.slint`.
//!
//! Nothing here touches the UI thread: these are plain value conversions, so they are
//! unit-testable without a Slint instance.

use std::collections::BTreeMap;

use gcl_core::auth::{Account, AccountKind};
use gcl_core::content::ManualDownload;
use gcl_core::instances::Instance;
use gcl_core::instances::model::ContentEntry;
use gcl_core::loaders::LoaderVersion;
use gcl_core::mojang::manifest::ManifestEntry;
use gcl_core::sources::SearchHit;

use crate::{
    AccountRow, ContentRow, InstanceRow, LoaderVersionRow, PendingRow, SearchRow, SettingRow,
    VersionRow,
};

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
pub fn content_row(e: &ContentEntry, update: bool) -> ContentRow {
    ContentRow {
        project_id: e.project_id.as_str().into(),
        source: e.source.as_str().into(),
        name: file_stem(&e.file_name).into(),
        version: e.version_id.as_str().into(),
        kind: e.kind.to_string().into(),
        enabled: e.enabled,
        update_available: update,
        file_name: e.file_name.as_str().into(),
    }
}

/// Builds the browser row for one search hit.
pub fn search_row(h: &SearchHit) -> SearchRow {
    SearchRow {
        source: h.source.to_string().into(),
        project_id: h.project_id.as_str().into(),
        slug: h.slug.as_str().into(),
        title: h.title.as_str().into(),
        description: h.description.as_str().into(),
        author: h.author.as_str().into(),
        kind: h.kind.to_string().into(),
        downloads: format_downloads(h.downloads).into(),
        page_url: h.page_url.as_str().into(),
    }
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

/// The file name without its last extension.
fn file_stem(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    }
}

#[cfg(test)]
mod tests;
