# Modrinth and CurseForge APIs

Researched 2026-09-06. Source of truth for the `mod-sources` and `modpack-formats` skills.
Items marked **VERIFY** were not confirmed against the live API during research. The
`api-verifier` agent must confirm them before a parser depends on them.

## 1. Modrinth API v2

- Base: `https://api.modrinth.com/v2`
- Staging: `https://staging-api.modrinth.com/v2`
- Docs: https://docs.modrinth.com/api/ and https://docs.modrinth.com/openapi.yaml

### Headers

- `User-Agent` is required and must identify the app and a contact, for example
  `sixdd6/grid-craft-launcher/0.1.0 (sixdd6@gmail.com)`. Generic agents get blocked.
- No auth needed for read endpoints.

### Rate limit

- 300 requests per minute per IP. Headers: `X-Ratelimit-Limit`, `X-Ratelimit-Remaining`,
  `X-Ratelimit-Reset` (seconds). Back off when `Remaining` reaches zero.

### Endpoints

- `GET /search?query=&facets=&index=&offset=&limit=` (limit max 100).
  `index` is `relevance|downloads|follows|newest|updated`.
  Facets: JSON array of arrays. Inner array is OR, outer is AND.
  Example: `[["project_type:mod"],["categories:fabric"],["versions:1.20.1"]]`.
  Project types: `mod`, `modpack`, `resourcepack`, `shader`, `datapack`. **VERIFY** whether
  `world` exists as a project type at time of implementation; if not, worlds are Modrinth-unsupported.
  Hits carry `project_id`, `slug`, `title`, `description`, `author`, `icon_url`, `downloads`,
  `versions[]`, `categories[]`, `client_side`, `server_side`, `project_type`.
- `GET /project/{id|slug}` → full project.
- `GET /project/{id|slug}/version?loaders=["fabric"]&game_versions=["1.20.1"]&include_changelog=false`
  → `[{ id, project_id, name, version_number, version_type, game_versions[], loaders[],
  dependencies[] { version_id?, project_id?, dependency_type }, files[] { hashes { sha1, sha512 },
  url, filename, primary, size, file_type } }]`.
- `GET /version/{id}` → one version.
- `POST /version_files { hashes: [], algorithm: "sha1"|"sha512" }` → map hash → version.
- `GET /projects?ids=[...]` and `GET /versions?ids=[...]` for batch lookups.
- Datapacks: a version with loader `datapack` installs into `<world>/datapacks/`. Modrinth also
  publishes many datapacks as mods with loader `fabric`/`forge` wrappers; use the version's
  `loaders` to decide the target folder.

### Install target by project type

| project_type | target directory under instance |
|---|---|
| mod | `mods/` |
| resourcepack | `resourcepacks/` |
| shader | `shaderpacks/` |
| datapack | `saves/<world>/datapacks/` (user picks world) |
| modpack | instance created from `.mrpack` |

### .mrpack format

Zip with `modrinth.index.json` at the root and optional `overrides/`, `client-overrides/`,
`server-overrides/`.

```json
{
  "formatVersion": 1,
  "game": "minecraft",
  "versionId": "1.0.0",
  "name": "Pack",
  "summary": "...",
  "dependencies": {
    "minecraft": "1.20.1",
    "fabric-loader": "0.15.0"
  },
  "files": [
    {
      "path": "mods/example.jar",
      "hashes": { "sha1": "...", "sha512": "..." },
      "env": { "client": "required|optional|unsupported", "server": "required|optional|unsupported" },
      "downloads": ["https://cdn.modrinth.com/..."],
      "fileSize": 12345
    }
  ]
}
```

- Dependency keys: `minecraft`, `forge`, `neoforge`, `fabric-loader`, `quilt-loader`.
- Skip files with `env.client == "unsupported"`. Ask about `optional` (MVP: install them).
- Allowed download hosts: `cdn.modrinth.com`, `github.com`, `raw.githubusercontent.com`, `gitlab.com`.
- Apply `overrides/` then `client-overrides/`. Later folders win.
- Both hashes must be checked after download.

## 2. CurseForge Core API v1

- Base: `https://api.curseforge.com`
- Docs: https://docs.curseforge.com/rest-api/
- Header: `x-api-key: <key>`. Keys come from the CurseForge for Studios console. The key is a
  secret: read from `CURSEFORGE_API_KEY` env or the launcher config, never commit it.
- Minecraft `gameId` is `432`.

### Class IDs (gameId 432)

| Content | classId |
|---|---|
| Mods | 6 |
| Modpacks | 4471 |
| Resource packs | 12 |
| Worlds | 17 |
| Shaders | 6552 (**VERIFY**) |
| Data packs | 6945 (**VERIFY**) |

Verify shader and data pack IDs with `GET /v1/categories?gameId=432&classesOnly=true`, which
lists every class with `id` and `slug`. The launcher should load class IDs from that call at
runtime and cache them rather than hardcode them.

### Endpoints

- `GET /v1/mods/search?gameId=432&classId=&categoryId=&gameVersion=&searchFilter=&sortField=&sortOrder=&modLoaderType=&index=&pageSize=`
  `pageSize` max 50, `index + pageSize <= 10000`.
  `modLoaderType`: 1 Forge, 4 Fabric, 5 Quilt, 6 NeoForge.
  `sortField`: 1 Featured, 2 Popularity, 3 LastUpdated, 4 Name, 5 Author, 6 TotalDownloads.
- `GET /v1/mods/{modId}` → `{ data: { id, name, slug, summary, classId, logo { url }, latestFiles[], latestFilesIndexes[] } }`
- `GET /v1/mods/{modId}/files?gameVersion=&modLoaderType=&index=&pageSize=`
  → `{ data: [{ id, modId, displayName, fileName, releaseType, fileStatus, hashes[] { value, algo (1 sha1, 2 md5) },
  fileDate, fileLength, downloadUrl (nullable), gameVersions[], dependencies[] { modId, relationType },
  fileFingerprint }], pagination }`.
- `GET /v1/mods/{modId}/files/{fileId}` → one file.
- `GET /v1/mods/{modId}/files/{fileId}/download-url` → `{ data: "https://..." }`.
- `POST /v1/mods { modIds: [] }` and `POST /v1/mods/files { fileIds: [] }` batch lookups.
- `POST /v1/fingerprints { fingerprints: [] }` → `{ data: { exactMatches[] { id, file }, unmatchedFingerprints[] } }`.
- `GET /v1/minecraft/version`, `GET /v1/minecraft/modloader?version=`.

### Download URL rules

- `downloadUrl` is null when the author disabled third-party distribution
  (`allowModDistribution: false`). The launcher must then show the file's CurseForge page URL
  (`https://www.curseforge.com/minecraft/<class-slug>/<mod-slug>/download/<fileId>`) and let the
  user drop the file in; then verify by fingerprint.
- Do not construct a CDN URL to bypass a null `downloadUrl`. Prism Launcher and the Modrinth App
  both refuse; this launcher does the same (decision 2026-09-06).
- `relationType`: 1 EmbeddedLibrary, 2 OptionalDependency, 3 RequiredDependency, 4 Tool,
  5 Incompatible, 6 Include. Resolve type 3 recursively when installing.

### Fingerprint

MurmurHash2 (32-bit, the original `MurmurHash2` not `MurmurHash2A`), seed `1`, computed over
the file bytes after removing every byte equal to 9, 10, 13, or 32. Test against a known
CurseForge file before trusting an implementation.

### Modpack manifest format

Zip with `manifest.json`, `modlist.html` (ignore), and `overrides/` (name given by
`manifest.overrides`, usually `overrides`).

```json
{
  "manifestVersion": 1,
  "manifestType": "minecraftModpack",
  "name": "Pack",
  "version": "1.0",
  "author": "...",
  "overrides": "overrides",
  "minecraft": {
    "version": "1.20.1",
    "modLoaders": [{ "id": "forge-47.2.0", "primary": true }]
  },
  "files": [{ "projectID": 238222, "fileID": 5246076, "required": true }]
}
```

- `modLoaders[].id` is `<loader>-<version>`: `forge-47.2.0`, `neoforge-21.1.65`,
  `fabric-0.15.0`, `quilt-0.21.0`.
- Resolve every `files[]` entry with `POST /v1/mods/files` in batches of 50 and place each by
  its class (mods to `mods/`, resource packs to `resourcepacks/`, and so on).
- Copy `overrides/` into the instance after downloads.

## 3. Gotchas

- Modrinth blocks generic User-Agents. Set it once on the shared HTTP client.
- CurseForge without a key is a hard error at startup for CurseForge features only. Modrinth
  and everything else must keep working.
- CurseForge search needs at least one filter beyond `gameId`.
- Modrinth version filters are JSON arrays passed as query strings, URL-encoded.
- Both catalogs can serve the same file. Dedupe by sha1 in the download cache.
- CurseForge shader packs and data packs may not be exposed for every key tier. Detect a 403
  and disable that class in the UI instead of failing.

## Sources

- https://docs.modrinth.com/api/
- https://docs.modrinth.com/api/operations/searchprojects/
- https://docs.modrinth.com/api/operations/getprojectversions/
- https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack
- https://docs.curseforge.com/rest-api/
- https://github.com/meza/curseforge-fingerprint
- https://github.com/aternosorg/php-curseforge-api
