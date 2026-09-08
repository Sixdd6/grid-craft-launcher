//! HTTP client with retries, JSON helpers, and streaming downloads.
//!
//! The client holds no base URL: callers pass full URLs. Every request carries
//! [`crate::USER_AGENT`]. See the `download-cache` skill for the retry policy.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;
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
    /// The body ran past the cap the caller set, so the transfer was stopped.
    #[error("{url} sent more than {max} bytes")]
    TooLarge {
        /// The URL that was requested.
        url: String,
        /// Bytes received when the transfer was stopped.
        size: u64,
        /// The cap the caller set.
        max: u64,
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

impl Error {
    /// Whether the request failed because a redirect was refused or the chain ran too
    /// long. A guarded client ([`HttpClient::with_redirect_guard`]) answers this way when
    /// a hop names a host or a scheme the caller does not allow.
    #[must_use]
    pub fn is_redirect(&self) -> bool {
        matches!(self, Error::Request { source, .. } if source.is_redirect())
    }
}

/// What [`HttpClient::stream_to_file`] observed while writing the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamResult {
    /// Lowercase hex sha1 of the bytes written.
    pub sha1: String,
    /// Number of bytes written.
    pub size: u64,
}

/// Most redirect hops a guarded client follows. Every hop is checked by the guard.
pub const MAX_REDIRECTS: usize = 5;

/// Why [`HttpClient::with_redirect_guard`] stopped a redirect chain.
#[derive(Debug, thiserror::Error)]
enum RedirectRefused {
    /// The hop's URL is not one the guard allows.
    #[error("redirect to {0} is not allowed")]
    Disallowed(String),
    /// The chain ran past [`MAX_REDIRECTS`].
    #[error("too many redirects")]
    TooManyHops,
}

/// Connect timeout for every request. There is no overall timeout: jars are large.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound on a server-requested backoff, so a bad header cannot stall a download.
const MAX_SERVER_BACKOFF: Duration = Duration::from_secs(60);

/// Content type for JSON request bodies.
const JSON_CONTENT_TYPE: &str = "application/json";

/// Content type OAuth token endpoints expect.
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

/// Bytes left alone in a form field: the URL unreserved set. Everything else is escaped.
const FORM_FIELD: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Encodes `form` as `application/x-www-form-urlencoded`, percent-escaping both sides.
fn encode_form(form: &[(&str, &str)]) -> String {
    form.iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(name, FORM_FIELD),
                utf8_percent_encode(value, FORM_FIELD)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

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

    /// Rebuilds the client so every redirect hop is checked by `guard` before it is
    /// followed, with at most [`MAX_REDIRECTS`] hops.
    ///
    /// A hop `guard` refuses, and one past the hop budget, fail the request with a
    /// redirect error, which [`Error::is_redirect`] answers for. Retries and backoff carry
    /// over from `self`. The media caches use this: an allowlist that is only checked on
    /// the URL a caller passed is no allowlist at all, since the first host may answer 302
    /// and send the transfer anywhere.
    pub fn with_redirect_guard(
        mut self,
        guard: Arc<dyn Fn(&reqwest::Url) -> bool + Send + Sync>,
    ) -> Result<Self, Error> {
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error(RedirectRefused::TooManyHops);
            }
            if guard(attempt.url()) {
                return attempt.follow();
            }
            let refused = RedirectRefused::Disallowed(attempt.url().to_string());
            attempt.error(refused)
        });
        self.inner = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .gzip(true)
            .redirect(policy)
            .build()
            .map_err(|source| Error::Request {
                url: "<client builder>".to_string(),
                source,
            })?;
        Ok(self)
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
        self.send_method(reqwest::Method::GET, url, headers, None, false)
            .await
    }

    /// Sends a request with retries and returns the response for any status below 400.
    ///
    /// `body`, when set, is a content type and the request body bytes. The body is kept as
    /// bytes rather than a `reqwest::Body` so every retry can send it again.
    ///
    /// With `allow_client_errors`, a 4xx that is not retryable is returned to the caller
    /// instead of becoming [`Error::Status`]. 429 and 5xx are still retried either way.
    async fn send_method(
        &self,
        method: reqwest::Method,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<(&str, &[u8])>,
        allow_client_errors: bool,
    ) -> Result<reqwest::Response, Error> {
        let attempts = self.retries.max(1);
        let mut last = String::new();
        for attempt in 0..attempts {
            let mut req = self.inner.request(method.clone(), url);
            for (name, value) in headers {
                req = req.header(*name, *value);
            }
            if let Some((content_type, bytes)) = body {
                req = req
                    .header(reqwest::header::CONTENT_TYPE, content_type)
                    .body(bytes.to_vec());
            }
            let delay = match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.as_u16() < 400 {
                        return Ok(resp);
                    }
                    if !is_retryable_status(status.as_u16()) {
                        if allow_client_errors && status.as_u16() < 500 {
                            return Ok(resp);
                        }
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

    /// POSTs `body` as JSON and parses the response body as JSON.
    pub async fn post_json<T: DeserializeOwned, B: Serialize>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T, Error> {
        self.post_json_with_headers(url, body, &[]).await
    }

    /// POSTs `body` as JSON with extra request headers and parses the response body as JSON.
    ///
    /// Retries follow the same policy as GET: 429 and 5xx are retried, everything else fails.
    pub async fn post_json_with_headers<T: DeserializeOwned, B: Serialize>(
        &self,
        url: &str,
        body: &B,
        headers: &[(&str, &str)],
    ) -> Result<T, Error> {
        let payload = serde_json::to_vec(body).map_err(|source| Error::Json {
            url: url.to_string(),
            source,
        })?;
        let resp = self
            .send_method(
                reqwest::Method::POST,
                url,
                headers,
                Some((JSON_CONTENT_TYPE, &payload)),
                false,
            )
            .await?;
        let body = resp.bytes().await.map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })?;
        serde_json::from_slice(&body).map_err(|source| Error::Json {
            url: url.to_string(),
            source,
        })
    }

    /// POSTs `body` as JSON with extra headers and returns the status and the raw body.
    ///
    /// A 4xx is not an error here: the caller reads the body for the service's own error
    /// shape, such as the XSTS `XErr` code. 429 and 5xx are still retried.
    pub async fn post_json_raw_with_headers<B: Serialize>(
        &self,
        url: &str,
        body: &B,
        headers: &[(&str, &str)],
    ) -> Result<(u16, bytes::Bytes), Error> {
        let payload = serde_json::to_vec(body).map_err(|source| Error::Json {
            url: url.to_string(),
            source,
        })?;
        let resp = self
            .send_method(
                reqwest::Method::POST,
                url,
                headers,
                Some((JSON_CONTENT_TYPE, &payload)),
                true,
            )
            .await?;
        let status = resp.status().as_u16();
        let body = resp.bytes().await.map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })?;
        Ok((status, body))
    }

    /// POSTs `form` as `application/x-www-form-urlencoded` and parses the answer as JSON.
    ///
    /// Retries follow the same policy as GET: 429 and 5xx are retried, everything else fails.
    /// OAuth reports `authorization_pending` with HTTP 400, so a caller that needs the body of
    /// a 4xx uses [`HttpClient::post_form_raw`] instead.
    pub async fn post_form<T: DeserializeOwned>(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<T, Error> {
        let (_status, body) = self.send_form(url, form, false).await?;
        serde_json::from_slice(&body).map_err(|source| Error::Json {
            url: url.to_string(),
            source,
        })
    }

    /// POSTs `form` as `application/x-www-form-urlencoded` and returns the status and body.
    ///
    /// A 4xx is not an error here: the caller reads the body to tell `authorization_pending`
    /// from a real failure. 429 and 5xx are still retried.
    pub async fn post_form_raw(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> Result<(u16, bytes::Bytes), Error> {
        self.send_form(url, form, true).await
    }

    /// Shared body of [`HttpClient::post_form`] and [`HttpClient::post_form_raw`].
    async fn send_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        allow_client_errors: bool,
    ) -> Result<(u16, bytes::Bytes), Error> {
        let payload = encode_form(form);
        let resp = self
            .send_method(
                reqwest::Method::POST,
                url,
                &[],
                Some((FORM_CONTENT_TYPE, payload.as_bytes())),
                allow_client_errors,
            )
            .await?;
        let status = resp.status().as_u16();
        let body = resp.bytes().await.map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })?;
        Ok((status, body))
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
    ///
    /// `max_bytes` is a hard cap on the body: the transfer stops inside the chunk loop
    /// with [`Error::TooLarge`] as soon as the next chunk would run past it, so nothing
    /// larger than the cap is ever written and a server that streams forever cannot fill
    /// the disk. `None` means no cap, which is what a hash-verified download uses.
    pub async fn stream_to_file(
        &self,
        url: &str,
        dest: &Path,
        expected_size: Option<u64>,
        max_bytes: Option<u64>,
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
        match write_stream(resp, url, dest, max_bytes, on_chunk).await {
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
    max_bytes: Option<u64>,
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
        // The cap is checked before the chunk is written, so the file on disk never holds
        // more than the caller allowed, whatever a server sends.
        if let Some(max) = max_bytes
            && size + chunk.len() as u64 > max
        {
            return Err(Error::TooLarge {
                url: url.to_string(),
                size: size + chunk.len() as u64,
                max,
            });
        }
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
    use wiremock::matchers::{body_json, body_string, header, method, path};
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
    async fn post_json_sends_the_body_and_parses_the_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/lookup"))
            .and(header("content-type", "application/json"))
            .and(body_json(serde_json::json!({"hashes": ["aa"]})))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"1.20.1","release":true}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let got: Version = test_client()
            .post_json(
                &format!("{}/lookup", server.uri()),
                &serde_json::json!({"hashes": ["aa"]}),
            )
            .await
            .expect("request succeeds");
        assert_eq!(got.id, "1.20.1");
    }

    #[tokio::test]
    async fn post_json_retries_5xx_and_resends_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_json(serde_json::json!({"hashes": ["aa"]})))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_json(serde_json::json!({"hashes": ["aa"]})))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"a","release":false}"#),
            )
            .with_priority(2)
            .mount(&server)
            .await;

        let got: Version = test_client()
            .post_json(
                &format!("{}/lookup", server.uri()),
                &serde_json::json!({"hashes": ["aa"]}),
            )
            .await
            .expect("second attempt succeeds");
        assert_eq!(got.id, "a");
    }

    #[tokio::test]
    async fn post_form_encodes_the_body_and_parses_the_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string(
                "client_id=abc&scope=XboxLive.signin%20offline_access",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"1.20.1","release":true}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let got: Version = test_client()
            .post_form(
                &format!("{}/token", server.uri()),
                &[
                    ("client_id", "abc"),
                    ("scope", "XboxLive.signin offline_access"),
                ],
            )
            .await
            .expect("request succeeds");
        assert_eq!(got.id, "1.20.1");
    }

    #[tokio::test]
    async fn post_form_fails_on_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"error":"nope"}"#))
            .expect(1)
            .mount(&server)
            .await;

        let err = test_client()
            .post_form::<Version>(&format!("{}/token", server.uri()), &[("a", "b")])
            .await
            .expect_err("400 is fatal for post_form");
        assert!(
            matches!(err, Error::Status { status: 400, .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn post_form_raw_returns_the_body_of_a_400() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":"authorization_pending"}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (status, body) = test_client()
            .post_form_raw(&format!("{}/token", server.uri()), &[("a", "b")])
            .await
            .expect("400 is not an error");
        assert_eq!(status, 400);
        assert_eq!(&body[..], br#"{"error":"authorization_pending"}"#);
    }

    #[tokio::test]
    async fn post_form_raw_still_retries_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("done"))
            .with_priority(2)
            .mount(&server)
            .await;

        let (status, body) = test_client()
            .post_form_raw(&format!("{}/token", server.uri()), &[("a", "b")])
            .await
            .expect("second attempt succeeds");
        assert_eq!(status, 200);
        assert_eq!(&body[..], b"done");
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
                None,
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

    #[tokio::test]
    async fn stream_to_file_removes_partial_when_the_body_ends_early() {
        // A raw listener that promises 4096 bytes, sends 512, then closes.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\n")
                    .await;
                let _ = socket.write_all(&[1u8; 512]).await;
                let _ = socket.flush().await;
            }
        });

        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("cut.bin");
        let mut seen = Vec::new();
        let err = test_client()
            .stream_to_file(
                &format!("http://{addr}/cut.bin"),
                &dest,
                Some(4096),
                None,
                &mut |done| seen.push(done),
            )
            .await
            .expect_err("body ends early");
        assert!(matches!(err, Error::Request { .. }), "got {err:?}");
        assert!(!seen.is_empty(), "the failure was not mid-stream");
        assert!(!dest.exists(), "partial file was left behind");
    }

    #[tokio::test]
    async fn stream_to_file_stops_at_the_byte_cap_mid_stream() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![3u8; 8 * 1024 * 1024]))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("big.bin");
        let cap = 64 * 1024;
        let mut seen = Vec::new();
        let err = test_client()
            .stream_to_file(
                &format!("{}/big.bin", server.uri()),
                &dest,
                None,
                Some(cap),
                &mut |done| seen.push(done),
            )
            .await
            .expect_err("the body runs past the cap");

        assert!(
            matches!(err, Error::TooLarge { size, max, .. } if size > cap && max == cap),
            "got {err:?}"
        );
        assert!(
            seen.iter().all(|done| *done <= cap),
            "wrote past the cap: {seen:?}"
        );
        assert!(!dest.exists(), "the part file was left behind");
    }
}
