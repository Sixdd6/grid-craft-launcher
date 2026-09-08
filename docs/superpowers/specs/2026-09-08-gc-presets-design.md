# Garbage collector presets: design

Date: 2026-09-08. Status: approved in chat.

## Goal

An instance can pick a garbage collector preset. The picker offers only collectors the Java
that instance launches with supports. The launch applies the matching flags.

## Design

### Core

- `GcPreset` enum in `gcl-core::launch` (or `java`): `Default`, `Serial`, `Parallel`, `G1`,
  `Zgc`, `ZgcGenerational`, `Shenandoah`. `InstanceConfig.jvm.gc: GcPreset` with serde default
  `Default` and lowercase snake tokens (`zgc_generational`), so old `instance.toml` files load.
- `GcPreset::flags(major) -> Vec<String>`: `Default` none; `Serial` `-XX:+UseSerialGC`;
  `Parallel` `-XX:+UseParallelGC`; `G1` `-XX:+UseG1GC`; `Zgc` `-XX:+UseZGC` plus
  `-XX:-ZGenerational` on 21 and 22; `ZgcGenerational` `-XX:+UseZGC` plus `-XX:+ZGenerational`
  on 21 and 22, plain `-XX:+UseZGC` on 23 and later; `Shenandoah` `-XX:+UseShenandoahGC`.
- Probe: `java::gc::probe(java_path) -> Result<GcSupport>` runs `<java> -XX:+PrintFlagsFinal
  -version`, parses the flag names (`UseSerialGC`, `UseParallelGC`, `UseG1GC`, `UseZGC`,
  `ZGenerational`, `UseShenandoahGC`) and the major version, and returns
  `GcSupport { major, presets: Vec<GcPreset> }`. A preset is supported when its flag is present;
  `ZgcGenerational` needs `UseZGC` and either `ZGenerational` (21, 22) or major >= 23; `Zgc`
  on 23 and later is the same collector and is still listed. Results cache in memory per
  (path, mtime) and on disk at `cache/runtimes/gc-probe.json`.
- `Launcher::gc_support(slug)`: resolves the Java the instance launches with (instance
  `java_path`, else config default, else the Mojang runtime for its Minecraft version, using
  the same code the launch uses), installs the runtime if absent (progress events like a
  launch), then probes. `Launcher::set_instance_gc(slug, preset)` validates against the probe
  and saves.
- Launch: the preset's flags are inserted before the user's extra arguments. An extra argument
  that selects a collector (`-XX:+Use…GC`, `-XX:±ZGenerational`) while the preset is not
  `Default` is a launch error naming both. At launch the probe runs again (cached) and a
  preset the JVM lacks is a launch error before spawn.

### CLI

`gcl config set-jvm` gains `--gc <preset>` for the defaults; `gcl instance jvm <slug> --gc
<preset>` (create the subcommand if the instance JVM command does not exist; check the CLI)
and `gcl instance gc <slug>` prints the probed list with the Java it probed. Unsupported
presets fail with the supported list.

### UI

JVM tab: a "Garbage collector" ComboBox (`gc_combo`) under the heap fields, a description
line per preset, and a status line naming the probed Java ("Java 21.0.4 from the Mojang
runtime"). Opening the tab runs `gc_support` through `Bridge`; while a runtime downloads the
status shows "Preparing Java…" and the progress panel. The combo lists only supported presets
(plus Launcher default) and saves on selection through `set_instance_gc`. A saved preset the
current Java lacks shows as "Unavailable: <preset>" with the reason and keeps the value until
the user changes it.

### Tests

Core: flag expansion per preset and major; probe parser over recorded `PrintFlagsFinal` dumps
for Java 17, 21 and 25 (fixtures recorded from the Mojang runtimes on this machine and the
system JDK); conflict rejection; cache by mtime; `gc_support` with a stand-in java script that
prints a canned dump. CLI: set and list. GUI: `flow_instances` gains a sub-flow that opens the
JVM tab, sees the stand-in Java's presets, picks one, and launches; the stand-in Java's
argument file contains the flag.

### Out of scope

Tuning flags beyond the collector choice; JVM vendor detection beyond the probe.
