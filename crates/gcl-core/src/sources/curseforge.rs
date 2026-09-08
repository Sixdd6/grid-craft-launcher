//! CurseForge [`Source`] client: search, project, files, batch lookups, and
//! fingerprint matching against the CurseForge Core API v1.
//!
//! Endpoint detail lives in the `mod-sources` skill. This module builds the URLs, maps
//! the JSON onto [`crate::sources::types`], and turns HTTP failures into [`Error`].
//!
//! The API key is a secret. Every method carries `#[tracing::instrument(skip(self))]`
//! so the key never reaches a span field, the key travels only in the `x-api-key`
//! request header, and [`CurseForge`]'s [`std::fmt::Debug`] prints `api_key: <set>`.

use async_trait::async_trait;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use crate::http::HttpClient;
use crate::instances::model::Loader;

use super::{
    ContentKind, Dependency, DependencyKind, Error, Project, ReleaseKind, SearchHit, SearchPage,
    SearchQuery, Source, SourceId, Version, VersionFile, VersionFilter, pack_page_url, page_url,
};

/// Production base URL for the CurseForge Core API.
pub const BASE: &str = "https://api.curseforge.com";

/// CurseForge's game id for Minecraft.
pub const GAME_ID: u32 = 432;

/// This source's id, used in every error and mapped value.
const ID: SourceId = SourceId::CurseForge;

/// Header the API key travels in. It goes nowhere else: not a URL, not a log, not an
/// error message.
const KEY_HEADER: &str = "x-api-key";

/// Largest page CurseForge serves, and the batch size for `POST /v1/mods` and
/// `POST /v1/mods/files`.
pub const PAGE_SIZE: usize = 50;

/// Content kinds CurseForge can search and resolve.
///
/// Shaders and data packs are listed here, but [`Source::search`] still rejects them
/// when the live `classesOnly` response has no such class: their class ids are the two
/// unverified rows in the research doc, so the runtime answer wins over this list.
const KINDS: &[ContentKind] = &[
    ContentKind::Mod,
    ContentKind::ResourcePack,
    ContentKind::Shader,
    ContentKind::DataPack,
    ContentKind::World,
];

/// Loader names CurseForge reports inside a file's `gameVersions`.
const LOADER_NAMES: [&str; 4] = ["fabric", "forge", "quilt", "neoforge"];

/// Bytes escaped in a query-string value: everything but the unreserved set.
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Class ids for Minecraft, read once from `GET /v1/categories?classesOnly=true`.
///
/// The four required classes always exist. Shaders and data packs are optional: a
/// response without them leaves the field `None`, and searching for that kind then
/// fails with [`Error::UnsupportedKind`].
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClassIds {
    /// Class id of `mc-mods`.
    pub mods: u32,
    /// Class id of `modpacks`.
    pub modpacks: u32,
    /// Class id of `texture-packs`.
    pub resource_packs: u32,
    /// Class id of `worlds`.
    pub worlds: u32,
    /// Class id of `shaders`, when the game has that class.
    pub shaders: Option<u32>,
    /// Class id of `data-packs`, when the game has that class.
    pub data_packs: Option<u32>,
}

impl ClassIds {
    /// Maps a class id back onto a [`ContentKind`], or `None` for a class this
    /// launcher does not install as content (a modpack, or a class it does not know).
    fn kind_of(&self, class_id: u32) -> Option<ContentKind> {
        if class_id == self.mods {
            Some(ContentKind::Mod)
        } else if class_id == self.resource_packs {
            Some(ContentKind::ResourcePack)
        } else if class_id == self.worlds {
            Some(ContentKind::World)
        } else if self.shaders == Some(class_id) {
            Some(ContentKind::Shader)
        } else if self.data_packs == Some(class_id) {
            Some(ContentKind::DataPack)
        } else {
            None
        }
    }

    /// Class id to search with for a content kind, or `None` when this game has no
    /// such class.
    fn id_of(&self, kind: ContentKind) -> Option<u32> {
        match kind {
            ContentKind::Mod => Some(self.mods),
            ContentKind::ResourcePack => Some(self.resource_packs),
            ContentKind::World => Some(self.worlds),
            ContentKind::Shader => self.shaders,
            ContentKind::DataPack => self.data_packs,
        }
    }
}

/// The CurseForge content source.
pub struct CurseForge {
    http: HttpClient,
    base: String,
    api_key: String,
    classes: OnceCell<ClassIds>,
}

impl std::fmt::Debug for CurseForge {
    /// Prints the base URL and `api_key: <set>`. The key itself never appears.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurseForge")
            .field("base", &self.base)
            .field("api_key", &format_args!("<set>"))
            .finish()
    }
}

impl CurseForge {
    /// Builds a client against the live API at [`BASE`].
    pub fn new(http: HttpClient, api_key: String) -> Self {
        CurseForge::with_base_url(http, api_key, BASE.to_string())
    }

    /// Builds a client against `base`, for tests that point at a mock server. Any
    /// trailing slash is trimmed.
    pub fn with_base_url(http: HttpClient, api_key: String, base: String) -> Self {
        CurseForge {
            http,
            base: base.trim_end_matches('/').to_string(),
            api_key,
            classes: OnceCell::new(),
        }
    }

    /// Fetches the Minecraft class ids, once per client.
    ///
    /// The result is cached in a [`OnceCell`]; there is no disk cache in the MVP, so a
    /// new client fetches again. A response missing `mc-mods`, `modpacks`,
    /// `texture-packs`, or `worlds` is [`Error::BadResponse`].
    #[tracing::instrument(skip(self))]
    pub async fn class_ids(&self) -> Result<ClassIds, Error> {
        let cached = self
            .classes
            .get_or_try_init(|| self.fetch_class_ids())
            .await?;
        Ok(cached.clone())
    }

    /// Lists one mod's files, newest first, with the filter's Minecraft version and
    /// first loader applied.
    ///
    /// One page of [`PAGE_SIZE`] files is enough for the MVP: the launcher shows the
    /// newest releases, not the full history. Paging comes with the version picker.
    #[tracing::instrument(skip(self))]
    pub async fn mod_files(&self, mod_id: u32, f: &VersionFilter) -> Result<Vec<Version>, Error> {
        let loader = f.loaders.iter().find_map(|l| loader_type_of_name(l));
        self.fetch_files(mod_id, f.minecraft.as_deref(), loader)
            .await
    }

    /// Lists a modpack's files, whose primary file is the CurseForge pack zip.
    ///
    /// Same request as [`CurseForge::mod_files`] without a loader filter: a pack states
    /// its loader in `manifest.json`, not in the file's `gameVersions`.
    #[tracing::instrument(skip(self))]
    pub async fn pack_files(
        &self,
        mod_id: u32,
        minecraft: Option<&str>,
    ) -> Result<Vec<Version>, Error> {
        self.fetch_files(mod_id, minecraft, None).await
    }

    /// Resolves a modpack's id or slug to its numeric mod id.
    ///
    /// [`Source::project`] refuses a modpack, because a modpack is not a
    /// [`ContentKind`]; this is the same lookup with that check left out, for
    /// [`crate::modpacks::fetch_pack`].
    #[tracing::instrument(skip(self))]
    pub async fn resolve_pack_id(&self, id_or_slug: &str) -> Result<u32, Error> {
        if let Ok(id) = id_or_slug.parse::<u32>() {
            return Ok(id);
        }
        Ok(self.fetch_mod(id_or_slug).await?.id)
    }

    /// Fetches one mod by numeric id, or by slug through the search endpoint.
    async fn fetch_mod(&self, id_or_slug: &str) -> Result<RawMod, Error> {
        match id_or_slug.parse::<u32>() {
            Ok(id) => {
                let url = format!("{}/v1/mods/{id}", self.base);
                let body: Envelope<RawMod> = self
                    .get(&url)
                    .await
                    .map_err(|e| map_err(e, "project", Some(id_or_slug)))?;
                Ok(body.data)
            }
            Err(_) => {
                let url = format!(
                    "{}/v1/mods/search?gameId={GAME_ID}&slug={}",
                    self.base,
                    encode(id_or_slug)
                );
                let body: Envelope<Vec<RawMod>> = self
                    .get(&url)
                    .await
                    .map_err(|e| map_err(e, "project", Some(id_or_slug)))?;
                body.data
                    .into_iter()
                    // The search endpoint matches a slug loosely, so the answer can
                    // differ in case from what the user typed.
                    .find(|m| m.slug.eq_ignore_ascii_case(id_or_slug))
                    .ok_or_else(|| Error::NotFound {
                        source_id: ID,
                        id: id_or_slug.to_string(),
                    })
            }
        }
    }

    /// Resolves file ids to versions with `POST /v1/mods/files`, in chunks of
    /// [`PAGE_SIZE`]. File ids the server does not know are dropped.
    #[tracing::instrument(skip(self))]
    pub async fn files_batch(&self, file_ids: &[u32]) -> Result<Vec<Version>, Error> {
        let url = format!("{}/v1/mods/files", self.base);
        let mut out = Vec::with_capacity(file_ids.len());
        for chunk in file_ids.chunks(PAGE_SIZE) {
            let body = serde_json::json!({ "fileIds": chunk });
            let raw: Envelope<Vec<RawFile>> = self
                .post(&url, &body)
                .await
                .map_err(|e| map_err(e, "files", None))?;
            for file in raw.data {
                out.push(map_file(file)?);
            }
        }
        Ok(out)
    }

    /// Resolves mod ids to projects with `POST /v1/mods`, in chunks of [`PAGE_SIZE`].
    ///
    /// A mod whose class is not a [`ContentKind`] — a modpack, or a class this
    /// launcher does not know — is dropped, the same as an unmappable search hit.
    #[tracing::instrument(skip(self))]
    pub async fn mods_batch(&self, mod_ids: &[u32]) -> Result<Vec<Project>, Error> {
        if mod_ids.is_empty() {
            return Ok(Vec::new());
        }
        let classes = self.class_ids().await?;
        let url = format!("{}/v1/mods", self.base);
        let mut out = Vec::with_capacity(mod_ids.len());
        for chunk in mod_ids.chunks(PAGE_SIZE) {
            let body = serde_json::json!({ "modIds": chunk });
            let raw: Envelope<Vec<RawMod>> = self
                .post(&url, &body)
                .await
                .map_err(|e| map_err(e, "mods", None))?;
            out.extend(
                raw.data
                    .into_iter()
                    .filter_map(|m| map_project(m, &classes)),
            );
        }
        Ok(out)
    }

    /// GETs `{base}/v1/categories?gameId=432&classesOnly=true` and folds it into
    /// [`ClassIds`] by class slug.
    async fn fetch_class_ids(&self) -> Result<ClassIds, Error> {
        let url = format!(
            "{}/v1/categories?gameId={GAME_ID}&classesOnly=true",
            self.base
        );
        let raw: Envelope<Vec<RawCategory>> = self
            .get(&url)
            .await
            .map_err(|e| map_err(e, "categories", None))?;
        let find = |slugs: &[&str]| -> Option<u32> {
            raw.data
                .iter()
                .find(|c| slugs.contains(&c.slug.as_str()))
                .map(|c| c.id)
        };
        let required = |slugs: &[&str]| -> Result<u32, Error> {
            find(slugs).ok_or_else(|| Error::BadResponse {
                source_id: ID,
                what: "categories",
                detail: format!("no class with slug {}", slugs.join(" or ")),
            })
        };
        Ok(ClassIds {
            mods: required(&["mc-mods", "mods"])?,
            modpacks: required(&["modpacks"])?,
            resource_packs: required(&["texture-packs", "resource-packs"])?,
            worlds: required(&["worlds"])?,
            shaders: find(&["shaders"]),
            data_packs: find(&["data-packs"]),
        })
    }

    /// GETs `{base}/v1/mods/{mod_id}/files` with the optional filters and maps the body.
    async fn fetch_files(
        &self,
        mod_id: u32,
        minecraft: Option<&str>,
        loader_type: Option<u32>,
    ) -> Result<Vec<Version>, Error> {
        let mut url = format!(
            "{}/v1/mods/{mod_id}/files?pageSize={PAGE_SIZE}&index=0",
            self.base
        );
        if let Some(mc) = minecraft {
            url.push_str("&gameVersion=");
            url.push_str(&encode(mc));
        }
        if let Some(loader) = loader_type {
            url.push_str(&format!("&modLoaderType={loader}"));
        }
        let id = mod_id.to_string();
        let raw: Envelope<Vec<RawFile>> = self
            .get(&url)
            .await
            .map_err(|e| map_err(e, "files", Some(&id)))?;
        let mut out = raw
            .data
            .into_iter()
            .map(map_file)
            .collect::<Result<Vec<_>, Error>>()?;
        // Newest first. The sort is stable, so files sharing a date keep the API's order.
        out.sort_by(|a, b| b.published.cmp(&a.published));
        Ok(out)
    }

    /// GETs a URL with the API key header.
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
    ) -> Result<T, crate::http::Error> {
        self.http
            .get_json_with_headers(url, &[(KEY_HEADER, self.api_key.as_str())])
            .await
    }

    /// POSTs a JSON body with the API key header.
    async fn post<T: serde::de::DeserializeOwned, B: Serialize>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T, crate::http::Error> {
        self.http
            .post_json_with_headers(url, body, &[(KEY_HEADER, self.api_key.as_str())])
            .await
    }
}

#[async_trait]
impl Source for CurseForge {
    fn id(&self) -> SourceId {
        ID
    }

    fn supported_kinds(&self) -> &[ContentKind] {
        KINDS
    }

    fn as_curseforge(&self) -> Option<&CurseForge> {
        Some(self)
    }

    #[tracing::instrument(skip(self))]
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, Error> {
        let classes = self.class_ids().await?;
        let mut url = format!(
            "{}/v1/mods/search?gameId={GAME_ID}&sortField=2&sortOrder=desc&pageSize={}&index={}&searchFilter={}",
            self.base,
            // CurseForge rejects a page larger than `PAGE_SIZE`, so a caller asking for
            // more gets one full page instead of an error.
            q.limit.min(PAGE_SIZE as u32),
            q.offset,
            encode(&q.text)
        );
        if let Some(kind) = q.kind {
            // A kind whose class the live response does not list is unsupported here,
            // whatever `supported_kinds` says.
            let class_id = classes.id_of(kind).ok_or(Error::UnsupportedKind {
                source_id: ID,
                kind,
            })?;
            url.push_str(&format!("&classId={class_id}"));
        }
        if let Some(mc) = &q.minecraft {
            url.push_str("&gameVersion=");
            url.push_str(&encode(mc));
        }
        if let (Some(ContentKind::Mod), Some(loader)) = (q.kind, q.loader)
            && let Some(loader_type) = loader_type_of(loader)
        {
            url.push_str(&format!("&modLoaderType={loader_type}"));
        }

        let raw: Envelope<Vec<RawMod>> = self
            .get(&url)
            .await
            .map_err(|e| map_err(e, "search", None))?;
        let pagination = raw.pagination.unwrap_or_default();
        let hits = raw
            .data
            .into_iter()
            .filter_map(|m| {
                let kind = classes.kind_of(m.class_id?)?;
                Some(SearchHit {
                    source: ID,
                    page_url: mod_page_url(&m, kind),
                    project_id: m.id.to_string(),
                    title: m.name,
                    description: m.summary,
                    author: m
                        .authors
                        .first()
                        .map(|a| a.name.clone())
                        .unwrap_or_default(),
                    kind,
                    is_pack: false,
                    downloads: m.download_count,
                    icon_url: m.logo.and_then(|l| l.thumbnail_url),
                    slug: m.slug,
                })
            })
            .collect();
        Ok(SearchPage {
            hits,
            total: pagination.total_count,
            offset: pagination.index,
        })
    }

    #[tracing::instrument(skip(self))]
    async fn search_packs(&self, q: &SearchQuery) -> Result<SearchPage, Error> {
        let classes = self.class_ids().await?;
        let mut url = format!(
            "{}/v1/mods/search?gameId={GAME_ID}&sortField=2&sortOrder=desc&pageSize={}&index={}&searchFilter={}&classId={}",
            self.base,
            q.limit.min(PAGE_SIZE as u32),
            q.offset,
            encode(&q.text),
            classes.modpacks
        );
        if let Some(mc) = &q.minecraft {
            url.push_str("&gameVersion=");
            url.push_str(&encode(mc));
        }
        // No `modLoaderType`: a pack states its loader in `manifest.json`, and the filter
        // would drop packs whose files do not name one.

        let raw: Envelope<Vec<RawMod>> = self
            .get(&url)
            .await
            .map_err(|e| map_err(e, "search", None))?;
        let pagination = raw.pagination.unwrap_or_default();
        let hits = raw
            .data
            .into_iter()
            // The class filter is the server's job; this drops anything else it sends,
            // so a hit is never reported as a pack when it is a mod.
            .filter(|m| m.class_id == Some(classes.modpacks))
            .map(|m| SearchHit {
                source: ID,
                page_url: pack_page_url(ID, &m.slug),
                project_id: m.id.to_string(),
                title: m.name,
                description: m.summary,
                author: m
                    .authors
                    .first()
                    .map(|a| a.name.clone())
                    .unwrap_or_default(),
                // A modpack has no `ContentKind` of its own; `is_pack` is what a caller
                // reads. `Mod` is the placeholder, as it is in every pack hit.
                kind: ContentKind::Mod,
                is_pack: true,
                downloads: m.download_count,
                icon_url: m.logo.and_then(|l| l.thumbnail_url),
                slug: m.slug,
            })
            .collect();
        Ok(SearchPage {
            hits,
            total: pagination.total_count,
            offset: pagination.index,
        })
    }

    #[tracing::instrument(skip(self))]
    async fn project(&self, id_or_slug: &str) -> Result<Project, Error> {
        let classes = self.class_ids().await?;
        let raw = self.fetch_mod(id_or_slug).await?;
        let class_id = raw.class_id.ok_or_else(|| Error::BadResponse {
            source_id: ID,
            what: "classId",
            detail: "missing".to_string(),
        })?;
        // A modpack is not a `ContentKind`; Task 7 fetches its files through
        // [`CurseForge::pack_files`] instead of this method.
        if class_id == classes.modpacks {
            return Err(Error::BadResponse {
                source_id: ID,
                what: "classId",
                detail: "modpack".to_string(),
            });
        }
        map_project(raw, &classes).ok_or_else(|| Error::BadResponse {
            source_id: ID,
            what: "classId",
            detail: class_id.to_string(),
        })
    }

    #[tracing::instrument(skip(self))]
    async fn description(&self, project_id: &str) -> Result<String, Error> {
        let id = project_id.parse::<u32>().map_err(|_| Error::NotFound {
            source_id: ID,
            id: project_id.to_string(),
        })?;
        // VERIFY: the envelope is assumed to be `{"data": "<html>"}`, like every other
        // CurseForge endpoint this client parses. No `CURSEFORGE_API_KEY` was
        // available to record a real response, so the fixture is synthetic; see
        // `tests/fixtures/curseforge/README.md` and the `mod-sources` skill.
        let url = format!("{}/v1/mods/{id}/description", self.base);
        let body: Envelope<String> = self
            .get(&url)
            .await
            .map_err(|e| map_err(e, "description", Some(project_id)))?;
        Ok(body.data)
    }

    #[tracing::instrument(skip(self))]
    async fn versions(&self, project_id: &str, f: &VersionFilter) -> Result<Vec<Version>, Error> {
        let id = project_id.parse::<u32>().map_err(|_| Error::NotFound {
            source_id: ID,
            id: project_id.to_string(),
        })?;
        self.mod_files(id, f).await
    }

    #[tracing::instrument(skip(self))]
    async fn version(&self, version_id: &str) -> Result<Version, Error> {
        let id = version_id.parse::<u32>().map_err(|_| Error::NotFound {
            source_id: ID,
            id: version_id.to_string(),
        })?;
        self.files_batch(&[id])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::NotFound {
                source_id: ID,
                id: version_id.to_string(),
            })
    }

    #[tracing::instrument(skip(self))]
    async fn resolve_by_hash(&self, _sha1: &[String]) -> Result<Vec<Version>, Error> {
        // CurseForge has no hash lookup. Callers use `resolve_by_fingerprint`.
        Ok(Vec::new())
    }

    #[tracing::instrument(skip(self))]
    async fn resolve_by_fingerprint(&self, fps: &[u32]) -> Result<Vec<Version>, Error> {
        if fps.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/v1/fingerprints", self.base);
        let mut out = Vec::with_capacity(fps.len());
        for chunk in fps.chunks(PAGE_SIZE) {
            let body = serde_json::json!({ "fingerprints": chunk });
            let raw: Envelope<RawFingerprints> = self
                .post(&url, &body)
                .await
                .map_err(|e| map_err(e, "fingerprints", None))?;
            // Unmatched and partial fingerprints are dropped; only exact matches count.
            for m in raw.data.exact_matches {
                out.push(map_file(m.file)?);
            }
        }
        Ok(out)
    }
}

/// Builds the browser URL of one file inside a project's page, which is where the user
/// downloads a file whose `downloadUrl` is null.
///
/// Nothing here constructs a CDN URL: an author who opted out of third-party
/// distribution gets their page visited, not their file mirrored.
pub fn file_page_url(project_page_url: &str, file_id: &str) -> String {
    format!("{}/files/{file_id}", project_page_url.trim_end_matches('/'))
}

/// Percent-encodes one query-string value.
fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_VALUE).to_string()
}

/// CurseForge's `modLoaderType` for a loader, or `None` for vanilla.
fn loader_type_of(loader: Loader) -> Option<u32> {
    match loader {
        Loader::Forge => Some(1),
        Loader::Fabric => Some(4),
        Loader::Quilt => Some(5),
        Loader::NeoForge => Some(6),
        Loader::None => None,
    }
}

/// Same mapping as [`loader_type_of`], from the lowercase loader name a
/// [`VersionFilter`] carries.
fn loader_type_of_name(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

/// Page URL of one mod: the API's own `links.websiteUrl` when it has one, else the
/// class-slug form built by [`page_url`].
fn mod_page_url(m: &RawMod, kind: ContentKind) -> String {
    match m.links.as_ref().and_then(|l| l.website_url.as_deref()) {
        Some(url) if !url.is_empty() => url.to_string(),
        _ => page_url(ID, kind, &m.slug),
    }
}

/// Maps one API mod onto [`Project`], or `None` when its class is not a
/// [`ContentKind`].
fn map_project(m: RawMod, classes: &ClassIds) -> Option<Project> {
    let kind = classes.kind_of(m.class_id?)?;
    Some(Project {
        source: ID,
        page_url: mod_page_url(&m, kind),
        id: m.id.to_string(),
        title: m.name,
        description: m.summary,
        kind,
        slug: m.slug,
    })
}

/// Maps one API file onto [`Version`]. CurseForge has no version entity: a file is the
/// version, and its `displayName` is both the name and the number.
fn map_file(f: RawFile) -> Result<Version, Error> {
    let kind = match f.release_type {
        1 => ReleaseKind::Release,
        2 => ReleaseKind::Beta,
        3 => ReleaseKind::Alpha,
        other => {
            return Err(Error::BadResponse {
                source_id: ID,
                what: "releaseType",
                detail: other.to_string(),
            });
        }
    };
    // `gameVersions` mixes Minecraft versions ("1.20.1") with loader names ("Fabric").
    let game_versions = f
        .game_versions
        .iter()
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
        .cloned()
        .collect();
    let loaders = f
        .game_versions
        .iter()
        .map(|v| v.to_ascii_lowercase())
        .filter(|v| LOADER_NAMES.contains(&v.as_str()))
        .collect();
    let url = f.download_url.filter(|u| !u.is_empty());
    let sha1 = f
        .hashes
        .iter()
        .find(|h| h.algo == 1)
        .map(|h| h.value.clone());
    let dependencies = f
        .dependencies
        .into_iter()
        .filter_map(|d| {
            let kind = match d.relation_type {
                1 => DependencyKind::Embedded,
                2 => DependencyKind::Optional,
                3 => DependencyKind::Required,
                5 => DependencyKind::Incompatible,
                // 4 Tool and 6 Include install nothing; drop them.
                _ => return None,
            };
            Some(Dependency {
                project_id: Some(d.mod_id.to_string()),
                version_id: None,
                kind,
            })
        })
        .collect();
    Ok(Version {
        source: ID,
        project_id: f.mod_id.to_string(),
        id: f.id.to_string(),
        name: f.display_name.clone(),
        number: f.display_name,
        kind,
        game_versions,
        loaders,
        published: f.file_date,
        files: vec![VersionFile {
            url,
            file_name: f.file_name,
            size: f.file_length,
            sha1,
            sha512: None,
            fingerprint: f.file_fingerprint,
            // A CurseForge file is alone in its version, so it is always the primary.
            primary: true,
        }],
        dependencies,
    })
}

/// Turns an HTTP failure into a source error. The API key is never part of `what` or
/// `id`, so nothing here can leak it.
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

/// Every CurseForge body wraps its payload in `data`.
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
    #[serde(default)]
    pagination: Option<RawPagination>,
}

/// Paging block on the list endpoints.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPagination {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    total_count: u64,
}

/// One row of `GET /v1/categories`.
#[derive(Debug, Deserialize)]
struct RawCategory {
    id: u32,
    slug: String,
}

/// One mod, from `GET /v1/mods/{id}`, `GET /v1/mods/search`, or `POST /v1/mods`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMod {
    id: u32,
    name: String,
    slug: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    class_id: Option<u32>,
    #[serde(default)]
    download_count: u64,
    #[serde(default)]
    authors: Vec<RawAuthor>,
    #[serde(default)]
    logo: Option<RawLogo>,
    #[serde(default)]
    links: Option<RawLinks>,
}

/// One author of a mod.
#[derive(Debug, Deserialize)]
struct RawAuthor {
    name: String,
}

/// A mod's logo. The thumbnail is the size the launcher shows in a list.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLogo {
    #[serde(default)]
    thumbnail_url: Option<String>,
}

/// A mod's outbound links.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLinks {
    #[serde(default)]
    website_url: Option<String>,
}

/// One file, from `GET /v1/mods/{id}/files`, `POST /v1/mods/files`, or a fingerprint
/// match.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFile {
    id: u64,
    mod_id: u64,
    display_name: String,
    file_name: String,
    release_type: u8,
    #[serde(default)]
    hashes: Vec<RawHash>,
    file_date: String,
    #[serde(default)]
    file_length: Option<u64>,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    file_fingerprint: Option<u32>,
}

/// One hash on a file. `algo` 1 is sha1, 2 is md5.
#[derive(Debug, Deserialize)]
struct RawHash {
    value: String,
    algo: u8,
}

/// One dependency edge on a file.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDependency {
    mod_id: u64,
    relation_type: u8,
}

/// `POST /v1/fingerprints` payload.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFingerprints {
    #[serde(default)]
    exact_matches: Vec<RawFingerprintMatch>,
}

/// One exact fingerprint match.
#[derive(Debug, Deserialize)]
struct RawFingerprintMatch {
    file: RawFile,
}

#[cfg(test)]
mod tests;
