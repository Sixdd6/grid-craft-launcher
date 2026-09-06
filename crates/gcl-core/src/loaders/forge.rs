//! Forge: the maven version list, the promotions document, and installer URLs.
//!
//! Installing is shared with NeoForge; see [`super::forgelike::install`].

use std::collections::BTreeMap;

use serde::Deserialize;

use super::fabriclike::segment;
use super::{Error, Loader, LoaderCtx, LoaderVersion};

/// Production maven host, which serves installer jars.
pub const MAVEN: &str = "https://maven.minecraftforge.net";

/// Production metadata host, which serves the version list and the promotions.
///
/// The maven host answers 404 for both documents, so they are fetched from here.
pub const META: &str = "https://files.minecraftforge.net";

/// Repository path the metadata documents and every installer live under.
const GROUP_PATH: &str = "net/minecraftforge/forge";

/// `promotions_slim.json`: `{ "promos": { "<mc>-recommended": "<forge>" } }`.
#[derive(Debug, Deserialize)]
struct Promotions {
    #[serde(default)]
    promos: BTreeMap<String, String>,
}

/// Lists Forge builds for one Minecraft version, newest first.
///
/// `base` is the metadata host. The maven list is in oldest-first release order, so it is
/// reversed. The promotions document names the recommended and the latest build; both count as
/// stable. When it names neither, the newest build is recommended.
pub async fn list(ctx: &LoaderCtx<'_>, base: &str, mc: &str) -> Result<Vec<LoaderVersion>, Error> {
    let base = base.trim_end_matches('/');
    let all: BTreeMap<String, Vec<String>> = ctx
        .http
        .get_json(&format!("{base}/{GROUP_PATH}/maven-metadata.json"))
        .await?;
    let builds = all
        .get(mc)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| Error::Unsupported(mc.to_string(), Loader::Forge))?;

    let promotions: Promotions = ctx
        .http
        .get_json(&format!("{base}/{GROUP_PATH}/promotions_slim.json"))
        .await?;
    let recommended = promotions.promos.get(&format!("{mc}-recommended"));
    let latest = promotions.promos.get(&format!("{mc}-latest"));

    let prefix = format!("{mc}-");
    let mut versions: Vec<LoaderVersion> = builds
        .iter()
        .rev()
        .map(|full| {
            let version = full.strip_prefix(&prefix).unwrap_or(full).to_string();
            let is_recommended = recommended.is_some_and(|r| *r == version);
            LoaderVersion {
                stable: is_recommended || latest.is_some_and(|l| *l == version),
                recommended: is_recommended,
                version,
            }
        })
        .collect();
    if !versions.iter().any(|v| v.recommended)
        && let Some(first) = versions.first_mut()
    {
        first.recommended = true;
    }
    Ok(versions)
}

/// Builds the installer jar URL for one build. `base` is the maven host.
///
/// Both version parts are percent-encoded, so neither can add a path segment.
pub fn installer_url(base: &str, mc: &str, forge: &str) -> String {
    let build = segment(&format!("{mc}-{forge}"));
    format!(
        "{}/{GROUP_PATH}/{build}/forge-{build}-installer.jar",
        base.trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_url_names_the_build_twice() {
        assert_eq!(
            installer_url(MAVEN, "1.20.1", "47.4.10"),
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.4.10/forge-1.20.1-47.4.10-installer.jar"
        );
        // A trailing slash on the base does not double up.
        assert_eq!(
            installer_url("http://host/", "1.20.1", "47.4.10"),
            "http://host/net/minecraftforge/forge/1.20.1-47.4.10/forge-1.20.1-47.4.10-installer.jar"
        );
    }

    #[test]
    fn installer_url_escapes_path_separators() {
        let url = installer_url(MAVEN, "../..", "evil/x");
        assert!(!url.contains("../"), "{url}");
        assert!(url.contains("..%2F..-evil%2Fx"), "{url}");
    }
}
