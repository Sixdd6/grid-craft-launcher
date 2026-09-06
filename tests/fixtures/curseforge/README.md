# CurseForge fixtures — SYNTHETIC

These fixtures are **SYNTHETIC**. No `CURSEFORGE_API_KEY` was available when they were built, so
nothing here is a live recording. Every file was constructed by hand from the public CurseForge
Core API docs at https://docs.curseforge.com/rest-api/, which describe the request/response
schemas (`Mod`, `File`, `FileHash`, `FileDependency`, `Category`, `Pagination`,
`FingerprintMatch`, `FingerprintsMatchesResult`) but do not expose fetchable JSON example bodies.

Verified from docs: every field name, its type, and its nesting come straight from the docs
schema listing — `Mod` (`id`, `gameId`, `name`, `slug`, `links`, `summary`, `status`,
`downloadCount`, `classId`, `categories`, `authors`, `logo`, `latestFiles`,
`latestFilesIndexes`, `allowModDistribution`, etc.), `File` (`id`, `modId`, `displayName`,
`fileName`, `releaseType`, `fileStatus`, `hashes`, `fileDate`, `fileLength`, `downloadUrl`,
`gameVersions`, `dependencies`, `fileFingerprint`, etc.), `FileHash` (`value`, `algo`),
`FileDependency` (`modId`, `relationType`), `Category` (`id`, `slug`, `isClass`, `classId`,
`parentCategoryId`), `Pagination` (`index`, `pageSize`, `resultCount`, `totalCount`), and
`FingerprintsMatchesResult` (`isCacheBuilt`, `exactMatches`, `partialMatches`,
`installedFingerprints`, `unmatchedFingerprints`). `gameId` 432, `classId` 6 for mods,
`modLoaderType` 4 for Fabric, `relationType` 3 for a required dependency, and hash `algo` 1
(sha1) / 2 (md5) come from `docs/research/2026-09-06-modrinth-and-curseforge-apis.md`.

Assumed (plausible but not doc-verified): every concrete value — mod/file IDs, names, slugs,
download counts, dates, hash strings, fingerprint numbers, category icon URLs, and the shape of
`sortableGameVersions` / `modules` entries. The class IDs 4471 (Modpacks), 12 (Resource Packs),
17 (Worlds), 6552 (Shaders), and 6945 (Data Packs) in `categories_classes.json` are copied from
the research doc's **VERIFY** table and are themselves unconfirmed against a live response.

The `api-verifier` agent must re-record every one of these fixtures against the real API with a
valid `CURSEFORGE_API_KEY` before any CurseForge client code that depends on them is considered
verified. Until then, treat exact field values (not just field names) as unverified.
