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
source = "modrinth"             # modrinth | curseforge | file (a `.mrpack` file, not updatable)
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

## The settings catalog

`settings::catalog` is the table of keys the launcher understands: `CATALOG: &[Setting]`, one
`Setting { key, label, group, control, default }` per key, in the order a screen shows them.
`Group` is `Video`, `Controls`, `Sound`, `Chat`, or `Other`. `Control` is `Slider { min, max,
step, decimals }`, `Toggle`, `Choice(&[(token, label)])`, or `Text`. It is data, not code —
adding a key is one row, and `docs/research/2026-09-07-options-txt-catalog.md` is where the
ranges and tokens come from.

`catalog::find(key)` looks a key up. `format_value` and `parse_value` round-trip the stored
form Minecraft writes: `true`/`false` for a toggle, a float with its own `decimals` for a
slider, a bare token for a choice, the string verbatim for text. `parse_value` rejects an
out-of-range number, a non-finite one, and a token no choice holds.

**A key the catalog does not know is never lost.** It has no control and no validation beyond
`validate_key`/`validate_value`, and it survives every read and write.

## Layers

`settings::doc::merged(preseed, overrides, current)` builds the row list a settings
screen renders: one `Row { key, value, source, setting }` per catalog key in catalog order,
then every unknown key from any layer, alphabetically. `source` names the layer that won:

**Override > File > Preseed > Default.**

The file outranks the preseed because `apply_preseed` writes only when `options.txt` is absent.
Once the game has written the file, a line in it is what the game reads, so showing the preseed
value there would be a lie. An instance override still outranks the file, because launch writes
it back into the file every time.

`settings::doc::validate(setting, value)` checks one value against its catalog entry.

## Validation reaches both front ends

`Launcher::set_instance_override` / `unset_instance_override` edit one instance's override map;
`set_game_default` / `unset_game_default` do the same for the preseed in `config.toml`. All four
validate the key and value, and range- and choice-check a known catalog key. `gcl settings set`
and `gcl settings defaults set` route through those methods rather than writing the map
themselves, so the CLI rejects exactly what the GUI rejects — an out-of-range slider value or a
bad choice token fails at the command line too.

`Launcher::settings_rows_for_defaults()` and `settings_rows_for_instance(slug)` return the
merged rows for the two layers a screen can edit.

## accounts.json

```json
{ "accounts": [{ "id": "...", "name": "...", "kind": "offline" }], "active": "..." }
```

`kind` is `"offline"` or `"msa"`. See the `msa-auth` skill for the full `Account` shape and the
offline UUID algorithm.

## Content install (`instances::content`)

`target_dir(game_dir, kind, world)` maps a `ContentKind` to its folder: `Mod` → `mods/`,
`ResourcePack` → `resourcepacks/`, `Shader` → `shaderpacks/`, `World` → `saves/`, `DataPack` →
`saves/<world>/datapacks/` (`world` is required; `Error::WorldRequired` without one, and it goes
through `safe_join` so a world name cannot escape `saves/`).

- `place_file(instance, object, entry)` hard-links `object` (a path under `cache/objects/`, or
  copies across filesystems) into its target folder, replaces an already-installed entry from
  the same `(source, project_id)` in place — deleting its old file and any `.disabled` twin
  first — and always leaves the new file enabled. It records the `ContentEntry` in
  `instance.toml` and saves.
- `place_world(instance, zip_path, entry)` extracts a world zip under `saves/`. The archive must
  hold exactly one top-level folder — anything else is `Error::BadWorldZip` — and that folder
  name becomes `entry.world`. An existing `saves/<folder>` is never overwritten:
  `Error::WorldExists(folder)` instead. Every archive entry goes through `safe_join`.
- `set_enabled(instance, project_id, enabled)` renames the file to (or from) `<file>.disabled`
  and returns its new path; an entry already in the wanted state is a no-op that still returns
  the current path. A missing file is `Error::ContentFileMissing`.
- `remove(instance, project_id)` deletes both the enabled and disabled form of the file (a
  missing file is not an error) and drops the entry from `instance.toml`.
- `installed(instance, source, project_id)` finds the entry from `(source, project_id)`, if any.
- `file_path(instance, entry)` returns where an entry's file (or world folder) lives right now,
  including the `.disabled` suffix when it is disabled.

`content::add` (see the `mod-sources` skill for the source side) is what calls `place_file` /
`place_world` after resolving a project and downloading its file; nothing in `instances`
depends on `sources` or `content` beyond the `SourceId` type.

## `pack` and pending manual downloads

`instance.config.pack: Option<PackSource>` is set only when the instance came from a modpack
import (`{ source, project_id, version_id }`, `source` one of `modrinth`, `curseforge`, or
`file` for a local archive with no known origin). A vanilla or hand-built instance has none.

`instances/<slug>/pending-manual.json` (`Launcher::PENDING_MANUAL_FILE`) is not part of
`instance.toml`: it is a flat JSON array of pending hand downloads (source, project id, version
id, file name, page URL, expected fingerprint or sha1, and the target world for a data pack),
appended to by `content::add` and modpack import whenever a file has no download URL, and
pruned by `import_manual_file`. A missing file reads as an empty list — an instance that never
hit a manual download has none. `ManualDownload::world` is what `content::import_manual` puts
in the entry, so a hand-fetched data pack lands in the world the original request named;
`gcl content import-file --world <name>` overrides it.

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
