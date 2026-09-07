---
name: modpack-formats
description: Modrinth .mrpack and CurseForge modpack zip formats and the import algorithm into a new instance. Read before touching gcl-core modpacks/.
---

Full research: `docs/research/2026-09-06-modrinth-and-curseforge-apis.md` sections 1 and 2.
Types below are the ones `gcl-core::modpacks` actually builds.

## Detect (`modpacks::detect`)

- A `modrinth.index.json` at the archive root → `PackFormat::Mrpack`.
- A root `manifest.json` whose `manifestType` is `"minecraftModpack"` → `PackFormat::CurseForge`.
- Anything else, including a manifest nested in a subfolder → `Error::UnknownFormat`.

A manifest read out of the zip is capped at 8 MiB (`MAX_MANIFEST_BYTES`): a zip entry's declared
size is attacker-controlled, so this is a real read cap, not a trust of the header.

## PackPlan

`modpacks::read_plan(zip, extra_hosts)` returns `(PackFormat, PackPlan)`:

```rust
pub struct PackPlan {
    pub name: String,
    pub version: String,
    pub minecraft: String,
    pub loader: Loader,
    pub loader_version: String,
    pub files: Vec<PackFile>,
    pub overrides: Vec<String>,   // prefixes to copy, in apply order; later wins
}

pub struct PackFile {
    pub path: Option<String>,                          // .mrpack only
    pub url: Option<String>,                            // .mrpack only
    pub sha1: Option<String>,                            // .mrpack only
    pub size: Option<u64>,                               // .mrpack only
    pub source: Option<(SourceId, String, String)>,      // CurseForge only: (source, project_id, file_id)
    pub required: bool,
}
```

`extra_hosts` (also `modpacks::ImportRequest::extra_hosts`) adds to the `.mrpack` host
allowlist for one call; tests use it to point at a wiremock server, production code always
passes an empty slice.

## mrpack (`modpacks::mrpack::parse`)

- `dependencies` must name `minecraft` plus exactly one of `fabric-loader`, `quilt-loader`,
  `forge`, `neoforge`. Zero or two or more loader keys is `Error::Parse` — a pack with no loader
  is not something this launcher installs.
- `files[]`: `path`, `hashes.sha1` (required — a file with none is `Error::Parse`), `env.client`
  (`required` | `optional` | `unsupported`), `downloads[]` (first URL wins), `fileSize`.
- A file whose `env.client` is `"unsupported"` is dropped before the host or path checks run.
  An `"optional"` file is kept and installed — the MVP does not ask.
- Every kept file's path goes through `safe_join` before anything else, so a `..` or absolute
  path is `Error::UnsafePath` before an instance exists.
- The download host must be on `mrpack::ALLOWED_HOSTS` — `cdn.modrinth.com`, `github.com`,
  `raw.githubusercontent.com`, `gitlab.com` — or in `extra_hosts`; anything else is
  `Error::DisallowedHost`.
- `overrides: ["overrides/", "client-overrides/"]`, always in that order — `client-overrides/`
  is applied second and so overwrites what `overrides/` wrote.

## CurseForge (`modpacks::curseforge::parse`)

- `minecraft.version` and `minecraft.modLoaders[]`, each `{ id: "<loader>-<version>", primary }`.
  The loader used is the one marked `primary`, or the first when none is marked;
  `split_loader_id` splits `<loader>-<version>` on the first `-`, so `forge-47.2.0` becomes
  `(Loader::Forge, "47.2.0")`.
- `files[] { projectID, fileID, required }` (both keys capitalized, unlike the rest of the
  format) become `PackFile { source: Some((SourceId::CurseForge, project_id, file_id)), path:
  None, url: None, sha1: None, size: None, required }`. Nothing here calls the API; resolving
  ids to real files happens during install, not during parse.
- `overrides` names the folder to copy (default `"overrides"` when the manifest leaves it
  blank); `PackPlan::overrides` is `vec!["<name>/"]`, one entry.

## Import algorithm (`modpacks::import` / `import_plan`)

1. `read_plan` detects the format and parses the manifest (`import` does this itself, on
   `spawn_blocking`; `import_plan` takes an already-read plan, for a caller — `Launcher` — that
   needs the manifest's Minecraft version before it can build a JVM for a Forge/NeoForge
   installer).
2. `Instances::create` builds the new instance (name from `req.name` or the pack's own name,
   slug deduplicated) before anything downloads.
3. `instance.config.pack` is set and saved (`PackSource { source, project_id, version_id }`,
   `None` for a local file import) and `loaders::install` runs if the pack names a loader.
4. Files install:
   - `.mrpack`: each `PackFile.path` is `safe_join`ed under the game directory;
     `content::fetch_object` downloads (or reuses) the object by its published sha1, then
     `link_or_copy` links it into place. A file under `mods/`, `resourcepacks/`, or
     `shaderpacks/` gets a `ContentEntry` too. One batched Modrinth hash lookup
     (`Source::resolve_by_hash`, `POST /version_files`) runs first over every such file's
     sha1, so the entry carries the real `(project_id, version_id)` and `content::add` can
     see the pack's own copy of a mod. The lookup is best effort: no Modrinth source, a
     failed request, or a hash Modrinth does not know falls back to `source: "file"` with
     the sha1 standing in for both ids, and emits one `Event::Warning` naming the file.
     `SourceId::parse` does not accept `"file"`, so `content::check_updates` skips such an
     entry silently: there is no project to ask about. `instances::model::FILE_SOURCE` holds
     that string; CurseForge pack files are unchanged, since they carry ids already.
     Anything else (a config file, a script) is placed and left off the content list, like an
     override.
   - CurseForge: `files_batch` resolves every `(project_id, file_id)` to a `Version`,
     `mods_batch` resolves the distinct project ids to their class, and each file is placed
     under the folder its project's class maps to (a project the batch dropped is treated as a
     `Mod`). A file whose project's class is a data pack is skipped with an `Event::Warning`
     naming the file: a data pack needs `saves/<world>/datapacks/`, and a pack manifest names
     no world. A file with no `downloadUrl` becomes a `ManualDownload`, the same as in
     `content::add`, and does not fail the import. CurseForge silently drops a file id it does
     not recognize (a deleted file); the import logs which ids that was, since nothing else
     would ever say so.
5. Override folders are copied over the game directory last, in `PackPlan::overrides` order,
   every entry path-checked with `safe_join`.
6. On any failure after step 2, the half-built instance is deleted unless
   `ImportRequest::keep_partial` is set; the original error is what the caller sees either way.

`modpacks::fetch_pack(ctx, source, project, version)` downloads a pack's own archive — a
project id or slug, an optional version id or number (else the newest release) — through each
client's pack-only endpoints (`Modrinth::pack_versions`, `CurseForge::resolve_pack_id` +
`pack_files`), since a modpack is not a `ContentKind` and so has no `Source::project`. It
returns the archive's path plus the `PackSource` to record.
