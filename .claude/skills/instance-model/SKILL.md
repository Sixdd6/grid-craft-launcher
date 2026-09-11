---
name: instance-model
description: App root layout, instance.toml schema, installed content list, options.txt preseed and keyed overrides, and JVM memory fields. Read before touching gcl-core paths/, config/, instances/, or settings/.
---

## Root

`GCL_ROOT` env → `config.toml` `root` override → platform data dir plus `grid-craft-launcher`
(`directories::ProjectDirs::from("", "", "grid-craft-launcher").data_dir()`).

**The `root` pointer is read and written at the un-redirected root.** `Launcher::open` reads
`config.toml` at the resolved root (the platform data dir, unless `GCL_ROOT` or a caller
override decided it), applies its `root` key, and then loads the redirected root's own
`config.toml` when it has one. `Launcher` keeps the un-redirected path as `base_root`, and
`update_config` saves the config at the current root and then writes the `root` key alone back
into `base_root`'s `config.toml`. Without that write a second root change is lost: it would
land only at the first redirected root, which the next start never reads. A launcher whose
root was not redirected writes one file, since both paths are the same. `open` also rewrites
the in-memory `config.root` to the root it actually runs at, so a stale pointer left in a
target's own `config.toml` is never written back as the current choice.

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
gc = "default"                  # seeded into every new instance
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
gc = "zgc"                      # optional, absent means "default"

[settings_overrides]            # options.txt keys written on every launch
"renderDistance" = "16"

[[content]]
source = "modrinth"             # modrinth | curseforge | file (a `.mrpack` file, not updatable)
project_id = "AANobbMI"
version_id = "abc123"
file_name = "sodium-fabric-0.5.8.jar"
title = "Sodium"                # optional, the project title for display
sha1 = "..."                    # optional, when the source publishes one
fingerprint = 1234567890        # optional, CurseForge murmur2
kind = "mod"                    # mod | resourcepack | shader | datapack | world
world = "New World"             # optional, the target world for a datapack
enabled = true
```

## Garbage collector preset

`jvm.gc` is a `GcPreset` in `instances::model`: `default`, `serial`, `parallel`, `g1`, `zgc`,
`zgc_generational`, `shenandoah`. `default` is the value of a missing key and writes no key
back, so an older `instance.toml` round-trips unchanged. `GcPreset::flags(major)` gives the
JVM flags: none for `default`, `-XX:+UseSerialGC`, `-XX:+UseParallelGC`, `-XX:+UseG1GC`,
`-XX:+UseShenandoahGC`, and for ZGC `-XX:+UseZGC` plus `-XX:-ZGenerational` (`zgc`) or
`-XX:+ZGenerational` (`zgc_generational`) on Java 21 and 22 only; 23 and later pass
`-XX:+UseZGC` alone. `GcPreset::all`, `label`, and `description` feed the CLI and the JVM tab.
`config.jvm.gc` seeds a new instance at creation only, through
`Instances::with_gc_default`, which `Launcher::instances()` sets from the config. Changing
`config.jvm.gc` later leaves every existing instance as it was.

**An unknown token is the launcher default, not an error.** `GcPreset`'s `Deserialize` is
hand-written: a token this build does not know loads as `Default` and logs a `tracing::warn!`.
Failing there would fail the whole `instance.toml`, and `Instances::list` skips an instance
that will not parse, so one word written by a newer build would hide a whole pack.

`FromStr` is lenient the same way, for a token a person types: it trims, lowercases, and reads
`-` and a space as `_`, so `ZGC-Generational` is `zgc_generational`. Aliases: `g1gc` → `g1`,
`generational_zgc` and `zgcgenerational` → `zgc_generational`, `none` and `launcher` →
`default`. `UnknownGcPreset`'s message lists every valid token.

**A saved `zgc` folds onto `zgc_generational` from 23 on.** `GcPreset::for_major(major)`
returns `ZgcGenerational` for `Zgc` when `major >= 23` and every other preset unchanged.
`Launcher`'s `check_preset` (used by `set_instance_gc` and `prepare_launch`) folds before it
checks, so an instance saved as `zgc` before its runtime moved to 23 still launches; the flags
are the same either way. `set_instance_gc` saves the folded preset, and `GcSupportView.saved`
carries it, so a picker's selection is always one of `GcSupportView.presets`.

`saved` is what both pickers read. `gcl instance gc` prints it as `selected:` and the JVM tab
selects the row whose token matches it, so a `zgc` written before the runtime moved to 23
shows as the generational row that Java really offers instead of being called unavailable.
Neither reads `instance.toml`'s raw token for that.

**One entry per set of flags.** `supported_presets(major, flags)` reads the probe's flag names
and drops `Zgc` from 23 on, because there both ZGC presets expand to `-XX:+UseZGC` and a picker
would show two rows that do the same thing. `ZgcGenerational`'s label reads "ZGC (generational)"
so the one row that stays says what it is. Below 21 the generational mode does not exist:
`supported_presets` never offers it and `Launcher::set_instance_gc` and `prepare_launch` refuse
it, so `flags()`'s plain-ZGC answer for that pair is unreachable past validation.

**`launch::build` refuses a preset with no probed major.** Every preset's flags depend on the
java major, so `gc != Default` with `gc_major < 8` is `launch::Error::MissingGcMajor`: it means
the caller never probed the binary. `Launcher::prepare_launch` probes whenever the preset is
not `Default` and passes the probed major.

**A conflicting extra argument is found by flag name.** `launch::gc_conflict` strips `-XX:+` or
`-XX:-` and compares the rest against `UseSerialGC`, `UseParallelGC`, `UseParallelOldGC`,
`UseConcMarkSweepGC`, `UseG1GC`, `UseZGC`, `UseShenandoahGC`, `UseEpsilonGC`, and
`ZGenerational`. The disable form counts: `-XX:-UseG1GC` under a preset is
`launch::Error::GcConflict` too. With `GcPreset::Default` there is no conflict, since a
hand-written collector flag is what extra arguments are for.

**`Launcher::set_instance_jvm_gc` writes the block and its preset once.** It resolves the
java the block it is about to save will launch with — `jvm.java_path` when set, else
`config.toml`'s, else the runtime for the version this instance launches — probes that one,
folds and checks the preset against it, and only then saves. So `gcl instance jvm x
--java-path <new> --gc <preset>` is judged by the new binary, and a refused preset writes
nothing at all, heap fields included: there is no half-saved state and nothing to roll back.
`set_instance_gc` is that call with every other field left as it was. `gcl instance jvm` also
takes `--clear-extra` and `--clear-java-path`, because passing no `--extra` keeps the current
list rather than emptying it.

**`Launcher::set_instance_jvm` keeps the saved preset.** It replaces the heap fields, the java
path, and the extra arguments only, whatever `jvm.gc` holds. A JVM tab that sends the heap back
with a default `gc` would otherwise clear a preset it never asked about. Only
`set_instance_gc` changes the preset, because only it probes the JVM first.

**The probe cache never guesses a key.** `java::gc::ProbeCache` keys an answer by the binary's
canonical path and modification time. A binary whose metadata cannot be read has no key, so it
is probed every time rather than cached under a made-up one. Two callers asking for the same
key at once share one run: an in-flight gate per key means java is spawned once. A JVM that
exits non-zero is `java::Error::Probe` carrying the exit code and the first 200 characters of
its stderr.

`title` is absent in an `instance.toml` written before the key existed, and in one a modpack
import wrote: a pack index names no project title. `content::check_updates` backfills it, at
most 25 per call (`content::MAX_TITLE_BACKFILL`), each one a project request at the source;
the rest are filled by later calls, and hitting the cap logs at debug.

That backfill must never save the `Instance` the caller handed in. `check_updates` holds its
copy across every network round trip, so another writer — the GUI enabling a mod, a second
command — can rewrite `instance.toml` in between, and writing the stale copy back would undo
it. It collects `(project_id, title)` pairs instead, reads the instance from disk again at the
end (`content::save_titles`), applies the titles to whatever entries the fresh copy holds, and
saves that. Nothing was backfilled means nothing is written. Any future code that mutates an
instance after an `await` on the network follows the same rule.

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
step, decimals, display }`, `Toggle`, `Choice(&[(token, label)])`, or `Text`. It is data, not
code — adding a key is one row, and `docs/research/2026-09-07-options-txt-catalog.md` is where
the ranges and tokens come from. That document was checked against a real 26.2 `options.txt` on
2026-09-07; entries it still marks VERIFY were not.

**A choice token carries the quotes the file writes.** Minecraft JSON-quotes some string values
and not others: `renderClouds:"true"` and `mainHand:"right"` are quoted, `lang:en_us` is not.
The catalog stores the quotes as part of the token, so `renderClouds`'s tokens are `"\"true\""`,
`"\"fast\""`, `"\"false\""`. `parse_value` also takes the bare word a person types and gives back
the stored form: `fast` parses to `"fast"`. `settings::doc::normalize(key, value)` is that
rewrite for a caller holding only strings, and the CLI runs every `settings set` value through
it. A token no choice holds is `Error::BadChoice`, whose message lists the bare tokens
(`true, fast, false`).

**`guiScale` is a choice, not a slider.** Its tokens are `0` (Auto) and `1` to `6`, labelled
`1×` to `6×`, in numeric order so a picker still reads like the game's own control. `0` is
not a scale below one, so a slider would have shown it as the smallest size; the catalog names
it instead. A value outside the list is `Error::BadChoice`.

**A slider's stored number is not always the number a user reads.** `Slider.display` is
`Option<Display>`, and `Display { mul, add, decimals, unit }` gives `shown = stored * mul + add`.
`fov` is the only key that uses it: 26.2 stores a float in `[-1.0, 1.0]` where `0.0` is 70
degrees, so `fov` carries `mul: 40.0, add: 70.0, decimals: 0, unit: "°"`. `Setting::to_display`
and `Setting::from_display` convert both ways and are the identity for every other setting.

`catalog::find(key)` looks a key up. `format_value` and `parse_value` round-trip the stored
form Minecraft writes: `true`/`false` for a toggle, a float with its own `decimals` for a
slider, a quoted token for a choice, the string verbatim for text. `parse_value` rejects an
out-of-range number, a non-finite one, and a token no choice holds. `Error::OutOfRange` names
the range the way a user reads it, unit and all — `"fov" must be between 30° and 110°` — while
the value itself stays in the stored form, which is why `gcl settings set` says so in its help.

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

One `(source, project_id)` has one entry and one file. `place_file` replaces an existing entry
where it sits and deletes its file first, so a project can never end up with two jars in
`mods/`. `content::add` only ever asks for that replacement for the top-level request; a
dependency that pins another version is reported in `AddOutcome.conflicts` instead (see the
`mod-sources` skill).

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
