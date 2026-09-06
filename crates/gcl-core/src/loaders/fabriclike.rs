//! The Fabric meta protocol, which Quilt serves too under a different API version.

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;

use super::{Error, Loader, LoaderCtx, LoaderVersion, version_id};
use crate::mojang::VersionJson;
use crate::paths::{safe_join, write_atomic};

/// Characters escaped in a URL path segment, so a version string cannot add path parts.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'\\')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// Percent-encodes one URL path segment.
fn segment(value: &str) -> String {
    utf8_percent_encode(value, SEGMENT).to_string()
}

/// One entry of `GET /versions/loader/{mc}`. Only the `loader` block matters here.
#[derive(Debug, Deserialize)]
struct Entry {
    loader: EntryLoader,
}

/// The `loader` block of a meta entry. Quilt omits `stable`.
#[derive(Debug, Deserialize)]
struct EntryLoader {
    version: String,
    #[serde(default)]
    stable: bool,
}

/// Lists loader builds for one Minecraft version. The first stable build is recommended.
///
/// Quilt publishes no `stable` flag, so when nothing is stable the newest build is recommended.
pub async fn list(
    ctx: &LoaderCtx<'_>,
    base: &str,
    api_path: &str,
    mc: &str,
) -> Result<Vec<LoaderVersion>, Error> {
    let url = format!(
        "{}/{api_path}/versions/loader/{}",
        base.trim_end_matches('/'),
        segment(mc)
    );
    let entries: Vec<Entry> = ctx.http.get_json(&url).await?;
    let mut recommended_taken = false;
    let mut versions: Vec<LoaderVersion> = entries
        .into_iter()
        .map(|e| {
            let recommended = e.loader.stable && !recommended_taken;
            recommended_taken |= recommended;
            LoaderVersion {
                version: e.loader.version,
                stable: e.loader.stable,
                recommended,
            }
        })
        .collect();
    if !recommended_taken && let Some(first) = versions.first_mut() {
        first.recommended = true;
    }
    Ok(versions)
}

/// Installs one build's profile JSON into `cache/versions/<id>.json` and returns the id.
///
/// Returns without a request when that file already exists and parses.
pub async fn install(
    ctx: &LoaderCtx<'_>,
    base: &str,
    api_path: &str,
    loader: Loader,
    mc: &str,
    loader_version: &str,
) -> Result<String, Error> {
    let id = version_id(loader, mc, loader_version);
    let file = safe_join(&ctx.root.versions_dir(), &format!("{id}.json"))?;
    if let Some(body) = read_optional(&file)?
        && serde_json::from_str::<VersionJson>(&body).is_ok()
    {
        tracing::debug!(%id, "loader profile already cached");
        return Ok(id);
    }

    let url = format!(
        "{}/{api_path}/versions/loader/{}/{}/profile/json",
        base.trim_end_matches('/'),
        segment(mc),
        segment(loader_version)
    );
    let mut profile: VersionJson = ctx.http.get_json(&url).await.map_err(|err| match err {
        crate::http::Error::Status { status: 404, .. } => Error::NoSuchVersion {
            loader,
            mc: mc.to_string(),
            version: loader_version.to_string(),
        },
        other => Error::Http(other),
    })?;
    profile.id = id.clone();
    profile.inherits_from = Some(mc.to_string());

    let bytes = serde_json::to_vec_pretty(&profile).map_err(|source| Error::Json {
        url: file.display().to_string(),
        source,
    })?;
    write_atomic(&file, &bytes)?;
    Ok(id)
}

/// Reads a file, returning `None` when it does not exist.
fn read_optional(path: &std::path::Path) -> Result<Option<String>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::segment;

    #[test]
    fn segment_keeps_ordinary_version_characters() {
        assert_eq!(segment("1.20.1"), "1.20.1");
        assert_eq!(segment("0.20.0-beta.9"), "0.20.0-beta.9");
        assert_eq!(segment("1.21.1_pre~2"), "1.21.1_pre~2");
    }

    #[test]
    fn segment_escapes_anything_that_could_add_a_path_part() {
        assert_eq!(segment("../evil"), "..%2Fevil");
        assert_eq!(segment("a b"), "a%20b");
        assert_eq!(segment("a?b#c"), "a%3Fb%23c");
        assert_eq!(segment("a%2Fb"), "a%252Fb");
    }
}
