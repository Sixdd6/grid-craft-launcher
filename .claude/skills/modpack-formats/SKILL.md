---
name: modpack-formats
description: Modrinth .mrpack and CurseForge modpack zip formats and the import algorithm into a new instance. Read before touching gcl-core modpacks/.
---

Full detail: `docs/research/2026-09-06-modrinth-and-curseforge-apis.md` sections 1 and 2.

## Detect

- Zip with `modrinth.index.json` at root → mrpack.
- Zip with `manifest.json` where `manifestType == "minecraftModpack"` → CurseForge.
- Anything else → `Error::UnknownPackFormat`.

## mrpack

- `dependencies`: `minecraft` plus one of `fabric-loader`, `quilt-loader`, `forge`, `neoforge`.
- `files[]`: `path`, `hashes { sha1, sha512 }`, `env.client` (`required`, `optional`, `unsupported`), `downloads[]`, `fileSize`.
- Skip `env.client == unsupported`. Install `optional` in the MVP.
- Download hosts must be one of `cdn.modrinth.com`, `github.com`, `raw.githubusercontent.com`, `gitlab.com`; refuse others.
- Copy `overrides/` then `client-overrides/` into `.minecraft/`. Later wins.
- `path` is relative to `.minecraft/`; refuse paths containing `..`.

## CurseForge

- `minecraft.version` and `minecraft.modLoaders[]` with `id` `<loader>-<version>`; use the `primary` one.
- `files[] { projectID, fileID, required }`. Resolve in batches of 50 with `POST /v1/mods/files`. Place by each file's class id. Null download URL follows the mod-sources rule.
- Copy the folder named by `overrides` (default `overrides`) into `.minecraft/`.

## Import algorithm

1. Parse and detect.
2. Create the instance (name from the pack, slug deduplicated).
3. Install Minecraft version and loader through `loaders::install`.
4. Download all files through the download cache in parallel, then hard-link or copy into the instance.
5. Apply overrides.
6. Record every file in the instance content list with its source ids and sha1.
7. Emit progress events per file and per phase.

On failure after step 2, delete the partial instance unless `keep_partial` is set.
