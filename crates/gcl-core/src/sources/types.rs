//! Data types shared by every content [`super::Source`]: ids, search queries and
//! results, projects, versions, and the `page_url` helpers that build a browser link
//! back to the source's own site.

use serde::{Deserialize, Serialize};

use crate::instances::model::Loader;

pub use crate::instances::model::ContentKind;

/// Which content source a piece of data came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceId {
    /// Modrinth (<https://modrinth.com>).
    Modrinth,
    /// CurseForge (<https://www.curseforge.com>).
    CurseForge,
}

impl SourceId {
    /// Parses the lowercase form used in config and CLI flags: `modrinth` or `curseforge`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "modrinth" => Some(SourceId::Modrinth),
            "curseforge" => Some(SourceId::CurseForge),
            _ => None,
        }
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SourceId::Modrinth => "modrinth",
            SourceId::CurseForge => "curseforge",
        };
        f.write_str(s)
    }
}

/// Search parameters common to every source.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Free-text search string.
    pub text: String,
    /// Restrict results to one content kind, when set.
    pub kind: Option<ContentKind>,
    /// Restrict results to one Minecraft version, when set.
    pub minecraft: Option<String>,
    /// Restrict results to one loader, when set.
    pub loader: Option<Loader>,
    /// Offset into the result set, for pagination.
    pub offset: u32,
    /// Maximum number of hits to return. Defaults to 20.
    pub limit: u32,
}

impl Default for SearchQuery {
    fn default() -> Self {
        SearchQuery {
            text: String::new(),
            kind: None,
            minecraft: None,
            loader: None,
            offset: 0,
            limit: 20,
        }
    }
}

/// One search result row.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    /// Source this hit came from.
    pub source: SourceId,
    /// Project id at the source.
    pub project_id: String,
    /// URL-safe slug at the source.
    pub slug: String,
    /// Display title.
    pub title: String,
    /// Short description.
    pub description: String,
    /// Author or team name.
    pub author: String,
    /// Content kind.
    pub kind: ContentKind,
    /// Whether this hit is a modpack, which only [`super::Source::search_packs`] returns.
    ///
    /// A modpack is not a [`ContentKind`], so a pack hit reports [`ContentKind::Mod`] and
    /// sets this flag; its `page_url` comes from [`pack_page_url`]. Every hit from
    /// [`super::Source::search`] has it `false`.
    pub is_pack: bool,
    /// Total download count.
    pub downloads: u64,
    /// Icon image URL, when the source has one.
    pub icon_url: Option<String>,
    /// Browser link to the project's page at the source.
    pub page_url: String,
    /// Newest file per Minecraft version and loader, when the source ships one with the hit.
    ///
    /// CurseForge sends `latestFilesIndexes` with every search hit, so a caller can name the
    /// newest file for a target without a second request. Modrinth has no comparable field,
    /// so a Modrinth hit always leaves this empty and the caller lists versions instead.
    pub latest_files: Vec<LatestFileIndex>,
}

/// One entry of a search hit's newest-file index: a file id for one Minecraft version and loader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LatestFileIndex {
    /// Minecraft version this file targets.
    pub game_version: String,
    /// Source loader id, `None` when the file names no loader.
    ///
    /// CurseForge numbers them: Forge 1, Fabric 4, Quilt 5, NeoForge 6.
    pub loader: Option<u32>,
    /// File id at the source, as a version id.
    pub file_id: String,
}

/// One page of search results.
#[derive(Debug, Clone, Serialize)]
pub struct SearchPage {
    /// Hits in this page, in the source's own order.
    pub hits: Vec<SearchHit>,
    /// Total hits available across all pages.
    pub total: u64,
    /// Offset this page started at.
    pub offset: u32,
}

/// A project (mod, modpack, resource pack, etc.) at a source.
#[derive(Debug, Clone, Serialize)]
pub struct Project {
    /// Source this project came from.
    pub source: SourceId,
    /// Project id at the source.
    pub id: String,
    /// URL-safe slug at the source.
    pub slug: String,
    /// Display title.
    pub title: String,
    /// Full description.
    pub description: String,
    /// Content kind.
    pub kind: ContentKind,
    /// Browser link to the project's page at the source.
    pub page_url: String,
}

/// Release channel of a version, ordered `Alpha < Beta < Release`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum ReleaseKind {
    /// Alpha (Modrinth) or alpha (CurseForge).
    Alpha,
    /// Beta.
    Beta,
    /// Release.
    Release,
}

/// One downloadable file attached to a [`Version`].
#[derive(Debug, Clone, Serialize)]
pub struct VersionFile {
    /// Direct download URL. `None` when the author opted out of third-party
    /// distribution; see [`super::Error::ManualDownload`].
    pub url: Option<String>,
    /// File name as the author uploaded it.
    pub file_name: String,
    /// File size in bytes, when known.
    pub size: Option<u64>,
    /// SHA-1 hex digest, when the source provides one.
    pub sha1: Option<String>,
    /// SHA-512 hex digest, when the source provides one (Modrinth).
    pub sha512: Option<String>,
    /// CurseForge fingerprint (MurmurHash2), when the source provides one.
    pub fingerprint: Option<u32>,
    /// Whether this is the primary file for the version.
    pub primary: bool,
}

/// How a [`Version`] depends on another project or version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DependencyKind {
    /// Must be installed for this version to work.
    Required,
    /// May be installed for extra features.
    Optional,
    /// Must not be installed alongside this version.
    Incompatible,
    /// Bundled inside this version's file; nothing extra to install.
    Embedded,
}

/// One dependency edge from a [`Version`] to another project or version.
#[derive(Debug, Clone, Serialize)]
pub struct Dependency {
    /// Dependency's project id, when known.
    pub project_id: Option<String>,
    /// Dependency's specific version id, when known.
    pub version_id: Option<String>,
    /// Kind of dependency.
    pub kind: DependencyKind,
}

/// One version (release) of a project.
#[derive(Debug, Clone, Serialize)]
pub struct Version {
    /// Source this version came from.
    pub source: SourceId,
    /// Id of the project this version belongs to.
    pub project_id: String,
    /// Id of this version at the source.
    pub id: String,
    /// Display name of the version.
    pub name: String,
    /// Version number string, e.g. `1.2.3`.
    pub number: String,
    /// Release channel.
    pub kind: ReleaseKind,
    /// Minecraft versions this version supports.
    pub game_versions: Vec<String>,
    /// Loaders this version supports, lowercase (`fabric`, `quilt`, `forge`,
    /// `neoforge`, `minecraft`, `datapack`, `iris`, `optifine`, ...).
    pub loaders: Vec<String>,
    /// Publish time, RFC 3339.
    pub published: String,
    /// Files attached to this version.
    pub files: Vec<VersionFile>,
    /// Dependencies of this version.
    pub dependencies: Vec<Dependency>,
    /// Release notes for this version, in the source's own markup, when the source sent
    /// them with the version itself.
    ///
    /// Modrinth carries them in `changelog`, but only when the request asked for them:
    /// the version *list* sends `include_changelog=false`, so a listed version leaves this
    /// `None` and [`crate::sources::Source::changelog`] fetches them. CurseForge never
    /// sends them with a file, so they are always `None` there.
    pub changelog: Option<String>,
}

/// Filter applied when listing a project's versions.
#[derive(Debug, Clone, Default)]
pub struct VersionFilter {
    /// Restrict to one Minecraft version, when set.
    pub minecraft: Option<String>,
    /// Restrict to these loaders. Empty means no loader filter.
    pub loaders: Vec<String>,
}

/// Builds the browser page URL for a project of the given kind at the given source.
pub fn page_url(source: SourceId, kind: ContentKind, slug: &str) -> String {
    match source {
        SourceId::Modrinth => {
            let segment = match kind {
                ContentKind::Mod => "mod",
                ContentKind::ResourcePack => "resourcepack",
                ContentKind::Shader => "shader",
                ContentKind::DataPack => "datapack",
                ContentKind::World => "world",
            };
            format!("https://modrinth.com/{segment}/{slug}")
        }
        SourceId::CurseForge => {
            let segment = match kind {
                ContentKind::Mod => "mc-mods",
                ContentKind::ResourcePack => "texture-packs",
                ContentKind::Shader => "shaders",
                ContentKind::DataPack => "data-packs",
                ContentKind::World => "worlds",
            };
            format!("https://www.curseforge.com/minecraft/{segment}/{slug}")
        }
    }
}

/// Builds the browser page URL for a modpack at the given source. Modpacks are not
/// a [`ContentKind`] variant, so they get their own helper rather than a case in
/// [`page_url`].
pub fn pack_page_url(source: SourceId, slug: &str) -> String {
    match source {
        SourceId::Modrinth => format!("https://modrinth.com/modpack/{slug}"),
        SourceId::CurseForge => format!("https://www.curseforge.com/minecraft/modpacks/{slug}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_query_default_limit_is_twenty() {
        assert_eq!(SearchQuery::default().limit, 20);
    }

    #[test]
    fn source_id_parse_round_trips() {
        for id in [SourceId::Modrinth, SourceId::CurseForge] {
            assert_eq!(SourceId::parse(&id.to_string()), Some(id));
        }
        assert_eq!(SourceId::parse("bogus"), None);
    }

    #[test]
    fn release_kind_orders_release_above_beta_above_alpha() {
        assert!(ReleaseKind::Release > ReleaseKind::Beta);
        assert!(ReleaseKind::Beta > ReleaseKind::Alpha);
        assert!(ReleaseKind::Release > ReleaseKind::Alpha);
    }

    #[test]
    fn modrinth_page_url_for_every_kind() {
        assert_eq!(
            page_url(SourceId::Modrinth, ContentKind::Mod, "sodium"),
            "https://modrinth.com/mod/sodium"
        );
        assert_eq!(
            page_url(SourceId::Modrinth, ContentKind::ResourcePack, "faithful"),
            "https://modrinth.com/resourcepack/faithful"
        );
        assert_eq!(
            page_url(SourceId::Modrinth, ContentKind::Shader, "complementary"),
            "https://modrinth.com/shader/complementary"
        );
        assert_eq!(
            page_url(SourceId::Modrinth, ContentKind::DataPack, "vanilla-tweaks"),
            "https://modrinth.com/datapack/vanilla-tweaks"
        );
        assert_eq!(
            page_url(SourceId::Modrinth, ContentKind::World, "skyblock"),
            "https://modrinth.com/world/skyblock"
        );
        assert_eq!(
            pack_page_url(SourceId::Modrinth, "fabulously-optimized"),
            "https://modrinth.com/modpack/fabulously-optimized"
        );
    }

    #[test]
    fn curseforge_page_url_for_every_kind() {
        assert_eq!(
            page_url(SourceId::CurseForge, ContentKind::Mod, "jei"),
            "https://www.curseforge.com/minecraft/mc-mods/jei"
        );
        assert_eq!(
            page_url(SourceId::CurseForge, ContentKind::ResourcePack, "faithful"),
            "https://www.curseforge.com/minecraft/texture-packs/faithful"
        );
        assert_eq!(
            page_url(SourceId::CurseForge, ContentKind::Shader, "bsl-shaders"),
            "https://www.curseforge.com/minecraft/shaders/bsl-shaders"
        );
        assert_eq!(
            page_url(
                SourceId::CurseForge,
                ContentKind::DataPack,
                "vanilla-tweaks"
            ),
            "https://www.curseforge.com/minecraft/data-packs/vanilla-tweaks"
        );
        assert_eq!(
            page_url(SourceId::CurseForge, ContentKind::World, "skyblock"),
            "https://www.curseforge.com/minecraft/worlds/skyblock"
        );
        assert_eq!(
            pack_page_url(SourceId::CurseForge, "all-the-mods-9"),
            "https://www.curseforge.com/minecraft/modpacks/all-the-mods-9"
        );
    }
}
