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
//!
//! The work itself is [`super::media::MediaCache`], which the description-image cache uses
//! too; this module is the icon [`Policy`] over it.

use std::path::PathBuf;

use super::Error;
use super::media::{MediaCache, Policy, Pruned};
use crate::http::HttpClient;
use crate::paths::Root;

/// Hosts the content sources serve project icons from.
pub const ALLOWED_HOSTS: [&str; 3] = [
    "cdn.modrinth.com",
    "media.forgecdn.net",
    "edge.forgecdn.net",
];

/// Largest icon body accepted, in bytes.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Largest `cache/icons/` may grow before [`IconCache::prune`] deletes its oldest files.
pub const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;

/// Downloads icons into `cache/icons/`, at most one request per URL at a time.
///
/// Hold one per launcher. It is `Send + Sync`, so any thread may ask it for an icon.
#[derive(Debug)]
pub struct IconCache {
    inner: MediaCache,
}

impl Default for IconCache {
    fn default() -> Self {
        Self {
            inner: MediaCache::new(policy()),
        }
    }
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
    /// seam; pass an empty slice for the shipped list on its own. Two concurrent calls for
    /// one URL make one request: the second waits on the first and then finds the file.
    #[tracing::instrument(skip(self, http, root, extra_hosts))]
    pub async fn fetch(
        &self,
        http: &HttpClient,
        root: &Root,
        url: &str,
        extra_hosts: &[String],
    ) -> Result<PathBuf, Error> {
        self.inner.fetch(http, root, url, extra_hosts).await
    }

    /// Deletes the oldest icons until `cache/icons/` fits in `max_bytes`.
    ///
    /// Pass [`MAX_CACHE_BYTES`] for the shipped cap. Blocking filesystem work: see
    /// [`super::media::MediaCache::prune`].
    pub fn prune(&self, root: &Root, max_bytes: u64) -> Result<Pruned, Error> {
        self.inner.prune(root, max_bytes)
    }
}

/// The icon policy: the source CDNs, 2 MiB, `cache/icons/`.
fn policy() -> Policy {
    Policy {
        allowed_hosts: Some(ALLOWED_HOSTS.iter().map(|h| (*h).to_string()).collect()),
        max_bytes: MAX_BYTES,
        refuse_private: true,
        dir: Root::icons_dir,
    }
}

/// The cache file name for `url`: its sha1 in hex, then the extension label.
#[must_use]
pub fn cache_file_name(url: &str) -> String {
    super::media::cache_file_name(url)
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
    fn only_the_source_cdns_are_allowed_by_default() {
        let policy = policy();
        for host in ALLOWED_HOSTS {
            assert!(
                super::super::media::host_allowed(host, &[], &policy),
                "{host}"
            );
        }
        assert!(!super::super::media::host_allowed(
            "evil.example",
            &[],
            &policy
        ));
        assert!(super::super::media::host_allowed(
            "evil.example",
            &["evil.example".to_string()],
            &policy
        ));
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
            !icons.inner.in_flight(&url),
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

    #[tokio::test]
    async fn a_redirect_to_a_host_outside_the_allowlist_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/hop.png"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "https://evil.example/icon.png"),
            )
            .mount(&server)
            .await;

        let url = format!("{}/hop.png", server.uri());
        let err = IconCache::new()
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect_err("the hop leaves the allowlist");

        assert!(matches!(err, Error::DisallowedHost { .. }), "{err:?}");
        assert!(!root.icons_dir().join(cache_file_name(&url)).exists());
    }

    #[tokio::test]
    async fn a_backslash_before_the_at_sign_does_not_make_a_url_the_cdn() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let url = "https://evil.example\\@cdn.modrinth.com/a.png";
        let err = IconCache::new()
            .fetch(&client(), &root, url, &[])
            .await
            .expect_err("the host is evil.example, not the CDN");

        assert!(
            matches!(&err, Error::DisallowedHost { host, .. } if host == "evil.example"),
            "{err:?}"
        );
        assert!(!root.icons_dir().join(cache_file_name(url)).exists());
    }
}
