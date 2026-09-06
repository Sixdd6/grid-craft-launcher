---
name: modloaders
description: Fabric, Quilt, Forge, and NeoForge version discovery and headless installation into the shared version cache. Read before touching gcl-core loaders/.
---

Full detail: `docs/research/2026-09-06-mojang-and-modloader-apis.md` sections 2 to 5.

## Common contract

Each loader module exposes, through `loaders::list_versions` and `loaders::install`:

```rust
pub async fn list(ctx: &LoaderCtx<'_>, base: &str, mc: &str) -> Result<Vec<LoaderVersion>, Error>;
pub async fn install(ctx: &LoaderCtx<'_>, base: &str, mc: &str, loader_version: &str)
    -> Result<String, Error>;
```

`LoaderCtx<'a>` carries `http`, `root`, `dl` (`DownloadCtx`), `java: Option<&JavaInstall>` (needed
for Forge and NeoForge), `runner: Option<&dyn ProcessRunner>`, and `mojang: Option<&Mojang>`.
`runner` is the test seam: `None` defaults to `JavaRunner`, which spawns a real child JVM; tests
pass a fake that records calls and writes the files a real processor would (see the `testing`
skill). `LoaderEndpoints { fabric, quilt, forge_meta, forge_maven, neoforge }` holds every base
URL; `Launcher::loader_endpoints()` builds it from `Endpoints::from_env()`.

`loaders::version_id(loader, mc, loader_version)` builds the cached id: `fabric-loader-<v>-<mc>`,
`quilt-loader-<v>-<mc>`, `<mc>-forge-<v>`, `neoforge-<v>`. `install` writes that id's version JSON
to `cache/versions/<id>.json` with `inheritsFrom = mc` and downloads its libraries into
`cache/libraries/`. Calling it again with a complete cache does no network work.
`loaders::keep_both_libraries(loader)` is true for Forge and NeoForge: their profile expects both
the loader's and vanilla's copy of a library on the classpath, so `Mojang::resolve` is told to
keep both rather than let the child version win per `group:artifact:classifier` key.

For Fabric and Quilt, `recommended` is the first `stable` build in the list, or the first entry
when nothing is stable (Quilt publishes no `stable` flag at all).

## Fabric

- `https://meta.fabricmc.net/v2/versions/loader/<mc>` → list with `loader.version`, `loader.stable`.
- `https://meta.fabricmc.net/v2/versions/loader/<mc>/<loader>/profile/json` → version JSON. Save as is.
- Libraries carry `name` and `url` (maven root); newer responses add `sha1`. Build the path from the maven coordinate.

## Quilt

Same shape at `https://meta.quiltmc.org/v3/versions/loader/<mc>` and `.../<mc>/<loader>/profile/json`.

## Forge (1.13+)

- Versions and promotions: `https://files.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json` and `.../promotions_slim.json`. Use `files.minecraftforge.net`, not the maven host: `maven.minecraftforge.net` answers 404 for both documents.
- Installer: `https://maven.minecraftforge.net/net/minecraftforge/forge/<mc>-<forge>/forge-<mc>-<forge>-installer.jar`. Cache under `cache/installers/`.
- Inside: `install_profile.json` (`spec`, `json`, `data`, `processors`, `libraries`), `version.json`, `data/`, `maven/`.

## NeoForge

- Versions: `https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge` → `{ versions: [] }`. Up to major 25, version `A.B.x` targets Minecraft `1.A.B` (`21.1.65` → 1.21.1; `21.0.x` → 1.21). From major 26 the mapping switches to Minecraft's own year-based versioning (`neoforge::mc_for_version`), verified live 2026-09-06: `loader list 26.2 --loader neoforge` returns 26.2.0.x builds.
- NeoForge has no build for Minecraft 1.20.1: that release line shipped as `net.neoforged:forge` 47.1.x instead, under the old artifact name. `verify-source neoforge` and `scripts/e2e.sh` use 1.20.2, the first Minecraft version the `neoforge` artifact covers.
- Installer: `https://maven.neoforged.net/releases/net/neoforged/neoforge/<v>/neoforge-<v>-installer.jar`.
- Same installer layout and algorithm as Forge.

## Forge and NeoForge headless algorithm

`forgelike::install` runs these steps in order:

1. Download the installer jar into `cache/installers/` (skipped when a cache check already
   confirms the install is complete; see "Cache short-circuit" below).
2. Open it (`zip`, in `spawn_blocking`) and read `install_profile.json`. If it has `versionInfo`
   and no `spec`, it is legacy; unsupported in the MVP, return `Error::LegacyInstaller`. Extract
   `maven/**` into `cache/libraries/` — those files ship inside the installer rather than on a
   maven repository.
3. Download every library from `install_profile.libraries` and `version.json.libraries`, skipping
   any whose artifact URL is empty and which names no repository `url` (those are the ones just
   extracted from `maven/`).
4. Install the vanilla Minecraft version the build targets — the processors patch its client jar,
   so it must exist before they run.
5. Build the data map for side `client`: `[g:a:v]` → library path, `'literal'` → literal, `/path`
   → extract from the jar to a temp dir under `cache/installers/<id>/`. Add `MINECRAFT_JAR`
   (vanilla client jar path), `SIDE=client`, `INSTALLER`, `ROOT`, `MINECRAFT_VERSION`,
   `LIBRARY_DIR`.
6. For each processor where `sides` is absent or contains `client`: skip when every `outputs`
   file already exists with the expected sha1. Otherwise run
   `java -cp <processor jar + classpath> <Main-Class from manifest> <args with {KEY} substituted>`
   using the Java picked for that Minecraft version. Output is appended to
   `logs/installers/<id>/<index>-<artifact>.log`.
7. Verify `outputs` once more. Write `version.json` to `cache/versions/<id>.json`.

### Cache short-circuit

A repeated `install` call is a no-op only when the installer jar is still present in
`cache/installers/`: the profile that says which outputs to check lives inside that jar, so the
short-circuit check cannot run without it. If the jar is missing, the whole install runs again
even if the version JSON and processor outputs are still correct.

## Do not

- Do not run the installer's own `main`; run processors.
- Do not extract the whole installer jar; read entries by name.
- Do not assume `sha1` is present on loader libraries; fall back to size, then to a successful download.
