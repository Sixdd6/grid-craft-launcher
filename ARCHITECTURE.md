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
| `loaders` | `fabric`, `quilt`, `forge`, `neoforge`: version listing and headless install into `cache/versions/<id>.json`. `LoaderCtx` carries `http`, `root`, `dl`, `java`, `runner` (the `ProcessRunner` test seam; production uses `JavaRunner`), `mojang`. `LoaderEndpoints` holds the four base URLs | `download`, `mojang`, `java` |
| `sources` (planned, plan 3) | `Source` trait; `modrinth`, `curseforge` clients | `http`, `download` |
| `modpacks` (planned, plan 3) | import mrpack and CurseForge zip into a new instance | `sources`, `instances`, `loaders` |
| `instances` | instance layout, `instance.toml`, content list (install into folders: plan 2), `list` skips unparsable instances with a warning; instance creation preseeds `options.txt` through `settings` | `paths`, `settings` |
| `settings` | `options.txt` preseed (`apply_preseed`) and keyed overrides (`apply_overrides_to`), plus `validate_key`/`validate_value`. Takes a game directory and a map, so it does not depend on `instances` | `paths` |
| `auth` | offline accounts (`auth::offline`) and the account store (`auth::store`, `accounts.json`); `LaunchIdentity` placeholders. The Microsoft device-code chain is plan 4 | `paths` |
| `launch` | `launch::command::build` turns an `InstallPlan`, account, instance, and JVM settings into a `LaunchCommand`; `launch::spawn` starts and streams it | `instances`, `mojang`, `auth` |
| `events` | `Progress` and `LogLine` event types and the channel | none |
| `launcher` | `Launcher` handle: owns the tokio runtime, root, config, HttpClient, event channel, and cancellation token; every binary entry point goes through it. Orchestrates `install_loader`, `install_instance`, `launch_instance`, `java_for_version`, `apply_settings_overrides`. `Endpoints::from_env()` reads `GCL_MOJANG_BASE_URL`, `GCL_FABRIC_BASE_URL`, `GCL_QUILT_BASE_URL`, `GCL_FORGE_META_BASE_URL`, `GCL_FORGE_MAVEN_BASE_URL`, and `GCL_NEOFORGE_BASE_URL` (all test-only); `open_with_endpoints` is the test seam that takes `Endpoints` directly and reads no environment | `config`, `http`, `events`, `paths`, `download`, `mojang`, `java`, `instances`, `loaders`, `auth`, `launch`, `settings` |

Rules:

- Only `launcher` depends on `launch`.
- `sources` never touches `instances`. Placing files is `instances::install`.
- Blocking work (zip, hashing, processors) runs under `tokio::task::spawn_blocking`.
- Every remote JSON has a serde struct in the owning module and a fixture under `tests/fixtures/`.
- `launcher` is the only module binaries call directly.

## Launch flow

`Launcher::launch_instance(slug, account, offline_user, dry_run)` runs these steps in order:

1. Resolve the account: `offline_user` creates or reuses an offline account, `account` selects a
   saved one by id or name, otherwise the active account is used.
2. `install_loader`: resolve the instance's loader version (`recommended`/`latest`/unset become
   a concrete build) and install it into the shared version cache.
3. `install_instance`: resolve the merged version JSON and install libraries, assets, and
   natives.
4. `java_for_version` / `ensure_java_for`: pick or install the Java runtime the version JSON
   asks for, unless the instance or config names a Java path.
5. `apply_settings_overrides`: write the instance's `options.txt` overrides.
6. `launch::build`: build the `LaunchCommand`.
7. `launch::spawn` and `launch::wait`: start Java and stream its output, unless `dry_run` is
   set, in which case the command is returned unstarted.

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
