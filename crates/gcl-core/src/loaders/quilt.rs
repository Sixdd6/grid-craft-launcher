//! Quilt loader, served by `meta.quiltmc.org` with the same shape as Fabric meta.

use super::{Error, Loader, LoaderCtx, LoaderVersion, fabriclike};

/// Production base URL for Quilt meta.
pub const BASE: &str = "https://meta.quiltmc.org";

/// API version path below [`BASE`].
pub const API: &str = "v3";

/// Lists Quilt loader builds for one Minecraft version.
pub async fn list(ctx: &LoaderCtx<'_>, base: &str, mc: &str) -> Result<Vec<LoaderVersion>, Error> {
    fabriclike::list(ctx, base, API, mc).await
}

/// Installs one Quilt loader build and returns its cached version id.
pub async fn install(
    ctx: &LoaderCtx<'_>,
    base: &str,
    mc: &str,
    loader_version: &str,
) -> Result<String, Error> {
    fabriclike::install(ctx, base, API, Loader::Quilt, mc, loader_version).await
}
