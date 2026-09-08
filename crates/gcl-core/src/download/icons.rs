//! URL-keyed icon cache under `cache/icons/`.
//!
//! Project icons have no published hash, so they cannot use the sha1-addressed object store
//! the rest of [`crate::download`] uses. The cache key is the URL instead: an icon lands at
//! `cache/icons/<sha1(url)>.<ext>`, where `<ext>` is a label taken from the URL's path and
//! not a promise about the bytes (the decoder sniffs the real format). A file already at that
//! path is returned without a request.
//!
//! Three guards keep an untrusted URL from costing anything: the URL must be `https://`,
//! the host must be one of [`ALLOWED_HOSTS`] (plus whatever a test adds), and a body over
//! [`MAX_BYTES`] is deleted and refused. The size cap is checked after the transfer, since
//! a host may send no `Content-Length`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{Error, remove_quietly, store_object};
use crate::http::HttpClient;
use crate::paths::{Root, safe_join};

/// Hosts the content sources serve project icons from.
pub const ALLOWED_HOSTS: [&str; 3] = [
    "cdn.modrinth.com",
    "media.forgecdn.net",
    "edge.forgecdn.net",
];

/// Largest icon body accepted, in bytes.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Extension used when the URL's path names none.
const DEFAULT_EXT: &str = "img";

/// Downloads icons into `cache/icons/`, at most one request per URL at a time.
///
/// Hold one per launcher. It is `Send + Sync`, so any thread may ask it for an icon.
#[derive(Debug, Default)]
pub struct IconCache {
    /// One lock per URL that is being fetched right now. An entry is removed once the last
    /// caller waiting on it is done, so a long session does not grow the map forever.
    flight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl IconCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached path for `url`, downloading it first when it is not there.
    ///
    /// The URL must start with `https://` and its host must be allowed, or the call fails
    /// with [`Error::DisallowedHost`] before any request. `extra_hosts` adds to
    /// [`ALLOWED_HOSTS`] for this call and lets that host be plain HTTP, which is the test
    /// seam; pass an empty slice for the shipped list on its own. Two concurrent calls for one URL make one request: the
    /// second waits on the first and then finds the file.
    #[tracing::instrument(skip(self, http, root, extra_hosts))]
    pub async fn fetch(
        &self,
        http: &HttpClient,
        root: &Root,
        url: &str,
        extra_hosts: &[String],
    ) -> Result<PathBuf, Error> {
        let dest = safe_join(&root.icons_dir(), &cache_file_name(url))?;
        if is_file(&dest).await {
            return Ok(dest);
        }
        let host = host_of(url).ok_or_else(|| Error::DisallowedHost {
            url: url.to_string(),
            host: String::new(),
        })?;
        if !scheme_allowed(url, host, extra_hosts) {
            return Err(Error::DisallowedHost {
                url: url.to_string(),
                host: host.to_string(),
            });
        }
        if !host_allowed(host, extra_hosts) {
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
                download(http, root, url, &dest).await
            }
        };
        self.release(url);
        result
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

/// Streams one icon into a staging file, checks its size, and moves it to `dest`.
///
/// The size cap is checked after the transfer, not from `Content-Length`: a host may send
/// none, and the allowlist already keeps this off arbitrary servers. An oversized body is
/// deleted rather than stored.
async fn download(
    http: &HttpClient,
    root: &Root,
    url: &str,
    dest: &Path,
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
    if result.size > MAX_BYTES {
        remove_quietly(&part).await;
        return Err(Error::TooLarge {
            url: url.to_string(),
            size: result.size,
            max: MAX_BYTES,
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

/// Whether an icon URL's scheme is allowed.
///
/// It must be `https://`. The one exception is a host a caller named in `extra_hosts`,
/// which is the test seam ([`crate::Launcher::with_icon_hosts`], empty in a shipped
/// launcher): a wiremock server speaks plain HTTP. A URL naming no scheme at all is
/// refused, since [`host_of`] needs one.
fn scheme_allowed(url: &str, host: &str, extra: &[String]) -> bool {
    url.starts_with("https://")
        || extra
            .iter()
            .any(|allowed| allowed.trim().eq_ignore_ascii_case(host))
}

/// Whether an icon may be fetched from `host`.
fn host_allowed(host: &str, extra: &[String]) -> bool {
    ALLOWED_HOSTS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(host))
        || extra
            .iter()
            .any(|allowed| allowed.trim().eq_ignore_ascii_case(host))
}

/// The host part of an absolute URL, without userinfo or port.
///
/// A comparison helper for the allowlist, not a URL parser: the authority between `://` and
/// the first `/`, `?`, or `#`, without a `user:pass@` prefix and without IPv6 brackets.
fn host_of(url: &str) -> Option<&str> {
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
    use std::time::Duration;
    use wiremock::matchers::{method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client() -> HttpClient {
        HttpClient::new()
            .expect("client builds")
            .with_backoff(vec![Duration::ZERO; 3])
    }

    /// The wiremock server runs on `127.0.0.1`, which no shipped allowlist entry covers.
    fn local_hosts() -> Vec<String> {
        vec!["127.0.0.1".to_string(), "localhost".to_string()]
    }

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
    fn only_the_source_cdns_are_allowed_by_default() {
        for host in ALLOWED_HOSTS {
            assert!(host_allowed(host, &[]), "{host}");
        }
        assert!(!host_allowed("evil.example", &[]));
        assert!(host_allowed("evil.example", &["evil.example".to_string()]));
    }

    #[tokio::test]
    async fn fetch_writes_under_cache_icons_hashed_by_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/icon.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"icon bytes".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/icon.png", server.uri());
        let icons = IconCache::new();
        let path = icons
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect("icon fetched");

        assert_eq!(path, root.icons_dir().join(cache_file_name(&url)));
        assert_eq!(std::fs::read(&path).expect("read icon"), b"icon bytes");
    }

    #[tokio::test]
    async fn fetch_reuses_the_cached_file_without_a_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/icon.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"icon bytes".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/icon.png", server.uri());
        let icons = IconCache::new();
        let http = client();
        let first = icons
            .fetch(&http, &root, &url, &local_hosts())
            .await
            .expect("first fetch");
        // A second cache is used on purpose: the hit must come from the file on disk, not
        // from any state the first call left in memory.
        let second = IconCache::new()
            .fetch(&http, &root, &url, &local_hosts())
            .await
            .expect("second fetch");

        assert_eq!(first, second);
        // The mock's `expect(1)` is checked when the server drops; make the count explicit.
        assert_eq!(server.received_requests().await.map(|r| r.len()), Some(1));
    }

    #[tokio::test]
    async fn fetch_is_single_flight_for_one_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/icon.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"icon bytes".to_vec())
                    .set_delay(Duration::from_millis(300)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/icon.png", server.uri());
        let icons = IconCache::new();
        let http = client();
        let hosts = local_hosts();
        let (first, second) = tokio::join!(
            icons.fetch(&http, &root, &url, &hosts),
            icons.fetch(&http, &root, &url, &hosts)
        );

        assert_eq!(first.expect("first"), second.expect("second"));
        assert_eq!(server.received_requests().await.map(|r| r.len()), Some(1));
        assert!(
            icons
                .flight
                .lock()
                .expect("flight lock")
                .get(&url)
                .is_none(),
            "the per-URL lock is dropped once both callers are done"
        );
    }

    #[tokio::test]
    async fn a_body_over_the_cap_is_refused_and_not_stored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        let big = vec![7u8; (MAX_BYTES + 1) as usize];
        Mock::given(method("GET"))
            .and(path_matcher("/big.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(big))
            .mount(&server)
            .await;

        let url = format!("{}/big.png", server.uri());
        let err = IconCache::new()
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect_err("the body is over the cap");

        assert!(
            matches!(err, Error::TooLarge { size, max, .. } if size == MAX_BYTES + 1 && max == MAX_BYTES),
            "{err:?}"
        );
        assert!(!root.icons_dir().join(cache_file_name(&url)).exists());
    }

    #[tokio::test]
    async fn a_url_that_is_not_https_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        for url in [
            "http://cdn.modrinth.com/icon.png",
            "ftp://cdn.modrinth.com/icon.png",
            "cdn.modrinth.com/icon.png",
        ] {
            let err = IconCache::new()
                .fetch(&client(), &root, url, &[])
                .await
                .expect_err("the scheme is not https");
            assert!(
                matches!(err, Error::DisallowedHost { .. }),
                "{url}: {err:?}"
            );
            assert!(!root.icons_dir().join(cache_file_name(url)).exists());
        }
    }

    #[tokio::test]
    async fn a_host_outside_the_allowlist_is_refused_without_a_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/icon.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"icon".to_vec()))
            .expect(0)
            .mount(&server)
            .await;

        let err = IconCache::new()
            .fetch(
                &client(),
                &root,
                "https://evil.example/icon.png",
                &["localhost".to_string()],
            )
            .await
            .expect_err("the host is not allowed");

        assert!(
            matches!(&err, Error::DisallowedHost { host, .. } if host == "evil.example"),
            "{err:?}"
        );
    }
}
