---
name: mod-sources
description: Modrinth and CurseForge API usage — endpoints, headers, rate limits, content types, install targets, null download URLs, fingerprints, and the Source trait. Read before touching gcl-core sources/.
---

Full detail: `docs/research/2026-09-06-modrinth-and-curseforge-apis.md`. Items marked VERIFY there
need api-verifier confirmation before code depends on them.

## Source trait

```rust
#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;                       // Modrinth | CurseForge
    fn supported_types(&self) -> &[ContentType];    // Mod, Modpack, ResourcePack, Shader, DataPack, World
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, Error>;
    async fn project(&self, id: &str) -> Result<Project, Error>;
    async fn versions(&self, id: &str, filter: &VersionFilter) -> Result<Vec<Version>, Error>;
    async fn resolve_by_hash(&self, hashes: &[FileHash]) -> Result<Vec<Version>, Error>;
}
```

`Version` carries `files[] { url: Option<Url>, filename, size, sha1: Option<String>, sha512: Option<String>, fingerprint: Option<u32>, primary }`
and `dependencies[] { project_id, version_id, kind: Required | Optional | Incompatible | Embedded }`.

Constructors: `ModrinthSource::with_base_url(client: HttpClient, base: String)`,
`CurseForgeSource::with_base_url(client: HttpClient, base: String, api_key: String)`. Production
code passes the real base URLs.

## Modrinth

- Base `https://api.modrinth.com/v2`. User-Agent is `gcl_core::USER_AGENT`. 300 requests per minute; honor `X-Ratelimit-Remaining` and `X-Ratelimit-Reset`.
- Search: `GET /search?query=&facets=<json>&index=&offset=&limit=`. Facets: outer array AND, inner OR. `project_type:mod|modpack|resourcepack|shader|datapack`. `world` is VERIFY.
- Versions: `GET /project/{id}/version?loaders=["fabric"]&game_versions=["1.20.1"]&include_changelog=false`.
- Hash lookup: `POST /version_files { hashes, algorithm: "sha1" }`.
- Files have both `sha1` and `sha512`. Check sha1 after download.

## CurseForge

- Base `https://api.curseforge.com`. Header `x-api-key`. Game id `432`. Page size max 50, `index + pageSize <= 10000`.
- Class ids: load at startup from `GET /v1/categories?gameId=432&classesOnly=true` and cache; expected mods 6, modpacks 4471, resource packs 12, worlds 17, shaders 6552 (VERIFY), data packs 6945 (VERIFY).
- Loader ids: Forge 1, Fabric 4, Quilt 5, NeoForge 6.
- Files: `GET /v1/mods/{id}/files?gameVersion=&modLoaderType=`. `downloadUrl` may be null.
- Batch: `POST /v1/mods/files { fileIds }` in chunks of 50.
- Fingerprint: MurmurHash2 32-bit, seed 1, over bytes with 9, 10, 13, 32 removed. `POST /v1/fingerprints { fingerprints }`.

## Null download URL

A null `downloadUrl` means the author opted out of third-party distribution. Return
`Error::ManualDownload { page_url, expected_fingerprint, file_name }`. The UI and CLI show the
page and accept a dropped file, then verify the fingerprint.

## Install targets

| ContentType | Directory under `.minecraft/` |
|---|---|
| Mod | `mods/` |
| ResourcePack | `resourcepacks/` |
| Shader | `shaderpacks/` |
| DataPack | `saves/<world>/datapacks/` (caller supplies the world) |
| World | `saves/` (zip extracted, one folder) |
| Modpack | new instance via `modpacks` |

## Do not

- Do not build a second `reqwest::Client`. Use `HttpClient`.
- Do not send the CurseForge key anywhere but `api.curseforge.com`.
- Do not hardcode class ids as the only source; the runtime fetch wins.
