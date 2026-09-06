# Architecture

Three crates. Logic lives in `gcl-core`. The binaries render and forward.

| Crate | Binary | Role |
|---|---|---|
| `gcl-core` | none | all launcher logic; tokio runtime owner |
| `gcl-cli` | `gcl` | clap commands over core; prints text or JSON |
| `gcl-ui` | `grid-craft-launcher` | Slint screens over core; no logic |

## gcl-core modules

Each module owns one concern. Its public interface is documented at the top of its `mod.rs`.
Add a row here when you add a module.

| Module | Owns | Depends on |
|---|---|---|
| `paths` | root resolution, subdirectory constants | none |
| `config` | `config.toml` read and write, env overrides | none |
| `http` | shared reqwest client, User-Agent, retry, rate-limit backoff | none |
| `download` | content-addressed cache, parallel queue, hash checks, progress events | `http`, `paths`, `events` |
| `mojang` | version manifest, version JSON, rules, argument templating, assets, `inheritsFrom` merge | `download`, `http`, `paths`, `events` |
| `java` | detect runtimes, fetch Mojang runtimes, pick by major version. The runtime component comes from the version JSON (`javaVersion.component`); `component_for_major` only guesses when the version JSON names none | `download`, `http`, `paths` |
| `loaders` (planned, plan 2+) | `fabric`, `quilt`, `forge`, `neoforge` producing version JSONs | `download`, `mojang`, `java` |
| `sources` (planned, plan 2+) | `Source` trait; `modrinth`, `curseforge` clients | `http`, `download` |
| `modpacks` (planned, plan 2+) | import mrpack and CurseForge zip into a new instance | `sources`, `instances`, `loaders` |
| `instances` | instance layout, `instance.toml`, content list (install into folders: plan 2), `list` skips unparsable instances with a warning | `paths` |
| `settings` (planned, plan 2+) | `options.txt` preseed and keyed overrides | `instances` |
| `auth` (planned, plan 2+) | Microsoft device-code chain, refresh, keyring, offline accounts | `http`, `config` |
| `launch` (planned, plan 2+) | classpath, arguments, spawn, log streaming | `instances`, `mojang`, `loaders`, `java`, `auth`, `settings` |
| `events` | `Progress` and `LogLine` event types and the channel | none |
| `launcher` | `Launcher` handle: owns the tokio runtime, root, config, HttpClient, event channel, and cancellation token; every binary entry point goes through it. `Launcher::mojang()` reads `GCL_MOJANG_BASE_URL` (test-only) to point at a mock server instead of `piston-meta` | `config`, `http`, `events`, `paths`, `download`, `mojang`, `java`, `instances` |

Rules:

- Nothing depends on `launch`.
- `sources` never touches `instances`. Placing files is `instances::install`.
- Blocking work (zip, hashing, processors) runs under `tokio::task::spawn_blocking`.
- Every remote JSON has a serde struct in the owning module and a fixture under `tests/fixtures/`.
- `launcher` is the only module binaries call directly.

## Event flow

Core functions take an `EventSink` (an `mpsc::Sender<Event>`). The CLI prints events. The UI
forwards them to the Slint event loop with `slint::invoke_from_event_loop` and updates models.

## Threading

`gcl-core` builds one tokio runtime in `Launcher::new()`. Binaries call `Launcher` methods and
never create a runtime. The UI runs on the main thread and talks to core through a `Launcher`
handle that spawns tasks on the runtime.

## App root layout

```
<root>/
  config.toml
  accounts.json
  instances/<slug>/instance.toml
  instances/<slug>/.minecraft/
  cache/versions/<id>.json
  cache/libraries/<maven path>
  cache/assets/{indexes,objects}/
  cache/natives/<version>/
  cache/runtimes/<component>/<platform>/
  cache/installers/
  cache/objects/<sha1[0..2]>/<sha1>
  logs/
```
