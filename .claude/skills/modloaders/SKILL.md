---
name: modloaders
description: Fabric, Quilt, Forge, and NeoForge version discovery and headless installation into the shared version cache. Read before touching gcl-core loaders/.
---

Full detail: `docs/research/2026-09-06-mojang-and-modloader-apis.md` sections 2 to 5.

## Common contract

Each loader module exposes:

```rust
pub async fn list_versions(client: &HttpClient, mc: &str) -> Result<Vec<LoaderVersion>, Error>;
pub async fn install(ctx: &InstallCtx, mc: &str, loader: &str) -> Result<VersionId, Error>;
```

`install` writes a version JSON to `cache/versions/<id>.json` with `inheritsFrom = mc`, downloads
its libraries into `cache/libraries/`, and returns the id (`fabric-loader-<v>-<mc>`,
`quilt-loader-<v>-<mc>`, `<mc>-forge-<v>`, `neoforge-<v>`). Calling it again with a complete
cache does no network work.

## Fabric

- `https://meta.fabricmc.net/v2/versions/loader/<mc>` → list with `loader.version`, `loader.stable`.
- `https://meta.fabricmc.net/v2/versions/loader/<mc>/<loader>/profile/json` → version JSON. Save as is.
- Libraries carry `name` and `url` (maven root); newer responses add `sha1`. Build the path from the maven coordinate.

## Quilt

Same shape at `https://meta.quiltmc.org/v3/versions/loader/<mc>` and `.../<mc>/<loader>/profile/json`.

## Forge (1.13+)

- Versions: `https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.json` → `{ "<mc>": ["<mc>-<forge>", ...] }`. Recommended/latest from `promotions_slim.json`.
- Installer: `https://maven.minecraftforge.net/net/minecraftforge/forge/<mc>-<forge>/forge-<mc>-<forge>-installer.jar`. Cache under `cache/installers/`.
- Inside: `install_profile.json` (`spec`, `json`, `data`, `processors`, `libraries`), `version.json`, `data/`, `maven/`.

## NeoForge

- Versions: `https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge` → `{ versions: [] }`. Version `A.B.x` targets Minecraft `1.A.B` (`21.1.65` → 1.21.1; `21.0.x` → 1.21).
- Installer: `https://maven.neoforged.net/releases/net/neoforged/neoforge/<v>/neoforge-<v>-installer.jar`.
- Same installer layout and algorithm as Forge.

## Forge and NeoForge headless algorithm

1. Download and open the installer jar (`zip`, in `spawn_blocking`).
2. Read `install_profile.json`. If it has `versionInfo` and no `spec`, it is legacy; unsupported in the MVP, return `Error::LegacyInstaller`.
3. Extract `maven/**` into `cache/libraries/`.
4. Download every library from `install_profile.libraries` and `version.json.libraries`.
5. Build the data map for side `client`: `[g:a:v]` → library path, `'literal'` → literal, `/path` → extract from jar to a temp dir. Add `MINECRAFT_JAR` (vanilla client jar path), `SIDE=client`, `INSTALLER`, `ROOT`, `MINECRAFT_VERSION`, `LIBRARY_DIR`.
6. For each processor where `sides` is absent or contains `client`: skip if every `outputs` file exists with the expected sha1. Otherwise run `java -cp <processor jar + classpath> <Main-Class from manifest> <args with {KEY} substituted>` using the Java picked for that Minecraft version. Capture output to `logs/`.
7. Verify `outputs`. Write `version.json` to `cache/versions/<id>.json`.

## Do not

- Do not run the installer's own `main`; run processors.
- Do not extract the whole installer jar; read entries by name.
- Do not assume `sha1` is present on loader libraries; fall back to size, then to a successful download.
