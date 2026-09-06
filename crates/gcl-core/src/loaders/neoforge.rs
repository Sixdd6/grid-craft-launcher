//! NeoForge: the maven version API, its Minecraft version mapping, and installer URLs.
//!
//! Installing is shared with Forge; see [`super::forgelike::install`].

use serde::Deserialize;

use super::fabriclike::segment;
use super::{Error, Loader, LoaderCtx, LoaderVersion, sort_newest_first};

/// Production maven host, which serves the version API and installer jars.
pub const MAVEN: &str = "https://maven.neoforged.net";

/// Repository path every release lives under.
const GROUP_PATH: &str = "net/neoforged/neoforge";

/// First NeoForge major that is a Minecraft release year rather than a `1.x` minor.
const YEAR_MAJOR: u32 = 26;

/// `GET /api/maven/versions/releases/net/neoforged/neoforge`.
#[derive(Debug, Deserialize)]
struct Versions {
    #[serde(default)]
    versions: Vec<String>,
}

/// Lists NeoForge builds for one Minecraft version, newest first.
///
/// The API returns every build in upstream order, which mixes release lines, so the list is
/// filtered by [`mc_for_version`] and sorted numerically. A `-beta` build is not stable; the
/// newest stable build is recommended, or the newest build when none is stable.
pub async fn list(ctx: &LoaderCtx<'_>, base: &str, mc: &str) -> Result<Vec<LoaderVersion>, Error> {
    let url = format!(
        "{}/api/maven/versions/releases/{GROUP_PATH}",
        base.trim_end_matches('/')
    );
    let all: Versions = ctx.http.get_json(&url).await?;
    let mut versions: Vec<LoaderVersion> = all
        .versions
        .iter()
        .filter(|v| mc_for_version(v).as_deref() == Some(mc))
        .map(|v| LoaderVersion {
            stable: !v.contains("-beta"),
            recommended: false,
            version: v.clone(),
        })
        .collect();
    sort_newest_first(&mut versions);
    if versions.is_empty() {
        return Err(Error::Unsupported(mc.to_string(), Loader::NeoForge));
    }
    let pick = versions.iter().position(|v| v.stable).unwrap_or(0);
    versions[pick].recommended = true;
    Ok(versions)
}

/// Maps a NeoForge version to the Minecraft version it targets.
///
/// Up to major 25 the first two components are Minecraft's minor and patch: `21.1.65` is
/// 1.21.1 and `21.0.5` is 1.21. From major 26 on, Minecraft itself is versioned by year, and
/// the first components carry that version instead: `26.2.0.75` is 26.2.
///
/// Returns `None` for anything that does not start with numeric components.
pub fn mc_for_version(v: &str) -> Option<String> {
    let parts: Vec<&str> = v.split('.').collect();
    let major = number(parts.first()?)?;
    let minor = number(parts.get(1)?)?;
    if major < YEAR_MAJOR {
        return Some(match minor {
            0 => format!("1.{major}"),
            _ => format!("1.{major}.{minor}"),
        });
    }
    // VERIFY against live manifest: no year-based Minecraft release has shipped yet, so the
    // shape of these builds is taken from NeoForge's announced scheme.
    let patch = number(parts.get(2)?)?;
    Some(match patch {
        0 => format!("{major}.{minor}"),
        _ => format!("{major}.{minor}.{patch}"),
    })
}

/// Builds the installer jar URL for one build. `base` is the maven host.
///
/// The version is percent-encoded, so it cannot add a path segment.
pub fn installer_url(base: &str, version: &str) -> String {
    let v = segment(version);
    format!(
        "{}/releases/{GROUP_PATH}/{v}/neoforge-{v}-installer.jar",
        base.trim_end_matches('/')
    )
}

/// Reads the leading digits of one version component.
fn number(part: &str) -> Option<u32> {
    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mc_for_version_maps_the_leading_components() {
        assert_eq!(mc_for_version("21.1.65").as_deref(), Some("1.21.1"));
        assert_eq!(mc_for_version("20.4.190").as_deref(), Some("1.20.4"));
        assert_eq!(mc_for_version("21.0.5").as_deref(), Some("1.21"));
        assert_eq!(mc_for_version("21.1.65-beta").as_deref(), Some("1.21.1"));
    }

    #[test]
    fn mc_for_version_maps_year_based_builds_to_their_own_line() {
        assert_eq!(mc_for_version("26.2.0.75").as_deref(), Some("26.2"));
        assert_eq!(mc_for_version("26.2.1.3").as_deref(), Some("26.2.1"));
    }

    #[test]
    fn mc_for_version_rejects_anything_without_numeric_components() {
        assert_eq!(mc_for_version(""), None);
        assert_eq!(mc_for_version("21"), None);
        assert_eq!(mc_for_version("main-SNAPSHOT"), None);
        // A year-based build needs a third component to map at all.
        assert_eq!(mc_for_version("26.2"), None);
    }

    #[test]
    fn installer_url_names_the_version_twice() {
        assert_eq!(
            installer_url(MAVEN, "21.1.250"),
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.250/neoforge-21.1.250-installer.jar"
        );
    }

    #[test]
    fn installer_url_escapes_path_separators() {
        let url = installer_url(MAVEN, "../evil");
        assert!(!url.contains("../"), "{url}");
    }
}
