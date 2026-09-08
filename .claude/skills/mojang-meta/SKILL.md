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

`logging.client` names a log4j2 XML config: `{ argument, file: { id, sha1, size, url } }`.
`InstallPlan.log_config` downloads it to `cache/assets/log_configs/<id>` and records that path;
`launch::build` inserts `argument` (with `${path}` replaced by that path) between the expanded
JVM args and the main class, only when `log_config` is `Some`. A version with no logging block
gets no `-Dlog4j...` argument at all — older Minecraft did not ship one.

## Rules

- No rules: include. With rules: start disallowed; walk rules in order; a matching `allow` sets allowed, a matching `disallow` sets disallowed.
- `os.name`: `linux`, `windows`, `osx`. `os.arch`: `x86`, `x86_64`, `arm64`. `os.version`: regex against the OS version string.
- `features`: only in argument rules. Known: `is_demo_user`, `has_custom_resolution`, `has_quick_plays_support`, `is_quick_play_singleplayer`, `is_quick_play_multiplayer`, `is_quick_play_realms`. Unknown features are false.

## inheritsFrom

Loader profiles set `inheritsFrom`. Resolve parent first. Child `mainClass` wins. Child
`arguments` append to parent. Libraries are merged by `group:artifact:classifier`
(`MavenCoord::merge_key`), so a natives classifier jar and its plain jar merge independently,
with the child's version winning per key. The one exception: Forge and NeoForge expect both
versions on the classpath, so they keep both there. Under `keep_both` a library the child
repeats at the *same* version is kept once, and the copy kept is the parent's: vanilla
publishes `downloads.artifact` with Mojang's URL and sha1, while a loader profile usually
names only a maven root, so taking the child would lose the checksum the download layer
verifies against. A classpath naming one jar twice fails BootstrapLauncher's duplicate check.

The caller passes that choice, it is not sniffed from the profile: `merge(parent, child,
keep_both_libraries)` and `Mojang::resolve(v, keep_both_libraries)` take the flag, and the
Forge and NeoForge loader callers pass `true`. `Mojang::resolve_auto(v)` guesses the flag from
the child id containing `forge` and is what the vanilla install path uses. See
`docs/research/2026-09-06-mojang-and-modloader-apis.md` section 6.

`logging` merges by a separate rule from every other field: a Forge or NeoForge profile publishes
an empty `"logging": {}` block, which parses to a config with no `client`. Taking the child
whole there would drop vanilla's log4j config and leave the game with no
`-Dlog4j.configurationFile`. So the child's `logging` wins only when it names a `client`;
otherwise the parent's `logging` is kept, even though the child is otherwise present
(`merge_logging` in `mojang/mod.rs`).

## Natives

- Modern versions: natives are ordinary libraries picked by rules. Nothing to extract.
- Old versions (`natives` map present): pick `downloads.classifiers[natives[os]]`, extract into `cache/natives/<version>/` minus `extract.exclude`, pass `-Djava.library.path`.
- A library with a `natives` entry for this OS and no `downloads.artifact` is natives-only:
  download only the classifier jar, do not put a base jar on the classpath (1.8.9
  `jinput-platform`, `twitch-platform`).

## Garbage collector probe

`java::gc::probe(java)` runs `<java> -XX:+PrintFlagsFinal -version` once, with a 20 s timeout,
and returns `GcSupport { major, version, flags }`. The `-version` banner goes to stderr and the
flag dump to stdout, so the parser reads both streams as one text.

- Major and version come from the banner line's quoted string (`openjdk version "21.0.7"`),
  through `detect::parse_java_version`.
- A dump line is whitespace columns: C++ type, flag name, `=`, value, origin tags. Names are
  right-aligned to the longest name in the file, so read tokens, never fixed columns.
- Only lines whose first token is `bool` count, and only the names in
  `java::gc::supported_flag_names()` are kept: `UseSerialGC`, `UseParallelGC`, `UseG1GC`,
  `UseZGC`, `ZGenerational`, `UseShenandoahGC`. `GcSupport::supports(flag)` asks about one.
- A flag name being present means that build compiled the collector in. The value only says
  which collector is active by default, so never read it.
- Recorded dumps live in `tests/fixtures/java/printflags-{17,21,25}.txt`, trimmed to the banner
  and the lines mentioning `GC` or `ZGenerational`. They come from Microsoft OpenJDK 17.0.15 and
  21.0.7 (Mojang's `java-runtime-gamma` and `java-runtime-delta`) and Red Hat OpenJDK 25.0.4.1.
  17 has no `ZGenerational`; 21 has it; 25 dropped it, because ZGC is generational there.

**The picker probes the Java the launch runs.** `Launcher::gc_support` and `prepare_launch`
resolve it through one helper, `Launcher::launch_java`: the instance's own `java_path`, then
`config.toml`'s, then `ensure_java_for` on the plan of the version this instance launches.
That plan is the loader profile merged over vanilla, not vanilla alone, because
`javaVersion` merges child-wins like every other field — a Fabric or NeoForge profile naming
its own major would otherwise send the picker to a different JVM than the launch.
`Launcher::launch_plan` builds it without downloading a game file: it reads the profile from
`cache/versions/<id>.json` when `loader_version` names a build already installed, and
installs the loader first when it does not.

`java::gc::ProbeCache::get_or_probe(root, java)` answers from an in-memory map, then from
`cache/runtimes/gc-probe.json`, then by running the JVM. Both are keyed by the binary's
canonical path and its modification time in nanoseconds, so a runtime replaced in place is
probed again rather than answered from a stale entry. A missing or unparsable cache file is
treated as empty. The disk file is rewritten through `paths::write_atomic` and has no eviction.

## Placeholders

`${auth_player_name} ${auth_uuid} ${auth_access_token} ${user_type} ${clientid} ${auth_xuid}
${version_name} ${version_type} ${game_directory} ${assets_root} ${assets_index_name}
${natives_directory} ${launcher_name} ${launcher_version} ${classpath} ${library_directory}
${classpath_separator} ${resolution_width} ${resolution_height}`. Unknown placeholders stay as-is and log a warning.

## Do not

- Do not hardcode a version's library list. Always read the JSON.
- Do not trust size alone when sha1 is present.
- Do not fetch the manifest more than once per launcher run.
