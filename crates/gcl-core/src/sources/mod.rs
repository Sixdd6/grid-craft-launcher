//! Content sources: Modrinth and CurseForge.
//!
//! A [`Source`] searches and resolves projects and versions at one host. The
//! `mod-sources` skill has the endpoint and rate-limit detail for each source's
//! implementation; this module holds only the shared trait and data types.

pub mod curseforge;
pub mod fingerprint;
pub mod modrinth;
pub mod richtext;
pub mod types;

pub use types::{
    ContentKind, Dependency, DependencyKind, GalleryImage, LatestFileIndex, Project, ReleaseKind,
    SearchHit, SearchPage, SearchQuery, SourceId, Version, VersionFile, VersionFilter,
    pack_page_url, page_url,
};

use async_trait::async_trait;

/// Errors from a content source.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The underlying HTTP request failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// The requested project does not exist at this source.
    #[error("{source_id}: project {id} not found")]
    NotFound {
        /// Source that was queried.
        source_id: SourceId,
        /// Id or slug that was not found.
        id: String,
    },
    /// This source does not support the requested content kind.
    #[error("{source_id} does not support {kind:?}")]
    UnsupportedKind {
        /// Source that was queried.
        source_id: SourceId,
        /// Content kind that is not supported.
        kind: ContentKind,
    },
    /// This source cannot search modpacks.
    #[error("{source_id} does not support modpack search")]
    UnsupportedPacks {
        /// Source that was queried.
        source_id: SourceId,
    },
    /// This source is disabled, typically for a missing API key.
    #[error("{source_id} is disabled: {reason}")]
    Disabled {
        /// Source that is disabled.
        source_id: SourceId,
        /// Why it is disabled.
        reason: String,
    },
    /// The author opted this file out of third-party distribution; the user must
    /// download it by hand from the project's page.
    #[error("file {file_name} must be downloaded by hand from {page_url}")]
    ManualDownload {
        /// Project page to send the user to.
        page_url: String,
        /// Name of the file they need to fetch.
        file_name: String,
        /// Expected CurseForge fingerprint, to verify what they drop in.
        fingerprint: Option<u32>,
        /// Expected sha1, to verify what they drop in.
        sha1: Option<String>,
    },
    /// The source answered, but not in the shape this client expects.
    #[error("{source_id}: unexpected response for {what}: {detail}")]
    BadResponse {
        /// Source that answered.
        source_id: SourceId,
        /// What was being parsed when the shape did not match.
        what: &'static str,
        /// Detail of the mismatch.
        detail: String,
    },
    /// Reading or writing a local file failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// Path being read or written.
        path: std::path::PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Parsing a local JSON file failed.
    #[error("{path}: invalid JSON: {source}")]
    Json {
        /// Path of the file being parsed.
        path: std::path::PathBuf,
        /// Underlying parse error.
        source: serde_json::Error,
    },
}

/// A content source: searches and resolves projects and versions at one host.
///
/// Implementations must not build their own `reqwest::Client`; they take an
/// [`crate::http::HttpClient`] so tests can point them at a mock server.
#[async_trait]
pub trait Source: Send + Sync {
    /// Which source this is.
    fn id(&self) -> SourceId;
    /// Content kinds this source can search and resolve.
    fn supported_kinds(&self) -> &[ContentKind];
    /// Searches for projects matching `q`.
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, Error>;
    /// Searches for modpacks matching `q`.
    ///
    /// A modpack is not a [`ContentKind`], so it has its own search: `q.kind` and
    /// `q.loader` are ignored and every hit reports [`SearchHit::is_pack`] `true`. The
    /// default is [`Error::UnsupportedPacks`], for a source that has no modpacks.
    async fn search_packs(&self, q: &SearchQuery) -> Result<SearchPage, Error> {
        let _ = q;
        Err(Error::UnsupportedPacks {
            source_id: self.id(),
        })
    }
    /// Fetches one project by id or slug.
    async fn project(&self, id_or_slug: &str) -> Result<Project, Error>;
    /// Fetches a project's long description, in the source's own markup.
    ///
    /// Modrinth answers markdown (the project's `body`), CurseForge answers HTML.
    /// The caller converts with [`richtext::from_markdown`] or
    /// [`richtext::from_html`]. There is no default body: every source answers this.
    async fn description(&self, project_id: &str) -> Result<String, Error>;
    /// Fetches one version's release notes, in the source's own markup.
    ///
    /// Modrinth answers markdown (the version's `changelog`, from `GET /version/{id}`),
    /// CurseForge HTML (`GET /v1/mods/{id}/files/{fileId}/changelog`). The caller converts
    /// with [`richtext::from_markdown`] or [`richtext::from_html`], the same way
    /// [`Source::description`] is handled; [`crate::Launcher::version_notes`] does both.
    /// A version with no notes answers an empty string, not an error. There is no default
    /// body: every source answers this.
    async fn changelog(&self, project_id: &str, version_id: &str) -> Result<String, Error>;
    /// Lists a project's versions, optionally filtered.
    async fn versions(&self, project_id: &str, f: &VersionFilter) -> Result<Vec<Version>, Error>;
    /// Fetches one version by id.
    async fn version(&self, version_id: &str) -> Result<Version, Error>;
    /// Resolves versions by file sha1. CurseForge has no hash endpoint and always
    /// returns `Ok(vec![])`; use [`Source::resolve_by_fingerprint`] there instead.
    async fn resolve_by_hash(&self, sha1: &[String]) -> Result<Vec<Version>, Error>;
    /// Resolves versions by CurseForge fingerprint. Modrinth has no fingerprint
    /// endpoint and always returns `Ok(vec![])`.
    async fn resolve_by_fingerprint(&self, fps: &[u32]) -> Result<Vec<Version>, Error>;

    /// The concrete Modrinth client behind this source, when it is one.
    ///
    /// Modpack import needs endpoints that are not part of this trait, because a
    /// modpack is not a [`ContentKind`]: only [`crate::modpacks`] uses them. The
    /// default is `None`, so no other implementation has to care.
    fn as_modrinth(&self) -> Option<&modrinth::Modrinth> {
        None
    }

    /// The concrete CurseForge client behind this source, when it is one.
    ///
    /// See [`Source::as_modrinth`]. CurseForge additionally needs its batch endpoints
    /// here, which resolve a pack's file ids to files and their classes.
    fn as_curseforge(&self) -> Option<&curseforge::CurseForge> {
        None
    }
}

/// A shared, type-erased [`Source`].
pub type BoxSource = std::sync::Arc<dyn Source>;
