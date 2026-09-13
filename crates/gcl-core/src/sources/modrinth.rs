//! Modrinth [`Source`] client: search, project, versions, and sha1 lookup against
//! the Modrinth API v2.
//!
//! Endpoint and rate-limit detail lives in the `mod-sources` skill. This module only
//! builds the URLs, maps the JSON onto [`crate::sources::types`], and turns HTTP
//! failures into [`Error`].

use async_trait::async_trait;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;

use crate::http::HttpClient;
use crate::instances::model::Loader;

use super::{
    ContentKind, Dependency, DependencyKind, Error, GalleryImage, Project, ReleaseKind, SearchHit,
    SearchPage, SearchQuery, Source, SourceId, Version, VersionFile, VersionFilter, pack_page_url,
    page_url,
};

/// Production base URL for the Modrinth API.
pub const BASE: &str = "https://api.modrinth.com/v2";

/// This source's id, used in every error and mapped value.
const ID: SourceId = SourceId::Modrinth;

/// Content kinds Modrinth can search and resolve.
///
/// `world` is not a Modrinth project type: a live `project_type:world` search returns
/// zero hits (`tests/fixtures/modrinth/search_types.json`), so [`ContentKind::World`]
/// is left out and [`Source::search`] rejects it.
const KINDS: &[ContentKind] = &[
    ContentKind::Mod,
    ContentKind::ResourcePack,
    ContentKind::Shader,
    ContentKind::DataPack,
];

/// Bytes escaped in a query-string value. Everything but the unreserved set, so a JSON
/// facet or `game_versions` array survives intact.
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The Modrinth content source.
#[derive(Debug, Clone)]
pub struct Modrinth {
    http: HttpClient,
    base: String,
}

impl Modrinth {
    /// Builds a client against the live API at [`BASE`].
    pub fn new(http: HttpClient) -> Self {
        Modrinth::with_base_url(http, BASE.to_string())
    }

    /// Builds a client against `base`, for tests that point at a mock server. Any
    /// trailing slash is trimmed.
    pub fn with_base_url(http: HttpClient, base: String) -> Self {
        Modrinth {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// Lists a modpack project's versions, whose primary file is the `.mrpack`.
    ///
    /// Same request as [`Source::versions`], without a loader filter: a modpack version
    /// lists its loader in the pack index, not in the version's `loaders`.
    #[tracing::instrument(skip(self))]
    pub async fn pack_versions(
        &self,
        project_id: &str,
        minecraft: Option<&str>,
    ) -> Result<Vec<Version>, Error> {
        self.fetch_versions(project_id, minecraft, &[]).await
    }

    /// Fetches one project and its long description in one request.
    ///
    /// Modrinth's `GET /project/{id}` carries the `body` markdown, so [`Source::project`]
    /// and [`Source::description`] are the same request twice. A details screen calls this
    /// instead and pays for one.
    #[tracing::instrument(skip(self))]
    pub async fn project_with_body(&self, id_or_slug: &str) -> Result<(Project, String), Error> {
        let url = format!("{}/project/{}", self.base, encode(id_or_slug));
        let raw: RawProject = self
            .http
            .get_json(&url)
            .await
            .map_err(|e| map_err(e, "project", Some(id_or_slug)))?;
        // Modpacks are not a `ContentKind`; a pack is fetched through
        // [`Modrinth::pack_versions`] instead of this method.
        let kind = ContentKind::parse(&raw.project_type).ok_or(Error::BadResponse {
            source_id: ID,
            what: "project_type",
            detail: raw.project_type.clone(),
        })?;
        let project = Project {
            source: ID,
            page_url: page_url(ID, kind, &raw.slug),
            id: raw.id,
            slug: raw.slug,
            title: raw.title,
            description: raw.description,
            kind,
            gallery: map_gallery(raw.gallery),
        };
        Ok((project, raw.body))
    }

    /// Fetches one version by id, unmapped.
    ///
    /// Both [`Source::version`] and [`Source::changelog`] answer from this one `GET
    /// /version/{id}`, so neither duplicates the request or its error mapping.
    #[tracing::instrument(skip(self))]
    async fn version_raw(&self, version_id: &str) -> Result<RawVersion, Error> {
        let url = format!("{}/version/{}", self.base, encode(version_id));
        self.http
            .get_json(&url)
            .await
            .map_err(|e| map_err(e, "version", Some(version_id)))
    }

    /// GETs `{base}/project/{project_id}/version` with the given filters and maps the body.
    async fn fetch_versions(
        &self,
        project_id: &str,
        minecraft: Option<&str>,
        loaders: &[String],
    ) -> Result<Vec<Version>, Error> {
        let mut url = format!(
            "{}/project/{}/version?include_changelog=false",
            self.base,
            encode(project_id)
        );
        if let Some(mc) = minecraft {
            url.push_str("&game_versions=");
            url.push_str(&encode(&json_string_array(std::slice::from_ref(
                &mc.to_string(),
            ))));
        }
        if !loaders.is_empty() {
            url.push_str("&loaders=");
            url.push_str(&encode(&json_string_array(loaders)));
        }
        let raw: Vec<RawVersion> = self
            .http
            .get_json(&url)
            .await
            .map_err(|e| map_err(e, "versions", Some(project_id)))?;
        let mut out = raw
            .into_iter()
            .map(map_version)
            .collect::<Result<Vec<_>, Error>>()?;
        // Modrinth already answers newest first; this keeps that order explicit and is
        // stable, so versions published at the same instant keep the API's order.
        out.sort_by(|a, b| b.published.cmp(&a.published));
        Ok(out)
    }
}

#[async_trait]
impl Source for Modrinth {
    fn id(&self) -> SourceId {
        ID
    }

    fn supported_kinds(&self) -> &[ContentKind] {
        KINDS
    }

    fn as_modrinth(&self) -> Option<&Modrinth> {
        Some(self)
    }

    #[tracing::instrument(skip(self))]
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, Error> {
        let mut facets: Vec<Vec<String>> = Vec::new();
        if let Some(kind) = q.kind {
            let project_type = project_type_of(kind).ok_or(Error::UnsupportedKind {
                source_id: ID,
                kind,
            })?;
            facets.push(vec![format!("project_type:{project_type}")]);
        }
        if let Some(mc) = &q.minecraft {
            facets.push(vec![format!("versions:{mc}")]);
        }
        if let (Some(ContentKind::Mod), Some(loader)) = (q.kind, q.loader)
            && loader != Loader::None
        {
            facets.push(vec![format!("categories:{loader}")]);
        }

        let mut url = format!(
            "{}/search?query={}&index=relevance&limit={}&offset={}",
            self.base,
            encode(&q.text),
            q.limit,
            q.offset
        );
        if !facets.is_empty() {
            url.push_str("&facets=");
            url.push_str(&encode(&json_facets(&facets)));
        }

        let raw: RawSearch = self
            .http
            .get_json(&url)
            .await
            .map_err(|e| map_err(e, "search", None))?;
        let hits = raw
            .hits
            .into_iter()
            .filter_map(|hit| {
                // Ruling: a `project_type:datapack` search still reports `project_type:
                // "mod"` on every hit, so an explicit kind in the query wins. With no
                // kind, an unmappable type (a modpack) drops out of the page.
                let kind = match q.kind {
                    Some(kind) => kind,
                    None => ContentKind::parse(&hit.project_type)?,
                };
                Some(SearchHit {
                    source: ID,
                    page_url: page_url(ID, kind, &hit.slug),
                    project_id: hit.project_id,
                    slug: hit.slug,
                    title: hit.title,
                    description: hit.description,
                    author: hit.author,
                    kind,
                    is_pack: false,
                    downloads: hit.downloads,
                    updated: hit.date_modified,
                    icon_url: hit.icon_url,
                    // Modrinth ships no newest-file index with a search hit; the caller
                    // lists versions instead.
                    latest_files: Vec::new(),
                })
            })
            .collect();
        Ok(SearchPage {
            hits,
            total: raw.total_hits,
            offset: raw.offset,
        })
    }

    #[tracing::instrument(skip(self))]
    async fn search_packs(&self, q: &SearchQuery) -> Result<SearchPage, Error> {
        // `project_type:modpack` is the only kind facet a pack search needs. No loader
        // facet: a modpack states its loader in the pack index, not in its categories.
        let mut facets: Vec<Vec<String>> = vec![vec!["project_type:modpack".to_string()]];
        if let Some(mc) = &q.minecraft {
            facets.push(vec![format!("versions:{mc}")]);
        }
        let url = format!(
            "{}/search?query={}&index=relevance&limit={}&offset={}&facets={}",
            self.base,
            encode(&q.text),
            q.limit,
            q.offset,
            encode(&json_facets(&facets))
        );

        let raw: RawSearch = self
            .http
            .get_json(&url)
            .await
            .map_err(|e| map_err(e, "search", None))?;
        let hits = raw
            .hits
            .into_iter()
            .map(|hit| SearchHit {
                source: ID,
                page_url: pack_page_url(ID, &hit.slug),
                project_id: hit.project_id,
                slug: hit.slug,
                title: hit.title,
                description: hit.description,
                author: hit.author,
                // A modpack has no `ContentKind` of its own; `is_pack` is what a caller
                // reads. `Mod` is the placeholder, as it is in every pack hit.
                kind: ContentKind::Mod,
                is_pack: true,
                downloads: hit.downloads,
                updated: hit.date_modified,
                icon_url: hit.icon_url,
                // Modrinth ships no newest-file index with a search hit.
                latest_files: Vec::new(),
            })
            .collect();
        Ok(SearchPage {
            hits,
            total: raw.total_hits,
            offset: raw.offset,
        })
    }

    async fn project(&self, id_or_slug: &str) -> Result<Project, Error> {
        Ok(self.project_with_body(id_or_slug).await?.0)
    }

    async fn description(&self, project_id: &str) -> Result<String, Error> {
        // The long description is the project's own `body` field: Modrinth has no
        // separate description endpoint, so this is the same GET as `project`. A caller
        // that wants both asks [`Modrinth::project_with_body`] once.
        Ok(self.project_with_body(project_id).await?.1)
    }

    #[tracing::instrument(skip(self))]
    async fn versions(&self, project_id: &str, f: &VersionFilter) -> Result<Vec<Version>, Error> {
        self.fetch_versions(project_id, f.minecraft.as_deref(), &f.loaders)
            .await
    }

    #[tracing::instrument(skip(self))]
    async fn version(&self, version_id: &str) -> Result<Version, Error> {
        map_version(self.version_raw(version_id).await?)
    }

    /// The version list sends `include_changelog=false`, so the notes come from the
    /// single-version endpoint instead, which that flag does not filter.
    #[tracing::instrument(skip(self))]
    async fn changelog(&self, project_id: &str, version_id: &str) -> Result<String, Error> {
        let _ = project_id;
        Ok(self
            .version_raw(version_id)
            .await?
            .changelog
            .unwrap_or_default())
    }

    #[tracing::instrument(skip(self))]
    async fn resolve_by_hash(&self, sha1: &[String]) -> Result<Vec<Version>, Error> {
        if sha1.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/version_files", self.base);
        let body = serde_json::json!({ "hashes": sha1, "algorithm": "sha1" });
        let raw: std::collections::HashMap<String, RawVersion> = self
            .http
            .post_json(&url, &body)
            .await
            .map_err(|e| map_err(e, "version_files", None))?;
        // Answer in the caller's hash order; hashes the server did not know are dropped.
        let mut out = Vec::with_capacity(raw.len());
        for hash in sha1 {
            if let Some(version) = raw.get(hash) {
                out.push(map_version(version.clone())?);
            }
        }
        Ok(out)
    }

    #[tracing::instrument(skip(self))]
    async fn resolve_by_fingerprint(&self, _fps: &[u32]) -> Result<Vec<Version>, Error> {
        Ok(Vec::new())
    }
}

/// Percent-encodes one query-string value.
fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_VALUE).to_string()
}

/// Renders `["a","b"]`, the shape Modrinth wants for `game_versions` and `loaders`.
fn json_string_array(values: &[String]) -> String {
    serde_json::Value::from(values.to_vec()).to_string()
}

/// Renders `[["a"],["b"]]`, the shape Modrinth wants for `facets`.
fn json_facets(facets: &[Vec<String>]) -> String {
    let value: Vec<serde_json::Value> = facets
        .iter()
        .map(|group| serde_json::Value::from(group.clone()))
        .collect();
    serde_json::Value::from(value).to_string()
}

/// Modrinth's `project_type` string for a content kind, or `None` when Modrinth has
/// no such type.
fn project_type_of(kind: ContentKind) -> Option<&'static str> {
    match kind {
        ContentKind::Mod => Some("mod"),
        ContentKind::ResourcePack => Some("resourcepack"),
        ContentKind::Shader => Some("shader"),
        ContentKind::DataPack => Some("datapack"),
        ContentKind::World => None,
    }
}

/// Turns an HTTP failure into a source error: 404 on a known id is [`Error::NotFound`],
/// a body that does not parse is [`Error::BadResponse`], anything else bubbles up.
fn map_err(err: crate::http::Error, what: &'static str, id: Option<&str>) -> Error {
    match err {
        crate::http::Error::Status { status: 404, .. } => match id {
            Some(id) => Error::NotFound {
                source_id: ID,
                id: id.to_string(),
            },
            None => Error::Http(err),
        },
        crate::http::Error::Json { ref source, .. } => Error::BadResponse {
            source_id: ID,
            what,
            detail: source.to_string(),
        },
        other => Error::Http(other),
    }
}

/// Maps one API version onto [`Version`].
fn map_version(raw: RawVersion) -> Result<Version, Error> {
    let kind = match raw.version_type.as_str() {
        "release" => ReleaseKind::Release,
        "beta" => ReleaseKind::Beta,
        "alpha" => ReleaseKind::Alpha,
        other => {
            return Err(Error::BadResponse {
                source_id: ID,
                what: "version_type",
                detail: other.to_string(),
            });
        }
    };
    let files = raw
        .files
        .into_iter()
        .map(|f| VersionFile {
            url: Some(f.url),
            file_name: f.filename,
            size: Some(f.size),
            sha1: f.hashes.sha1,
            sha512: f.hashes.sha512,
            fingerprint: None,
            primary: f.primary,
        })
        .collect();
    let dependencies = raw
        .dependencies
        .into_iter()
        .map(|d| {
            let kind = match d.dependency_type.as_str() {
                "required" => DependencyKind::Required,
                "optional" => DependencyKind::Optional,
                "incompatible" => DependencyKind::Incompatible,
                "embedded" => DependencyKind::Embedded,
                other => {
                    return Err(Error::BadResponse {
                        source_id: ID,
                        what: "dependency_type",
                        detail: other.to_string(),
                    });
                }
            };
            Ok(Dependency {
                project_id: d.project_id,
                version_id: d.version_id,
                kind,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(Version {
        source: ID,
        project_id: raw.project_id,
        id: raw.id,
        name: raw.name,
        number: raw.version_number,
        kind,
        game_versions: raw.game_versions,
        loaders: raw.loaders,
        published: raw.date_published,
        changelog: raw.changelog,
        files,
        dependencies,
    })
}

/// `GET /search` body.
#[derive(Debug, Deserialize)]
struct RawSearch {
    hits: Vec<RawHit>,
    total_hits: u64,
    offset: u32,
}

/// One row of `GET /search`.
#[derive(Debug, Deserialize)]
struct RawHit {
    project_id: String,
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    project_type: String,
    #[serde(default)]
    downloads: u64,
    /// RFC 3339. Absent on a hit the API sent no date for.
    #[serde(default)]
    date_modified: String,
    #[serde(default)]
    icon_url: Option<String>,
}

/// `GET /project/{id}` body.
#[derive(Debug, Deserialize)]
struct RawProject {
    id: String,
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    /// The long description, in markdown. Absent on a project that has none.
    #[serde(default)]
    body: String,
    project_type: String,
    /// Images published with the project. Absent on a project with none.
    #[serde(default)]
    gallery: Vec<RawGalleryImage>,
}

/// One `gallery` entry of `GET /project/{id}`.
#[derive(Debug, Deserialize)]
struct RawGalleryImage {
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    featured: bool,
    /// Display position. Absent on an older entry, which then sorts first.
    #[serde(default)]
    ordering: i64,
}

/// Maps a project's `gallery` onto [`GalleryImage`], sorted by `ordering`.
///
/// Modrinth serves no separate thumbnail, so `thumbnail_url` is always `None`.
fn map_gallery(mut raw: Vec<RawGalleryImage>) -> Vec<GalleryImage> {
    raw.sort_by_key(|g| g.ordering);
    raw.into_iter()
        .map(|g| GalleryImage {
            url: g.url,
            thumbnail_url: None,
            title: g.title.filter(|t| !t.is_empty()),
            description: g.description.filter(|d| !d.is_empty()),
            featured: g.featured,
        })
        .collect()
}

/// One version, from `GET /version/{id}`, `GET /project/{id}/version`, or
/// `POST /version_files`.
#[derive(Debug, Clone, Deserialize)]
struct RawVersion {
    id: String,
    project_id: String,
    #[serde(default)]
    name: String,
    version_number: String,
    version_type: String,
    date_published: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    files: Vec<RawFile>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    /// Release notes. The list endpoint is asked not to send these, and a version may
    /// carry `null`, so it is optional on both paths.
    #[serde(default)]
    changelog: Option<String>,
}

/// One file attached to a version.
#[derive(Debug, Clone, Deserialize)]
struct RawFile {
    url: String,
    filename: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    hashes: RawHashes,
}

/// A file's hashes. Modrinth publishes both, but neither is required to parse.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawHashes {
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha512: Option<String>,
}

/// One dependency edge on a version.
#[derive(Debug, Clone, Deserialize)]
struct RawDependency {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    version_id: Option<String>,
    dependency_type: String,
}

#[cfg(test)]
mod tests;
