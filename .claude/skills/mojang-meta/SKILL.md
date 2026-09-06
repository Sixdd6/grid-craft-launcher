---
name: mojang-meta
description: Mojang version manifest, version JSON, rules, arguments, assets, natives, and Java runtime manifest. Read before writing or changing anything in gcl-core mojang/ or java/, or any launch argument code.
---

Full detail with JSON shapes: `docs/research/2026-09-06-mojang-and-modloader-apis.md` section 1.

## Endpoints

- Manifest: `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json`. Cache with ETag. Entries have `id`, `type`, `url`, `sha1`, `releaseTime`.
- Version JSON: the entry's `url`. Cache forever under `cache/versions/<id>.json`; the URL contains its sha1.
- Assets: `assetIndex.url` → `{ objects: { path: { hash, size } } }`. Object URL `https://resources.download.minecraft.net/<hash[0..2]>/<hash>`. Store under `cache/assets/objects/<hash[0..2]>/<hash>`. Index under `cache/assets/indexes/<id>.json`.
- Java runtimes: `https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json`, keyed by platform (`linux`, `windows-x64`, `mac-os`, `mac-os-arm64`) then component (`java-runtime-gamma`, `java-runtime-delta`). Each `manifest.url` lists files with `downloads.raw { sha1, size, url }` and `executable`. Take the component from the version JSON's `javaVersion.component` (`InstallPlan::java_component`, used by `Launcher::ensure_java_for`); `component_for_major` is only the fallback for a version JSON that names none.

## Version JSON fields you must handle

- `downloads.client { sha1, size, url }`
- `libraries[] { name, downloads.artifact { path, sha1, size, url }, downloads.classifiers?, rules?, natives?, extract.exclude? }`
- `arguments.game[]`, `arguments.jvm[]`: string or `{ rules, value }` with `value` string or array. Old versions: `minecraftArguments` single string, and no JVM args (use the standard `-Djava.library.path`, `-cp` set).
- `mainClass`, `assetIndex`, `assets`, `javaVersion.majorVersion`, `inheritsFrom`, `logging.client`.

## Rules

- No rules: include. With rules: start disallowed; walk rules in order; a matching `allow` sets allowed, a matching `disallow` sets disallowed.
- `os.name`: `linux`, `windows`, `osx`. `os.arch`: `x86`, `x86_64`, `arm64`. `os.version`: regex against the OS version string.
- `features`: only in argument rules. Known: `is_demo_user`, `has_custom_resolution`, `has_quick_plays_support`, `is_quick_play_singleplayer`, `is_quick_play_multiplayer`, `is_quick_play_realms`. Unknown features are false.

## inheritsFrom

Loader profiles set `inheritsFrom`. Resolve parent first. Child `mainClass` wins. Child
`arguments` append to parent. Libraries are merged by `group:artifact:classifier`
(`MavenCoord::merge_key`), so a natives classifier jar and its plain jar merge independently,
with the child's version winning per key. The one exception: Forge and NeoForge expect both
versions on the classpath, so they keep both there.

The caller passes that choice, it is not sniffed from the profile: `merge(parent, child,
keep_both_libraries)` and `Mojang::resolve(v, keep_both_libraries)` take the flag, and the
Forge and NeoForge loader callers pass `true`. `Mojang::resolve_auto(v)` guesses the flag from
the child id containing `forge` and is what the vanilla install path uses. See
`docs/research/2026-09-06-mojang-and-modloader-apis.md` section 6.

## Natives

- Modern versions: natives are ordinary libraries picked by rules. Nothing to extract.
- Old versions (`natives` map present): pick `downloads.classifiers[natives[os]]`, extract into `cache/natives/<version>/` minus `extract.exclude`, pass `-Djava.library.path`.
- A library with a `natives` entry for this OS and no `downloads.artifact` is natives-only:
  download only the classifier jar, do not put a base jar on the classpath (1.8.9
  `jinput-platform`, `twitch-platform`).

## Placeholders

`${auth_player_name} ${auth_uuid} ${auth_access_token} ${user_type} ${clientid} ${auth_xuid}
${version_name} ${version_type} ${game_directory} ${assets_root} ${assets_index_name}
${natives_directory} ${launcher_name} ${launcher_version} ${classpath} ${library_directory}
${classpath_separator} ${resolution_width} ${resolution_height}`. Unknown placeholders stay as-is and log a warning.

## Do not

- Do not hardcode a version's library list. Always read the JSON.
- Do not trust size alone when sha1 is present.
- Do not fetch the manifest more than once per launcher run.
