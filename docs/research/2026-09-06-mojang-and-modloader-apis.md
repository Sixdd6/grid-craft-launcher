# Mojang and modloader metadata APIs

Researched 2026-09-06. Source of truth for the `mojang-meta` and `modloaders` skills.
Agents must re-check a URL with the `api-verifier` agent before they change a parser.

## 1. Mojang piston-meta

### Version manifest

- `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json` (use this one; has `sha1` and `complianceLevel`)
- `https://piston-meta.mojang.com/mc/game/version_manifest.json` (v1, legacy)

```json
{
  "latest": { "release": "1.21.x", "snapshot": "..." },
  "versions": [
    {
      "id": "1.20.1",
      "type": "release|snapshot|old_beta|old_alpha",
      "url": "https://piston-meta.mojang.com/v1/packages/{sha1}/{id}.json",
      "time": "...", "releaseTime": "...",
      "sha1": "...", "complianceLevel": 1
    }
  ]
}
```

### Per-version JSON

Key sections:

- `downloads.client { sha1, size, url }` and `downloads.server`, `client_mappings`, `server_mappings`.
- `libraries[]`: `name` (maven coordinate), `downloads.artifact { path, sha1, size, url }`,
  optional `downloads.classifiers`, optional `rules[]`, optional `natives { linux, windows, osx }`,
  optional `extract.exclude[]`.
- `assetIndex { id, sha1, size, totalSize, url }`.
- `arguments.game[]` and `arguments.jvm[]`: each entry is a string or `{ rules[], value }` where
  `value` is a string or string array. Pre-1.13 versions use `minecraftArguments` (one string) instead.
- `mainClass`, `javaVersion { component, majorVersion }`, `logging.client`.
- Loader profiles add `inheritsFrom`; the launcher must merge the child over the parent.

### Rules

- `action` is `allow` or `disallow`.
- `os.name` is `windows`, `osx`, or `linux`. Optional `os.version` regex and `os.arch`.
- `features` object (for example `is_demo_user`, `has_custom_resolution`, `has_quick_plays_support`)
  only appears in argument rules. Unknown features evaluate false.
- A library or argument with no rules is always included. With rules, the default is
  disallow and the last matching rule wins.

### Assets

- Index URL is `assetIndex.url`. JSON is `{ "objects": { "<path>": { "hash", "size" } } }`.
- Object URL: `https://resources.download.minecraft.net/{hash[0..2]}/{hash}`. HTTPS only.
- Store at `<assets>/objects/{hash[0..2]}/{hash}` and the index at `<assets>/indexes/{id}.json`.
- Legacy indexes (`virtual: true` or `map_to_resources: true`) require copying objects to
  `<assets>/virtual/legacy/<path>` or `<game>/resources/<path>`.

### Natives

- Modern versions (1.19+) ship natives as normal libraries selected by `rules`; there is no
  `natives` map. Older versions use `natives[os]` to pick a classifier from
  `downloads.classifiers`, which the launcher extracts into a natives directory and passes as
  `-Djava.library.path`.
- Honor `extract.exclude` (usually `META-INF/`).

### Java runtimes

- `https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json`
  lists components (`java-runtime-gamma`, `java-runtime-delta`, ...) per platform key
  (`linux`, `linux-i386`, `windows-x64`, `mac-os`, `mac-os-arm64`, ...). Each entry has
  `manifest.url` pointing to a file list with `files { "<path>": { type, downloads.raw { sha1, size, url }, executable } }`.
- Fallback: Adoptium `https://api.adoptium.net/v3/binary/latest/{major}/ga/{os}/{arch}/jre/hotspot/normal/eclipse`.

### Rate limits and caching

- Mojang meta endpoints tolerate roughly 200 requests per 2 minutes per IP. Cache the manifest with
  its ETag, and cache every version JSON and asset index by sha1 forever.

## 2. Fabric meta

Base: `https://meta.fabricmc.net/v2`

- `GET /versions/game` → `[{ version, stable }]`
- `GET /versions/loader` → `[{ separator, build, maven, version, stable }]`
- `GET /versions/loader/{game}` → `[{ loader, intermediary, launcherMeta }]`
- `GET /versions/loader/{game}/{loader}/profile/json` → launcher version JSON with
  `inheritsFrom`, `mainClass`, `libraries[] { name, url, sha1?, size? }`, `arguments.jvm`.

Libraries here carry only `name` and `url` (maven root). Build the path from the maven coordinate:
`group/artifact/version/artifact-version[-classifier].ext`. Newer responses add `sha1` and `size`.

## 3. Quilt meta

Base: `https://meta.quiltmc.org/v3`

Same shape as Fabric: `/versions/game`, `/versions/loader`, `/versions/loader/{game}`,
`/versions/loader/{game}/{loader}/profile/json`. Quilt loader also requires the `hashed`
mappings artifact, which the profile JSON lists. URL-encode versions with spaces or hyphens.

## 4. Forge

- Maven: `https://maven.minecraftforge.net/`
- Promotions: `https://maven.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json`
  → `{ "promos": { "1.20.1-latest": "47.3.0", "1.20.1-recommended": "47.2.0" } }`
- Full list: `https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json`
  → `{ "1.20.1": ["1.20.1-47.2.0", ...] }`
- Installer: `https://maven.minecraftforge.net/net/minecraftforge/forge/{mc}-{forge}/forge-{mc}-{forge}-installer.jar`

### Installer JAR contents (1.13+)

- `install_profile.json`: `{ spec, profile, version, path, minecraft, json, data { KEY: { client, server } }, processors[], libraries[] }`
- `version.json` (path from `install_profile.json.json`): the launcher version JSON with `inheritsFrom`.
- `data/` entries: `client.lzma` and others referenced by `data` map values that start with `/`.
- `maven/` folder: artifacts that are not on any maven (for example the universal jar).

### Headless install algorithm

1. Download the installer jar, verify size, open it as a zip.
2. Read `install_profile.json`. If it has `versionInfo` instead of `spec`, it is a legacy
   (pre-1.13) profile: copy the universal jar, write the version JSON, done.
3. Download every library in `install_profile.libraries` and `version.json.libraries`
   into the shared libraries directory. Extract anything under `maven/` first.
4. Resolve `data` values for side `client`: values wrapped in `[...]` are maven coordinates,
   values wrapped in `'...'` are literals, values starting with `/` are files in the jar
   (extract to a temp dir). Add `MINECRAFT_JAR`, `SIDE`, `INSTALLER`, `ROOT`, `MINECRAFT_VERSION`,
   `LIBRARY_DIR`.
5. For each processor whose `sides` includes `client` or is absent: build the classpath from
   `jar` plus `classpath`, find the `Main-Class` from the processor jar manifest, substitute
   `{KEY}` tokens in `args`, and run `java -cp <cp> <main> <args>` with the selected Java.
   Before running, if every entry in `outputs` already exists with the expected sha1, skip it.
6. After all processors, check `outputs` hashes. Write `version.json` into the version cache.

Processors write the patched client jar to a path like
`net/minecraftforge/forge/{ver}/forge-{ver}-client.jar` inside the libraries directory.

## 5. NeoForge

- Maven: `https://maven.neoforged.net/releases`
- Version list: `https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge`
  → `{ isSnapshot, versions: ["20.4.190", "21.1.65", ...] }`
- Legacy 1.20.1 builds live under `net/neoforged/forge` with `1.20.1-47.1.x` names.
- Installer: `https://maven.neoforged.net/releases/net/neoforged/neoforge/{ver}/neoforge-{ver}-installer.jar`
- Installer layout and processor algorithm are identical to Forge 1.13+.
- NeoForge version to Minecraft version: `20.4.x` → 1.20.4, `21.1.x` → 1.21.1. The first
  two components map to the minor and patch of Minecraft 1.x.y.

## 6. Gotchas

- Classpath separator is `:` on Linux and macOS, `;` on Windows.
- Merge `inheritsFrom` chains before evaluating anything. Child libraries win on the same
  `group:artifact` when the child declares them; keep both when versions differ and the loader
  expects that (Forge does).
- Fabric and Quilt main class comes from the profile JSON, not the vanilla JSON.
- Always verify sha1 when the metadata provides one, and size when it does not.
- Run processors with the Java that will run the game; some processors need Java 17+.
- Forge installers before 1.12.2-14.23.5.2851 need a different flow; the MVP targets 1.13+
  for Forge and documents that limit.
- Mojang meta says nothing about which loader supports which version. Ask each loader's meta.

## Sources

- https://minecraft.wiki/w/Version_manifest.json
- https://minecraft.wiki/w/Client.json
- https://minecraft.wiki/w/Java_Edition_client_command_line_arguments
- https://github.com/FabricMC/fabric-meta
- https://github.com/QuiltMC/quilt-meta
- https://github.com/MinecraftForge/Installer
- https://docs.neoforged.net/user/docs/client/
- https://notes.highlysuspect.agency/neoforge-installer.html
- https://ryanccn.dev/posts/inside-minecraft-launcher/
- https://api.adoptium.net/
