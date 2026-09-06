//! HTTP client with retries, JSON helpers, and streaming downloads.
//!
//! The client holds no base URL: callers pass full URLs. Every request carries
//! [`crate::USER_AGENT`]. See the `download-cache` skill for the retry policy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

/// Errors from HTTP requests and streamed downloads.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request could not be completed at the transport level.
    #[error("request to {url} failed: {source}")]
    Request {
        /// The URL that was requested.
        url: String,
        /// The underlying transport error.
        source: reqwest::Error,
    },
    /// The server answered with a status that is not success and not retryable.
    #[error("{url} returned HTTP {status}")]
    Status {
        /// The URL that was requested.
        url: String,
        /// The HTTP status code.
        status: u16,
    },
    /// The response body was not the JSON the caller asked for.
    #[error("{url}: invalid JSON: {source}")]
    Json {
        /// The URL that was requested.
        url: String,
        /// The underlying parse error.
        source: serde_json::Error,
    },
    /// Writing the streamed body to disk failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// Every attempt failed with a retryable error.
    #[error("retries exhausted for {url}: last error: {last}")]
    RetriesExhausted {
        /// The URL that was requested.
        url: String,
        /// Display form of the final failure.
        last: String,
    },
}

/// What [`HttpClient::stream_to_file`] observed while writing the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamResult {
    /// Lowercase hex sha1 of the bytes written.
    pub sha1: String,
    /// Number of bytes written.
    pub size: u64,
}

/// Connect timeout for every request. There is no overall timeout: jars are large.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound on a server-requested backoff, so a bad header cannot stall a download.
const MAX_SERVER_BACKOFF: Duration = Duration::from_secs(60);

fn default_backoff() -> Vec<Duration> {
    vec![
        Duration::from_millis(500),
        Duration::from_secs(2),
        Duration::from_secs(8),
    ]
}

/// Shared HTTP client: one connection pool, gzip, rustls, and a retry policy.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
    retries: u32,
    backoff: Vec<Duration>,
}

impl HttpClient {
    /// Builds a client with the launcher User-Agent, gzip, and a 30 s connect timeout.
    pub fn new() -> Result<Self, Error> {
        let inner = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .gzip(true)
            .build()
            .map_err(|source| Error::Request {
                url: "<client builder>".to_string(),
                source,
            })?;
        Ok(HttpClient {
            inner,
            retries: 3,
            backoff: default_backoff(),
        })
    }

    /// Sets the total number of attempts per request. Default 3.
    pub fn with_retries(mut self, n: u32) -> Self {
        self.retries = n;
        self
    }

    /// Replaces the backoff schedule between attempts. Tests use zeros.
    pub fn with_backoff(mut self, backoff: Vec<Duration>) -> Self {
        self.backoff = backoff;
        self
    }

    /// Sends a GET with retries and returns the response for any status below 400.
    async fn send(&self, url: &str, headers: &[(&str, &str)]) -> Result<reqwest::Response, Error> {
        let attempts = self.retries.max(1);
        let mut last = String::new();
        for attempt in 0..attempts {
            let mut req = self.inner.get(url);
            for (name, value) in headers {
                req = req.header(*name, *value);
            }
            let delay = match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.as_u16() < 400 {
                        return Ok(resp);
                    }
                    if !is_retryable_status(status.as_u16()) {
                        return Err(Error::Status {
                            url: url.to_string(),
                            status: status.as_u16(),
                        });
                    }
                    last = Error::Status {
                        url: url.to_string(),
                        status: status.as_u16(),
                    }
                    .to_string();
                    self.backoff_for(attempt)
                        .max(server_backoff(resp.headers()))
                }
                Err(source) => {
                    if !is_retryable_error(&source) {
                        return Err(Error::Request {
                            url: url.to_string(),
                            source,
                        });
                    }
                    last = source.to_string();
                    self.backoff_for(attempt)
                }
            };
            if attempt + 1 < attempts && !delay.is_zero() {
                tracing::debug!(url, attempt, ?delay, "retrying request");
                tokio::time::sleep(delay).await;
            }
        }
        Err(Error::RetriesExhausted {
            url: url.to_string(),
            last,
        })
    }

    fn backoff_for(&self, attempt: u32) -> Duration {
        let idx = attempt as usize;
        self.backoff
            .get(idx)
            .or_else(|| self.backoff.last())
            .copied()
            .unwrap_or(Duration::ZERO)
    }

    /// Fetches a URL and parses the body as JSON.
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T, Error> {
        self.get_json_with_headers(url, &[]).await
    }

    /// Fetches a URL with extra request headers and parses the body as JSON.
    pub async fn get_json_with_headers<T: DeserializeOwned>(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<T, Error> {
        let resp = self.send(url, headers).await?;
        let body = resp.bytes().await.map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })?;
        serde_json::from_slice(&body).map_err(|source| Error::Json {
            url: url.to_string(),
            source,
        })
    }

    /// Fetches a URL and returns the whole body in memory. Not for large files.
    pub async fn get_bytes(&self, url: &str) -> Result<bytes::Bytes, Error> {
        let resp = self.send(url, &[]).await?;
        resp.bytes().await.map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })
    }

    /// Fetches text unless the server reports it unchanged; returns the body and new ETag.
    pub async fn get_text_if_changed(
        &self,
        url: &str,
        etag: Option<&str>,
    ) -> Result<Option<(String, Option<String>)>, Error> {
        let headers = match etag {
            Some(tag) => vec![("if-none-match", tag)],
            None => Vec::new(),
        };
        let resp = self.send(url, &headers).await?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        let new_etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = resp.text().await.map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })?;
        Ok(Some((body, new_etag)))
    }

    /// Streams a URL into `dest`, hashing as it goes and reporting bytes written.
    ///
    /// Writes to exactly `dest` (creating parent directories); `.part` naming is the
    /// caller's job. `expected_size` is only used to log a mismatched `Content-Length`.
    /// The partial file is removed if the transfer fails.
    pub async fn stream_to_file(
        &self,
        url: &str,
        dest: &Path,
        expected_size: Option<u64>,
        on_chunk: &mut dyn FnMut(u64),
    ) -> Result<StreamResult, Error> {
        let resp = self.send(url, &[]).await?;
        if let (Some(expected), Some(advertised)) = (expected_size, resp.content_length())
            && expected != advertised
        {
            tracing::debug!(
                url,
                expected,
                advertised,
                "content-length disagrees with spec"
            );
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| Error::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
        }
        match write_stream(resp, url, dest, on_chunk).await {
            Ok(res) => Ok(res),
            Err(err) => {
                let _ = tokio::fs::remove_file(dest).await;
                Err(err)
            }
        }
    }
}

/// Drains the response body into `dest`, hashing and reporting progress.
async fn write_stream(
    resp: reqwest::Response,
    url: &str,
    dest: &Path,
    on_chunk: &mut dyn FnMut(u64),
) -> Result<StreamResult, Error> {
    let io_err = |source: std::io::Error| Error::Io {
        path: dest.to_path_buf(),
        source,
    };
    let mut file = tokio::fs::File::create(dest).await.map_err(io_err)?;
    let mut hasher = Sha1::new();
    let mut size = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })?;
        hasher.update(&chunk);
        size += chunk.len() as u64;
        file.write_all(&chunk).await.map_err(io_err)?;
        on_chunk(size);
    }
    file.flush().await.map_err(io_err)?;
    Ok(StreamResult {
        sha1: hex::encode(hasher.finalize()),
        size,
    })
}

/// True for statuses worth another attempt: 429 and every 5xx.
fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// True for transport failures worth another attempt.
fn is_retryable_error(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout()
}

/// Reads `Retry-After` and `X-Ratelimit-Reset` as seconds, capped, taking the larger.
fn server_backoff(headers: &reqwest::header::HeaderMap) -> Duration {
    let seconds = |name: &str| -> Option<u64> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    let secs = seconds("retry-after")
        .into_iter()
        .chain(seconds("x-ratelimit-reset"))
        .max()
        .unwrap_or(0);
    Duration::from_secs(secs).min(MAX_SERVER_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::USER_AGENT;
    use sha1::Digest;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client() -> HttpClient {
        HttpClient::new()
            .expect("client builds")
            .with_backoff(vec![Duration::ZERO; 3])
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Version {
        id: String,
        release: bool,
    }

    #[tokio::test]
    async fn get_json_parses_body_and_sends_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v.json"))
            .and(header("user-agent", USER_AGENT))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"1.20.1","release":true}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let got: Version = test_client()
            .get_json(&format!("{}/v.json", server.uri()))
            .await
            .expect("request succeeds");
        assert_eq!(
            got,
            Version {
                id: "1.20.1".into(),
                release: true
            }
        );
    }

    #[tokio::test]
    async fn get_json_reports_invalid_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let err = test_client()
            .get_json::<Version>(&format!("{}/v.json", server.uri()))
            .await
            .expect_err("invalid json fails");
        assert!(matches!(err, Error::Json { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn retries_500_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .with_priority(2)
            .mount(&server)
            .await;

        let body = test_client()
            .get_bytes(&format!("{}/thing", server.uri()))
            .await
            .expect("second attempt succeeds");
        assert_eq!(&body[..], br#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn exhausted_retries_report_last_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .expect(3)
            .mount(&server)
            .await;

        let err = test_client()
            .get_bytes(&format!("{}/thing", server.uri()))
            .await
            .expect_err("always 503");
        assert!(matches!(err, Error::RetriesExhausted { .. }), "got {err:?}");
        assert!(err.to_string().contains("503"));
    }

    #[tokio::test]
    async fn status_404_is_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let url = format!("{}/missing", server.uri());
        let err = test_client()
            .get_bytes(&url)
            .await
            .expect_err("404 is fatal");
        match err {
            Error::Status { status, url: u } => {
                assert_eq!(status, 404);
                assert_eq!(u, url);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[tokio::test]
    async fn retries_429_honoring_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("done"))
            .with_priority(2)
            .mount(&server)
            .await;

        let body = test_client()
            .get_bytes(&format!("{}/limited", server.uri()))
            .await
            .expect("retry after 429 succeeds");
        assert_eq!(&body[..], b"done");
    }

    #[tokio::test]
    async fn get_json_with_headers_sends_them() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("x-api-key", "secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"a","release":false}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let got: Version = test_client()
            .get_json_with_headers(&format!("{}/k", server.uri()), &[("x-api-key", "secret")])
            .await
            .expect("header is sent");
        assert_eq!(got.id, "a");
    }

    #[tokio::test]
    async fn get_text_if_changed_returns_none_on_304() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("if-none-match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;

        let got = test_client()
            .get_text_if_changed(&format!("{}/manifest", server.uri()), Some("\"v1\""))
            .await
            .expect("304 is not an error");
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn get_text_if_changed_returns_body_and_etag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"v2\"")
                    .set_body_string("hello"),
            )
            .mount(&server)
            .await;

        let (body, etag) = test_client()
            .get_text_if_changed(&format!("{}/manifest", server.uri()), None)
            .await
            .expect("request succeeds")
            .expect("body changed");
        assert_eq!(body, "hello");
        assert_eq!(etag.as_deref(), Some("\"v2\""));
    }

    #[tokio::test]
    async fn stream_to_file_writes_bytes_and_hashes() {
        let payload = vec![7u8; 40_000];
        let expected = hex::encode(sha1::Sha1::digest(&payload));

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("nested").join("blob.bin");
        let mut seen = Vec::new();
        let mut on_chunk = |done: u64| seen.push(done);
        let res = test_client()
            .stream_to_file(
                &format!("{}/blob.bin", server.uri()),
                &dest,
                Some(payload.len() as u64),
                &mut on_chunk,
            )
            .await
            .expect("stream succeeds");

        assert_eq!(res.sha1, expected);
        assert_eq!(res.size, payload.len() as u64);
        assert_eq!(std::fs::read(&dest).expect("dest readable"), payload);
        assert_eq!(seen.last().copied(), Some(payload.len() as u64));
    }

    #[tokio::test]
    async fn stream_to_file_reports_status_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("blob.bin");
        let err = test_client()
            .stream_to_file(
                &format!("{}/blob.bin", server.uri()),
                &dest,
                None,
                &mut |_| {},
            )
            .await
            .expect_err("404 fails");
        assert!(
            matches!(err, Error::Status { status: 404, .. }),
            "got {err:?}"
        );
        assert!(!dest.exists());
    }
}
