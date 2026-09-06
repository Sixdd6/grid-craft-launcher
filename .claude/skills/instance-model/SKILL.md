---
name: instance-model
description: App root layout, instance.toml schema, installed content list, options.txt preseed and keyed overrides, and JVM memory fields. Read before touching gcl-core paths/, config/, instances/, or settings/.
---

## Root

`GCL_ROOT` env → `config.toml` `root` override → platform data dir plus `grid-craft-launcher`
(`directories::ProjectDirs::from("", "", "grid-craft-launcher").data_dir()`).

```
<root>/config.toml
<root>/accounts.json
<root>/instances/<slug>/instance.toml
<root>/instances/<slug>/.minecraft/
<root>/cache/{versions,libraries,assets,natives,runtimes,installers,objects}/
<root>/logs/
```

Slug: lowercase, `[a-z0-9-]`, from the name; append `-2`, `-3` on collision.

## config.toml

```toml
root = ""                       # optional override
[jvm]
min_mib = 1024
max_mib = 4096
java_path = ""                  # optional
[keys]
curseforge_api_key = ""         # env wins
msa_client_id = ""              # env wins
[game_defaults]                 # options.txt preseed, key = value strings
"renderDistance" = "12"
"guiScale" = "2"
```

## instance.toml

```toml
name = "Fabric 1.20.1"
minecraft = "1.20.1"
loader = "fabric"               # none | fabric | quilt | forge | neoforge
loader_version = "0.15.11"
created = "2026-09-06T12:00:00Z"
last_launched = ""

[jvm]
min_mib = 2048                  # optional, else global
max_mib = 6144
java_path = ""
extra_args = ["-XX:+UseG1GC"]

[settings_overrides]            # options.txt keys written on every launch
"renderDistance" = "16"

[[content]]
source = "modrinth"             # modrinth | curseforge | file
project_id = "AANobbMI"
version_id = "abc123"
file_name = "sodium-fabric-0.5.8.jar"
sha1 = "..."
kind = "mod"                    # mod | resourcepack | shader | datapack | world
enabled = true
```

## options.txt semantics

- Format: one `key:value` per line. Note the separator is a colon, not `=`.
- Preseed: on instance creation, write `[game_defaults]` as `key:value` lines when `options.txt` does not exist.
- Override: on every launch, for each key in `settings_overrides`, replace the line starting with `key:` or append `key:value` if absent. Leave every other line untouched. Preserve order.
- Values are stored as strings exactly as Minecraft writes them (`true`, `12`, `"en_us"` with quotes for strings).

## Content install

`instances::install(instance, kind, cached_object, file_name)` hard-links from
`cache/objects/` into the target folder, falling back to copy across filesystems. Disable
renames to `<file>.disabled`.

## Writing files

`paths::write_atomic(path, bytes)` is the way to write any file under the root: it writes to a
temp file and renames over the destination, so a crash mid-write never leaves a truncated
`config.toml` or `instance.toml`. Use it instead of `std::fs::write` for anything under `<root>/`.

## Listing instances

`Instances::list` skips a directory that fails to parse as an instance (missing or malformed
`instance.toml`, non-UTF-8 slug) and logs a `tracing::warn!` instead of failing the whole list.
One broken instance does not hide the rest.

## Do not

- Do not write `options.txt` with `=`.
- Do not delete a user's `.minecraft/` on instance update; only on explicit delete.
- Do not store absolute paths in `instance.toml`; everything is relative to the instance.
