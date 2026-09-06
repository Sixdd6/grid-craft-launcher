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
last_launched = "2026-09-06T13:00:00Z"   # optional, absent until the first launch

[pack]                          # optional, only when the instance came from a modpack
source = "modrinth"             # modrinth | curseforge | file
project_id = "AANobbMI"
version_id = "abc123"

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
sha1 = "..."                    # optional, when the source publishes one
fingerprint = 1234567890        # optional, CurseForge murmur2
kind = "mod"                    # mod | resourcepack | shader | datapack | world
world = "New World"             # optional, the target world for a datapack
enabled = true
```

## options.txt semantics

The `settings` module (`gcl-core/src/settings/mod.rs`) owns `options.txt`. It takes a game
directory and a `BTreeMap<String, String>`, never an `Instance`, so `instances::create` and
`Launcher::apply_settings_overrides` both call it without `settings` depending on `instances`.

- Format: one `key:value` per line. Note the separator is a colon, not `=`. A value may itself
  hold a `:` (only the first one on the line separates the key); a key may not.
- `apply_preseed(game_dir, defaults)`: writes `defaults` as `key:value` lines, but only when
  `options.txt` does not already exist. An existing file, even an empty one, is left untouched.
- `apply_overrides_to(game_dir, overrides)`: for each key, replaces the line starting with `key:`
  or appends `key:value` if absent. Every other line is left untouched, in its original order.
  Returns the number of keys that actually changed, so a caller can skip logging or rewriting
  when nothing changed; a second call with the same overrides writes nothing (checked by mtime
  in the test suite).
- `validate_key(key)` / `validate_value(value)`: a key must be non-empty, hold no `:`, `\r`, or
  `\n`, and have no leading or trailing whitespace; a value may hold `:` but not `\r` or `\n`.
  `apply_overrides_to` validates every entry before writing anything, so one bad override key in
  a hand-edited `instance.toml` fails the whole call rather than partially corrupting the file.
- CRLF is preserved: if the file's lines end in `\r\n`, every line `settings` writes back keeps
  `\r\n`, so a Windows-style `options.txt` round-trips byte for byte.
- Values are stored as strings exactly as Minecraft writes them (`true`, `12`, `"en_us"` with quotes for strings).

## accounts.json

```json
{ "accounts": [{ "id": "...", "name": "...", "kind": "offline" }], "active": "..." }
```

`kind` is `"offline"` or `"msa"`. See the `msa-auth` skill for the full `Account` shape and the
offline UUID algorithm.

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
