//! URL-keyed cache for description images, under `cache/images/`.
//!
//! A description at either source may point an `<img>` or a `![alt](url)` at any host its
//! author chose, so this cache — unlike [`super::icons`] — keeps no host allowlist: any
//! `https://` URL is fetched. The two guards left are the scheme and a
//! [`MAX_BYTES`] cap on the body. Everything else (the `cache/images/<sha1(url)>.<ext>`
//! key, the single request per URL, the staging file) is [`super::media::MediaCache`],
//! which the icon cache uses too.

use std::path::PathBuf;

use super::Error;
use super::media::{MediaCache, Policy};
use crate::http::HttpClient;
use crate::paths::Root;

/// Largest description-image body accepted, in bytes.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Downloads description images into `cache/images/`, at most one request per URL at a time.
///
/// Hold one per launcher. It is `Send + Sync`, so any thread may ask it for an image.
#[derive(Debug)]
pub struct ImageCache {
    inner: MediaCache,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self {
            inner: MediaCache::new(policy()),
        }
    }
}

impl ImageCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached path for `url`, downloading it first when it is not there.
    ///
    /// The URL must start with `https://`, or the call fails with
    /// [`Error::DisallowedHost`] before any request. There is no host allowlist: a
    /// description image is fetched from whatever host it names. `extra_hosts` is the
    /// scheme test seam ([`crate::Launcher::with_image_hosts`], empty in a shipped
    /// launcher): a host named there may be plain HTTP, so wiremock can serve an image.
    /// Two concurrent calls for one URL make one request: the second waits on the first
    /// and then finds the file.
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
}

/// The image policy: any host, 5 MiB, `cache/images/`.
fn policy() -> Policy {
    Policy {
        allowed_hosts: None,
        max_bytes: MAX_BYTES,
        refuse_private: true,
        dir: Root::images_dir,
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

    /// The wiremock server speaks plain HTTP, so its host needs the scheme exception.
    fn local_hosts() -> Vec<String> {
        vec!["127.0.0.1".to_string(), "localhost".to_string()]
    }

    #[tokio::test]
    async fn fetch_allows_any_host_and_writes_under_cache_images() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/shot.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"image bytes".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        // No host allowlist: this host is one the icon cache would refuse outright.
        let url = format!("{}/shot.png", server.uri());
        let images = ImageCache::new();
        let path = images
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect("image fetched");

        assert_eq!(path, root.images_dir().join(cache_file_name(&url)));
        assert_eq!(std::fs::read(&path).expect("read image"), b"image bytes");
    }

    #[tokio::test]
    async fn fetch_reuses_the_cached_file_without_a_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/shot.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"image bytes".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/shot.png", server.uri());
        let http = client();
        let first = ImageCache::new()
            .fetch(&http, &root, &url, &local_hosts())
            .await
            .expect("first fetch");
        let second = ImageCache::new()
            .fetch(&http, &root, &url, &local_hosts())
            .await
            .expect("second fetch");

        assert_eq!(first, second);
        assert_eq!(server.received_requests().await.map(|r| r.len()), Some(1));
    }

    #[tokio::test]
    async fn fetch_is_single_flight_for_one_url() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/shot.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"image bytes".to_vec())
                    .set_delay(Duration::from_millis(300)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/shot.png", server.uri());
        let images = ImageCache::new();
        let http = client();
        let hosts = local_hosts();
        let (first, second) = tokio::join!(
            images.fetch(&http, &root, &url, &hosts),
            images.fetch(&http, &root, &url, &hosts)
        );

        assert_eq!(first.expect("first"), second.expect("second"));
        assert_eq!(server.received_requests().await.map(|r| r.len()), Some(1));
        assert!(
            !images.inner.in_flight(&url),
            "the per-URL lock is dropped once both callers are done"
        );
    }

    #[tokio::test]
    async fn a_body_over_five_mib_is_refused_and_not_stored() {
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
        let err = ImageCache::new()
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect_err("the body is over the cap");

        assert!(
            matches!(err, Error::TooLarge { size, max, .. } if size == MAX_BYTES + 1 && max == MAX_BYTES),
            "{err:?}"
        );
        assert!(!root.images_dir().join(cache_file_name(&url)).exists());
    }

    #[tokio::test]
    async fn a_url_that_is_not_https_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        for url in [
            "http://images.example/shot.png",
            "ftp://images.example/shot.png",
            "images.example/shot.png",
        ] {
            let err = ImageCache::new()
                .fetch(&client(), &root, url, &[])
                .await
                .expect_err("the scheme is not https");
            assert!(
                matches!(err, Error::DisallowedHost { .. }),
                "{url}: {err:?}"
            );
            assert!(!root.images_dir().join(cache_file_name(url)).exists());
        }
    }

    /// A 302 to `location`, served once at `/hop.png`.
    async fn redirect(server: &MockServer, location: &str) {
        Mock::given(method("GET"))
            .and(path_matcher("/hop.png"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", location))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn a_redirect_to_a_plain_http_host_is_refused_and_nothing_is_stored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        redirect(&server, "http://images.example/shot.png").await;

        let url = format!("{}/hop.png", server.uri());
        let err = ImageCache::new()
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect_err("the hop is not https");

        assert!(matches!(err, Error::DisallowedHost { .. }), "{err:?}");
        assert!(!root.images_dir().join(cache_file_name(&url)).exists());
        assert!(!root.images_dir().exists() || dir_is_empty(&root.images_dir()));
    }

    #[tokio::test]
    async fn a_redirect_to_a_private_host_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        redirect(&server, "https://files.internal/shot.png").await;

        // The first URL is allowed by the test seam; the hop is the one refused.
        let url = format!("{}/hop.png", server.uri());
        let err = ImageCache::new()
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect_err("the hop names a private host");

        assert!(matches!(err, Error::DisallowedHost { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_same_host_redirect_is_followed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        redirect(&server, "/shot.png").await;
        Mock::given(method("GET"))
            .and(path_matcher("/shot.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"image bytes".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/hop.png", server.uri());
        let path = ImageCache::new()
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect("the hop stays on the same host");

        assert_eq!(std::fs::read(&path).expect("read image"), b"image bytes");
    }

    #[tokio::test]
    async fn a_redirect_chain_past_the_hop_budget_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let server = MockServer::start().await;
        for hop in 0..crate::http::MAX_REDIRECTS + 2 {
            Mock::given(method("GET"))
                .and(path_matcher(format!("/loop{hop}.png")))
                .respond_with(
                    ResponseTemplate::new(302)
                        .insert_header("location", format!("/loop{}.png", hop + 1)),
                )
                .mount(&server)
                .await;
        }

        let url = format!("{}/loop0.png", server.uri());
        let err = ImageCache::new()
            .fetch(&client(), &root, &url, &local_hosts())
            .await
            .expect_err("the chain runs past the budget");

        assert!(matches!(err, Error::DisallowedHost { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_private_host_is_refused_before_any_request() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        for url in [
            "https://127.0.0.1/shot.png",
            "https://[::1]/shot.png",
            "https://localhost/shot.png",
            "https://build.localhost/shot.png",
            "https://files.internal/shot.png",
            "https://nas.local/shot.png",
        ] {
            let err = ImageCache::new()
                .fetch(&client(), &root, url, &[])
                .await
                .expect_err("the host is private");
            assert!(
                matches!(err, Error::DisallowedHost { .. }),
                "{url}: {err:?}"
            );
            assert!(!root.images_dir().join(cache_file_name(url)).exists());
        }
    }

    /// Whether a directory holds nothing, for the "nothing was stored" assertions.
    fn dir_is_empty(dir: &std::path::Path) -> bool {
        std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
    }
}
