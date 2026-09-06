//! Fabric loader, served by `meta.fabricmc.net`.

use super::{Error, Loader, LoaderCtx, LoaderVersion, fabriclike};

/// Production base URL for Fabric meta.
pub const BASE: &str = "https://meta.fabricmc.net";

/// API version path below [`BASE`].
pub const API: &str = "v2";

/// Lists Fabric loader builds for one Minecraft version.
pub async fn list(ctx: &LoaderCtx<'_>, base: &str, mc: &str) -> Result<Vec<LoaderVersion>, Error> {
    fabriclike::list(ctx, base, API, mc).await
}

/// Installs one Fabric loader build and returns its cached version id.
pub async fn install(
    ctx: &LoaderCtx<'_>,
    base: &str,
    mc: &str,
    loader_version: &str,
) -> Result<String, Error> {
    fabriclike::install(ctx, base, API, Loader::Fabric, mc, loader_version).await
}
