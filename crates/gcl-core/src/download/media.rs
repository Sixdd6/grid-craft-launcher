//! URL-keyed cache shared by the icon and description-image caches.
//!
//! Neither icons nor description images have a published hash, so they cannot use the
//! sha1-addressed object store the rest of [`crate::download`] uses. The cache key is the
//! URL instead: a file lands at `<dir>/<sha1(url)>.<ext>`, where `<ext>` is a label taken
//! from the URL's path and not a promise about the bytes (the decoder sniffs the real
//! format). A file already at that path is returned without a request.
//!
//! What the two callers differ in is a [`Policy`]: which hosts are allowed, how large a
//! body may be, and which cache directory the file lands in. Everything else — the
//! `https://` rule, the staging file, the after-the-fact size check, and the one request
//! per URL at a time — lives here once.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{Error, remove_quietly, store_object};
use crate::http::HttpClient;
use crate::paths::{Root, safe_join};

/// Extension used when the URL's path names none.
const DEFAULT_EXT: &str = "img";

/// What one caller of [`MediaCache`] allows.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Hosts a file may be fetched from. `None` means any host, which is what a
    /// description image gets; `Some(list)` is the icon cache's CDN allowlist.
    pub allowed_hosts: Option<Vec<String>>,
    /// Largest body accepted, in bytes. Checked after the transfer, not from
    /// `Content-Length`: a host may send none.
    pub max_bytes: u64,
    /// The cache directory the file lands in, as a [`Root`] accessor.
    pub dir: fn(&Root) -> PathBuf,
}

/// Downloads URL-keyed files into one cache directory, at most one request per URL at a time.
///
/// It is `Send + Sync`, so any thread may ask it for a file.
#[derive(Debug, Default)]
pub struct MediaCache {
    /// One lock per URL that is being fetched right now. An entry is removed once the last
    /// caller waiting on it is done, so a long session does not grow the map forever.
    flight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl MediaCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached path for `url` under `policy.dir`, downloading it first when it
    /// is not there.
    ///
    /// The URL must start with `https://` and, when `policy.allowed_hosts` names a list,
    /// its host must be on that list — both checked before any request, both answered with
    /// [`Error::DisallowedHost`]. `extra_hosts` adds to the allowlist for this call and
    /// lets that host be plain HTTP, which is the test seam; pass an empty slice outside a
    /// test. Two concurrent calls for one URL make one request: the second waits on the
    /// first and then finds the file.
    pub async fn fetch(
        &self,
        http: &HttpClient,
        root: &Root,
        url: &str,
        extra_hosts: &[String],
        policy: &Policy,
    ) -> Result<PathBuf, Error> {
        let dest = safe_join(&(policy.dir)(root), &cache_file_name(url))?;
        if is_file(&dest).await {
            return Ok(dest);
        }
        let host = host_of(url).ok_or_else(|| Error::DisallowedHost {
            url: url.to_string(),
            host: String::new(),
        })?;
        if !scheme_allowed(url, host, extra_hosts) || !host_allowed(host, extra_hosts, policy) {
            return Err(Error::DisallowedHost {
                url: url.to_string(),
                host: host.to_string(),
            });
        }

        let lock = self.lock_for(url);
        let result = {
            let _guard = lock.lock().await;
            if is_file(&dest).await {
                Ok(dest.clone())
            } else {
                download(http, root, url, &dest, policy.max_bytes).await
            }
        };
        self.release(url);
        result
    }

    /// Whether a fetch of `url` is in flight. Test helper.
    #[cfg(test)]
    pub(crate) fn in_flight(&self, url: &str) -> bool {
        self.flight
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .contains_key(url)
    }

    /// The per-URL lock, created on first use.
    fn lock_for(&self, url: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.flight.lock().unwrap_or_else(|err| err.into_inner());
        Arc::clone(map.entry(url.to_string()).or_default())
    }

    /// Drops the per-URL lock once no other caller holds it.
    fn release(&self, url: &str) {
        let mut map = self.flight.lock().unwrap_or_else(|err| err.into_inner());
        // Two strong counts: the map's and the caller's, which is about to go out of scope.
        if map
            .get(url)
            .is_some_and(|lock| Arc::strong_count(lock) <= 2)
        {
            map.remove(url);
        }
    }
}

/// Streams one file into a staging file, checks its size, and moves it to `dest`.
///
/// The size cap is checked after the transfer, not from `Content-Length`: a host may send
/// none. An oversized body is deleted rather than stored.
async fn download(
    http: &HttpClient,
    root: &Root,
    url: &str,
    dest: &Path,
    max_bytes: u64,
) -> Result<PathBuf, Error> {
    let part = root
        .objects_dir()
        .join("tmp")
        .join(format!("{}.part", uuid::Uuid::new_v4()));
    let result = match http.stream_to_file(url, &part, None, &mut |_| {}).await {
        Ok(result) => result,
        Err(err) => {
            remove_quietly(&part).await;
            return Err(Error::Http(err));
        }
    };
    if result.size > max_bytes {
        remove_quietly(&part).await;
        return Err(Error::TooLarge {
            url: url.to_string(),
            size: result.size,
            max: max_bytes,
        });
    }
    store_object(&part, dest).await?;
    Ok(dest.to_path_buf())
}

/// True when `path` is a file that exists.
async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .is_ok_and(|meta| meta.is_file())
}

/// The cache file name for `url`: its sha1 in hex, then the extension label.
#[must_use]
pub fn cache_file_name(url: &str) -> String {
    format!(
        "{}.{}",
        super::hash::sha1_hex(url.as_bytes()),
        extension_of(url)
    )
}

/// The extension label for a URL, or [`DEFAULT_EXT`] when it carries none worth keeping.
///
/// Only a short all-alphanumeric suffix of the last path segment counts, so a query string
/// or an odd path cannot steer the file name.
fn extension_of(url: &str) -> String {
    let path = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let last = path.rsplit('/').next().unwrap_or_default();
    let ext = last.rsplit_once('.').map_or("", |(_, ext)| ext);
    if !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext.to_ascii_lowercase()
    } else {
        DEFAULT_EXT.to_string()
    }
}

/// Whether a URL's scheme is allowed.
///
/// It must be `https://`. The one exception is a host a caller named in `extra_hosts`,
/// which is the test seam ([`crate::Launcher::with_icon_hosts`] and
/// [`crate::Launcher::with_image_hosts`], both empty in a shipped launcher): a wiremock
/// server speaks plain HTTP. A URL naming no scheme at all is refused, since [`host_of`]
/// needs one.
pub(crate) fn scheme_allowed(url: &str, host: &str, extra: &[String]) -> bool {
    url.starts_with("https://")
        || extra
            .iter()
            .any(|allowed| allowed.trim().eq_ignore_ascii_case(host))
}

/// Whether a file may be fetched from `host` under `policy`.
///
/// A policy with no host list allows every host, so only the scheme rule applies there.
pub(crate) fn host_allowed(host: &str, extra: &[String], policy: &Policy) -> bool {
    let Some(allowed) = policy.allowed_hosts.as_ref() else {
        return true;
    };
    allowed
        .iter()
        .chain(extra.iter())
        .any(|allowed| allowed.trim().eq_ignore_ascii_case(host))
}

/// The host part of an absolute URL, without userinfo or port.
///
/// A comparison helper for the allowlist, not a URL parser: the authority between `://` and
/// the first `/`, `?`, or `#`, without a `user:pass@` prefix and without IPv6 brackets.
pub(crate) fn host_of(url: &str) -> Option<&str> {
    let authority = url.split_once("://")?.1;
    let authority = authority.split(['/', '?', '#']).next()?;
    let authority = match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    };
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().filter(|h| !h.is_empty());
    }
    authority.split(':').next().filter(|h| !h.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_file_name_is_the_url_sha1_plus_its_extension() {
        let url = "https://cdn.modrinth.com/data/AAAA/icon.png";
        let sha1 = super::super::hash::sha1_hex(url.as_bytes());
        assert_eq!(cache_file_name(url), format!("{sha1}.png"));
    }

    #[test]
    fn a_url_without_a_usable_extension_gets_the_default_label() {
        assert!(cache_file_name("https://cdn.modrinth.com/data/AAAA/icon").ends_with(".img"));
        assert!(cache_file_name("https://cdn.modrinth.com/icon.p%20g").ends_with(".img"));
        assert!(cache_file_name("https://cdn.modrinth.com/i.PNG?w=64").ends_with(".png"));
    }

    #[test]
    fn a_policy_without_a_host_list_allows_every_host() {
        let open = Policy {
            allowed_hosts: None,
            max_bytes: 1,
            dir: Root::images_dir,
        };
        assert!(host_allowed("anything.example", &[], &open));

        let closed = Policy {
            allowed_hosts: Some(vec!["cdn.example".to_string()]),
            max_bytes: 1,
            dir: Root::icons_dir,
        };
        assert!(host_allowed("cdn.example", &[], &closed));
        assert!(!host_allowed("evil.example", &[], &closed));
        assert!(host_allowed(
            "evil.example",
            &["evil.example".to_string()],
            &closed
        ));
    }

    #[test]
    fn the_host_is_read_without_userinfo_or_port() {
        assert_eq!(
            host_of("https://user:pw@cdn.example:8443/a.png"),
            Some("cdn.example")
        );
        assert_eq!(host_of("https://[::1]:8443/a.png"), Some("::1"));
        assert_eq!(host_of("cdn.example/a.png"), None);
    }
}
