//! Content-addressed download cache.
//!
//! Every file lands in `cache/objects/<sha1[0..2]>/<sha1>` first, then is hard-linked
//! (or copied) to its destination. See the `download-cache` skill.

pub mod hash;
pub mod icons;
pub mod images;
pub mod media;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::events::{Event, EventSink, TaskHandle};
use crate::http::HttpClient;
use crate::paths::Root;

/// Number of download attempts before a hash or size mismatch becomes an error.
const ATTEMPTS: u32 = 3;

/// Errors from the download cache.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The downloaded bytes did not hash to the sha1 the caller expected.
    #[error("hash mismatch for {url}: expected {expected}, got {actual}")]
    HashMismatch {
        /// The URL that was downloaded.
        url: String,
        /// The sha1 the caller asked for.
        expected: String,
        /// The sha1 of the bytes received.
        actual: String,
    },
    /// The downloaded byte count did not match the caller's expectation.
    #[error("size mismatch for {url}: expected {expected}, got {actual}")]
    SizeMismatch {
        /// The URL that was downloaded.
        url: String,
        /// The size the caller asked for.
        expected: u64,
        /// The number of bytes received.
        actual: u64,
    },
    /// The URL's host is not one an icon may be fetched from.
    #[error("icon host not allowed: {host} ({url})")]
    DisallowedHost {
        /// The URL that was refused.
        url: String,
        /// The host taken from that URL, empty when it had none.
        host: String,
    },
    /// The body was larger than the caller's cap.
    #[error("{url} is {size} bytes, over the {max} byte cap")]
    TooLarge {
        /// The URL that was downloaded.
        url: String,
        /// The number of bytes received.
        size: u64,
        /// The largest body the caller accepts.
        max: u64,
    },
    /// The cancellation token fired before or between attempts.
    #[error("cancelled")]
    Cancelled,
    /// The transfer itself failed.
    #[error(transparent)]
    Http(#[from] crate::http::Error),
    /// A filesystem operation in the cache failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A path in the app root layout could not be built.
    #[error(transparent)]
    Paths(#[from] crate::paths::Error),
}

/// One file to fetch: where from, how to verify it, and where it belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadSpec {
    /// Full URL to fetch.
    pub url: String,
    /// Expected sha1 as lowercase hex, when the source publishes one.
    pub sha1: Option<String>,
    /// Expected size in bytes, when known.
    pub size: Option<u64>,
    /// Final path the file must appear at.
    pub dest: PathBuf,
    /// Human-readable name used in progress events.
    pub label: String,
}

/// Everything a download needs that is shared across a batch.
#[derive(Debug, Clone, Copy)]
pub struct DownloadCtx<'a> {
    /// Shared HTTP client.
    pub http: &'a HttpClient,
    /// App root, which owns the object store.
    pub root: &'a Root,
    /// Where progress events go.
    pub sink: &'a EventSink,
    /// Checked before every attempt.
    pub cancel: &'a CancellationToken,
    /// Maximum number of files in flight in [`download_all`].
    pub parallel: usize,
}

/// Fetches one file into `spec.dest`, reusing the object store when it can.
#[tracing::instrument(skip(ctx, spec), fields(label = %spec.label))]
pub async fn download_one(ctx: &DownloadCtx<'_>, spec: &DownloadSpec) -> Result<PathBuf, Error> {
    if dest_is_current(ctx, spec).await? {
        return Ok(spec.dest.clone());
    }
    if let Some(sha1) = spec.sha1.as_deref()
        && let Ok(object) = ctx.root.object_path(sha1)
        && object.is_file()
    {
        link_or_copy(&object, &spec.dest).map_err(|source| Error::Io {
            path: spec.dest.clone(),
            source,
        })?;
        return Ok(spec.dest.clone());
    }

    let task = TaskHandle::start(ctx.sink, spec.label.clone(), spec.size);
    match fetch_into_cache(ctx, spec, &task).await {
        Ok(path) => {
            task.finish();
            Ok(path)
        }
        Err(err) => {
            task.fail(err.to_string());
            Err(err)
        }
    }
}

/// Downloads `spec`, verifies it, stores the object, and links it into place.
async fn fetch_into_cache(
    ctx: &DownloadCtx<'_>,
    spec: &DownloadSpec,
    task: &TaskHandle,
) -> Result<PathBuf, Error> {
    let mut attempt = 1u32;
    loop {
        if ctx.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let part = staging_path(ctx.root, spec);
        let result = ctx
            .http
            .stream_to_file(&spec.url, &part, spec.size, &mut |done| task.progress(done))
            .await;
        let result = match result {
            Ok(result) => result,
            Err(err) => {
                remove_quietly(&part).await;
                return Err(Error::Http(err));
            }
        };

        match verify(spec, &result) {
            Ok(()) => {
                let object = ctx.root.object_path(&result.sha1)?;
                store_object(&part, &object).await?;
                link_or_copy(&object, &spec.dest).map_err(|source| Error::Io {
                    path: spec.dest.clone(),
                    source,
                })?;
                return Ok(spec.dest.clone());
            }
            Err(err) => {
                remove_quietly(&part).await;
                if attempt >= ATTEMPTS {
                    return Err(err);
                }
                let _ = ctx.sink.send(Event::Warning(format!(
                    "{}: {err}; retrying ({attempt}/{ATTEMPTS})",
                    spec.label
                )));
                attempt += 1;
            }
        }
    }
}

/// Builds a staging path for one attempt, unique so concurrent attempts cannot collide.
///
/// A known sha1 stages beside its object; an unknown one stages under `objects/tmp`.
/// Both live inside `cache/`, so [`cleanup_partials`] sweeps whatever is abandoned.
fn staging_path(root: &Root, spec: &DownloadSpec) -> PathBuf {
    let id = uuid::Uuid::new_v4();
    match spec.sha1.as_deref().and_then(|s| root.object_path(s).ok()) {
        Some(object) => {
            let mut name = object.into_os_string();
            name.push(format!(".{id}.part"));
            PathBuf::from(name)
        }
        None => root.objects_dir().join("tmp").join(format!("{id}.part")),
    }
}

/// Compares a finished transfer against the spec's sha1, or its size when no sha1 is known.
fn verify(spec: &DownloadSpec, result: &crate::http::StreamResult) -> Result<(), Error> {
    if let Some(expected) = spec.sha1.as_deref() {
        if !expected.eq_ignore_ascii_case(&result.sha1) {
            return Err(Error::HashMismatch {
                url: spec.url.clone(),
                expected: expected.to_string(),
                actual: result.sha1.clone(),
            });
        }
    } else if let Some(expected) = spec.size
        && expected != result.size
    {
        return Err(Error::SizeMismatch {
            url: spec.url.clone(),
            expected,
            actual: result.size,
        });
    }
    Ok(())
}

/// True when `dest` already holds the file the spec asks for.
///
/// A known sha1 is answered without hashing whenever `dest` is the same file as the object in
/// the store: on unix that is the same `(dev, ino)` pair, elsewhere the same length. Anything
/// else falls back to hashing `dest`.
async fn dest_is_current(ctx: &DownloadCtx<'_>, spec: &DownloadSpec) -> Result<bool, Error> {
    let Ok(meta) = tokio::fs::metadata(&spec.dest).await else {
        return Ok(false);
    };
    if !meta.is_file() {
        return Ok(false);
    }
    if let Some(expected) = spec.sha1.clone() {
        if let Ok(object) = ctx.root.object_path(&expected)
            && let Ok(object_meta) = tokio::fs::metadata(&object).await
            && object_meta.is_file()
            && same_file(&meta, &object_meta)
        {
            return Ok(true);
        }
        let actual = hash_dest(&spec.dest).await?;
        return Ok(expected.eq_ignore_ascii_case(&actual));
    }
    Ok(spec.size == Some(meta.len()))
}

/// Counts every [`hash_dest`] call, so a test can prove the cheap path skipped hashing.
#[cfg(test)]
pub(crate) static HASH_CALLS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Hashes an existing destination file on a blocking thread.
async fn hash_dest(dest: &Path) -> Result<String, Error> {
    #[cfg(test)]
    HASH_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = dest.to_path_buf();
    tokio::task::spawn_blocking(move || hash::sha1_file(&path))
        .await
        .map_err(|source| Error::Io {
            path: dest.to_path_buf(),
            source: std::io::Error::other(source),
        })?
        .map_err(|source| Error::Io {
            path: dest.to_path_buf(),
            source,
        })
}

/// True when two metadata values describe the same file on disk.
#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// True when two metadata values describe files of the same length.
///
/// Windows has no cheap inode to compare, so a matching length plus a matching object in the
/// store is as far as the cheap path goes.
#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.len() == b.len()
}

/// Moves a finished `.part` file to its object path, copying across filesystems.
async fn store_object(part: &Path, object: &Path) -> Result<(), Error> {
    if let Some(parent) = object.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    if tokio::fs::rename(part, object).await.is_ok() {
        return Ok(());
    }
    tokio::fs::copy(part, object)
        .await
        .map_err(|source| Error::Io {
            path: object.to_path_buf(),
            source,
        })?;
    remove_quietly(part).await;
    Ok(())
}

/// Fetches every spec, at most `ctx.parallel` at a time, stopping at the first error.
///
/// The remaining transfers are dropped when one fails; their `.part` files are swept by
/// [`cleanup_partials`] on the next start.
#[tracing::instrument(skip(ctx, specs), fields(files = specs.len()))]
pub async fn download_all(ctx: &DownloadCtx<'_>, specs: Vec<DownloadSpec>) -> Result<(), Error> {
    let specs = dedupe(specs);
    if specs.is_empty() {
        return Ok(());
    }
    let total: Option<u64> = specs
        .iter()
        .try_fold(0u64, |acc, spec| spec.size.and_then(|s| acc.checked_add(s)));
    let batch = TaskHandle::start(ctx.sink, format!("{} files", specs.len()), total);

    let permits = Arc::new(Semaphore::new(ctx.parallel.max(1)));
    let mut running = FuturesUnordered::new();
    for spec in &specs {
        let permits = Arc::clone(&permits);
        running.push(async move {
            let _permit = permits.acquire().await;
            download_one(ctx, spec)
                .await
                .map(|_| spec.size.unwrap_or(0))
        });
    }

    let mut done = 0u64;
    while let Some(result) = running.next().await {
        match result {
            Ok(size) => {
                done += size;
                batch.progress(done);
            }
            Err(err) => {
                drop(running);
                batch.fail(err.to_string());
                return Err(err);
            }
        }
    }
    batch.finish();
    Ok(())
}

/// Drops later specs that repeat an earlier `(sha1, dest)` pair, keeping the first.
fn dedupe(specs: Vec<DownloadSpec>) -> Vec<DownloadSpec> {
    let mut seen = HashSet::new();
    specs
        .into_iter()
        .filter(|spec| seen.insert((spec.sha1.clone(), spec.dest.clone())))
        .collect()
}

/// Hard-links `src` to `dest`, falling back to a copy across filesystems.
pub fn link_or_copy(src: &Path, dest: &Path) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    match std::fs::hard_link(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::copy(src, dest).map(|_| ()),
    }
}

/// Deletes every `*.part` file under the cache directory and returns how many went.
pub fn cleanup_partials(root: &Root) -> std::io::Result<usize> {
    fn sweep(dir: &Path, removed: &mut usize) -> std::io::Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        };
        for entry in entries {
            let path = entry?.path();
            if path.is_dir() {
                sweep(&path, removed)?;
            } else if path.extension().is_some_and(|ext| ext == "part") {
                std::fs::remove_file(&path)?;
                *removed += 1;
            }
        }
        Ok(())
    }
    let mut removed = 0;
    sweep(&root.cache_dir(), &mut removed)?;
    Ok(removed)
}

/// Deletes a file, ignoring the error when it is already gone.
async fn remove_quietly(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Event, TaskId};
    use crate::http::HttpClient;
    use crate::paths::Root;
    use sha1::Digest;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client() -> HttpClient {
        HttpClient::new()
            .expect("client builds")
            .with_backoff(vec![Duration::ZERO; 3])
    }

    fn sha1_of(bytes: &[u8]) -> String {
        hex::encode(sha1::Sha1::digest(bytes))
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: Root,
        http: HttpClient,
        sink: crate::events::EventSink,
        cancel: CancellationToken,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        Fixture {
            _dir: dir,
            root,
            http: client(),
            sink: crate::events::null_sink(),
            cancel: CancellationToken::new(),
        }
    }

    impl Fixture {
        fn ctx(&self, parallel: usize) -> DownloadCtx<'_> {
            DownloadCtx {
                http: &self.http,
                root: &self.root,
                sink: &self.sink,
                cancel: &self.cancel,
                parallel,
            }
        }
    }

    #[tokio::test]
    async fn a_second_download_of_the_same_spec_neither_requests_nor_hashes() {
        let payload = b"a jar of bytes".to_vec();
        let sha1 = sha1_of(&payload);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/lib.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let f = fixture();
        let ctx = f.ctx(2);
        let spec = DownloadSpec {
            url: format!("{}/lib.jar", server.uri()),
            sha1: Some(sha1),
            size: Some(payload.len() as u64),
            dest: f.root.path().join("a").join("lib.jar"),
            label: "lib.jar".into(),
        };
        download_one(&ctx, &spec).await.expect("first download");

        HASH_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        let again = download_one(&ctx, &spec).await.expect("second download");
        assert_eq!(again, spec.dest);
        assert_eq!(
            HASH_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the hard link to the stored object must answer without hashing"
        );
        // The mock allows one request, so `verify` proves nothing was fetched again.
        server.verify().await;
    }

    #[tokio::test]
    async fn known_sha1_lands_in_objects_and_dest_then_reuses_object() {
        let payload = b"a jar of bytes".to_vec();
        let sha1 = sha1_of(&payload);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/lib.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let f = fixture();
        let ctx = f.ctx(2);
        let url = format!("{}/lib.jar", server.uri());
        let first = f.root.path().join("a").join("lib.jar");
        let got = download_one(
            &ctx,
            &DownloadSpec {
                url: url.clone(),
                sha1: Some(sha1.clone()),
                size: Some(payload.len() as u64),
                dest: first.clone(),
                label: "lib.jar".into(),
            },
        )
        .await
        .expect("first download");

        assert_eq!(got, first);
        assert_eq!(std::fs::read(&first).expect("dest"), payload);
        let object = f.root.object_path(&sha1).expect("object path");
        assert!(object.is_file(), "{object:?} missing");
        assert_eq!(std::fs::read(&object).expect("object"), payload);

        // Second destination is served from the object store: the mock allows one request.
        let second = f.root.path().join("b").join("lib.jar");
        download_one(
            &ctx,
            &DownloadSpec {
                url,
                sha1: Some(sha1),
                size: None,
                dest: second.clone(),
                label: "lib.jar".into(),
            },
        )
        .await
        .expect("second download");
        assert_eq!(std::fs::read(&second).expect("dest"), payload);
    }

    #[tokio::test]
    async fn existing_dest_with_matching_sha1_makes_no_request() {
        let payload = b"already here".to_vec();
        let sha1 = sha1_of(&payload);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let f = fixture();
        let dest = f.root.path().join("have.jar");
        std::fs::write(&dest, &payload).expect("plant dest");
        let got = download_one(
            &f.ctx(1),
            &DownloadSpec {
                url: format!("{}/have.jar", server.uri()),
                sha1: Some(sha1),
                size: None,
                dest: dest.clone(),
                label: "have.jar".into(),
            },
        )
        .await
        .expect("cache hit");
        assert_eq!(got, dest);
    }

    #[tokio::test]
    async fn wrong_sha1_retries_three_times_then_fails_leaving_no_part() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"wrong bytes".to_vec()))
            .expect(3)
            .mount(&server)
            .await;

        let expected = sha1_of(b"right bytes");
        let f = fixture();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = DownloadCtx {
            http: &f.http,
            root: &f.root,
            sink: &tx,
            cancel: &f.cancel,
            parallel: 1,
        };
        let dest = f.root.path().join("bad.jar");
        let err = download_one(
            &ctx,
            &DownloadSpec {
                url: format!("{}/bad.jar", server.uri()),
                sha1: Some(expected.clone()),
                size: None,
                dest: dest.clone(),
                label: "bad.jar".into(),
            },
        )
        .await
        .expect_err("hash never matches");

        match err {
            Error::HashMismatch {
                expected: e,
                actual,
                ..
            } => {
                assert_eq!(e, expected);
                assert_eq!(actual, sha1_of(b"wrong bytes"));
            }
            other => panic!("got {other:?}"),
        }
        assert!(!dest.exists());
        assert_eq!(count_parts(&f.root), 0, "a .part file was left behind");

        let mut warnings = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, Event::Warning(_)) {
                warnings += 1;
            }
        }
        assert_eq!(warnings, 2, "one warning per retried mismatch");
    }

    #[tokio::test]
    async fn size_only_spec_stores_object_under_computed_sha1() {
        let payload = vec![9u8; 1024];
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let f = fixture();
        let dest = f.root.path().join("sized.bin");
        download_one(
            &f.ctx(1),
            &DownloadSpec {
                url: format!("{}/sized.bin", server.uri()),
                sha1: None,
                size: Some(payload.len() as u64),
                dest: dest.clone(),
                label: "sized.bin".into(),
            },
        )
        .await
        .expect("size-only download");

        assert_eq!(std::fs::read(&dest).expect("dest"), payload);
        assert!(
            f.root
                .object_path(&sha1_of(&payload))
                .expect("object path")
                .is_file()
        );
        assert_eq!(count_parts(&f.root), 0);
    }

    #[tokio::test]
    async fn size_mismatch_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"12345".to_vec()))
            .mount(&server)
            .await;

        let f = fixture();
        let err = download_one(
            &f.ctx(1),
            &DownloadSpec {
                url: format!("{}/short.bin", server.uri()),
                sha1: None,
                size: Some(99),
                dest: f.root.path().join("short.bin"),
                label: "short.bin".into(),
            },
        )
        .await
        .expect_err("size never matches");
        assert!(
            matches!(
                err,
                Error::SizeMismatch {
                    expected: 99,
                    actual: 5,
                    ..
                }
            ),
            "got {err:?}"
        );
    }

    /// Records when each request arrived and holds the response open for `delay`.
    struct ArrivalRecorder {
        arrivals: Arc<Mutex<Vec<std::time::Instant>>>,
        delay: Duration,
    }

    impl wiremock::Respond for ArrivalRecorder {
        fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
            if let Ok(mut arrivals) = self.arrivals.lock() {
                arrivals.push(std::time::Instant::now());
            }
            let index: u8 = request
                .url
                .path()
                .trim_start_matches("/f")
                .trim_end_matches(".bin")
                .parse()
                .unwrap_or(0);
            ResponseTemplate::new(200)
                .set_delay(self.delay)
                .set_body_bytes(vec![index; 64])
        }
    }

    #[tokio::test]
    async fn download_all_keeps_only_two_files_in_flight() {
        let delay = Duration::from_millis(200);
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ArrivalRecorder {
                arrivals: Arc::clone(&arrivals),
                delay,
            })
            .expect(5)
            .mount(&server)
            .await;

        let f = fixture();
        let specs = (0..5u8)
            .map(|i| DownloadSpec {
                url: format!("{}/f{i}.bin", server.uri()),
                sha1: Some(sha1_of(&[i; 64])),
                size: Some(64),
                dest: f.root.path().join(format!("out/f{i}.bin")),
                label: format!("f{i}.bin"),
            })
            .collect();

        download_all(&f.ctx(2), specs).await.expect("batch");
        for i in 0..5u8 {
            let p = f.root.path().join(format!("out/f{i}.bin"));
            assert_eq!(std::fs::read(&p).expect("dest"), vec![i; 64]);
        }

        let arrivals = arrivals.lock().expect("arrivals").clone();
        assert_eq!(arrivals.len(), 5);
        assert!(
            arrivals[1].duration_since(arrivals[0]) < delay / 2,
            "the second file did not start alongside the first"
        );
        assert!(
            arrivals[2].duration_since(arrivals[0]) >= delay * 3 / 4,
            "a third file started before a permit was free"
        );
    }

    #[tokio::test]
    async fn download_all_returns_the_first_hard_error_and_stops_the_rest() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/gone.bin"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        // Each survivor is slow, so the batch can only finish them by ignoring the 404.
        for i in 0..4u8 {
            Mock::given(method("GET"))
                .and(path_matcher(format!("/ok{i}.bin")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_millis(300))
                        .set_body_bytes(vec![1u8; 8]),
                )
                .expect(0..=1)
                .mount(&server)
                .await;
        }

        let f = fixture();
        let mut specs = vec![DownloadSpec {
            url: format!("{}/gone.bin", server.uri()),
            sha1: None,
            size: Some(8),
            dest: f.root.path().join("gone.bin"),
            label: "gone.bin".into(),
        }];
        for i in 0..4u8 {
            specs.push(DownloadSpec {
                url: format!("{}/ok{i}.bin", server.uri()),
                sha1: None,
                size: Some(8),
                dest: f.root.path().join(format!("ok{i}.bin")),
                label: format!("ok{i}.bin"),
            });
        }

        let err = download_all(&f.ctx(2), specs)
            .await
            .expect_err("404 fails the batch");
        assert!(
            matches!(
                err,
                Error::Http(crate::http::Error::Status { status: 404, .. })
            ),
            "got {err:?}"
        );
        let landed = (0..4u8)
            .filter(|i| f.root.path().join(format!("ok{i}.bin")).exists())
            .count();
        assert_eq!(landed, 0, "downloads kept running after the batch failed");
    }

    #[tokio::test]
    async fn download_all_drops_repeated_specs() {
        let payload = b"shared jar".to_vec();
        let sha1 = sha1_of(&payload);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let f = fixture();
        let dest = f.root.path().join("only.jar");
        let spec = DownloadSpec {
            url: format!("{}/only.jar", server.uri()),
            sha1: Some(sha1),
            size: Some(payload.len() as u64),
            dest: dest.clone(),
            label: "only.jar".into(),
        };
        download_all(&f.ctx(4), vec![spec.clone(), spec.clone(), spec])
            .await
            .expect("batch");
        assert_eq!(std::fs::read(&dest).expect("dest"), payload);
    }

    #[tokio::test]
    async fn same_hash_to_two_destinations_never_corrupts_the_object() {
        let payload = vec![42u8; 128 * 1024];
        let sha1 = sha1_of(&payload);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .expect(1..=2)
            .mount(&server)
            .await;

        let f = fixture();
        let url = format!("{}/shared.jar", server.uri());
        let specs = vec![
            DownloadSpec {
                url: url.clone(),
                sha1: Some(sha1.clone()),
                size: Some(payload.len() as u64),
                dest: f.root.path().join("one/shared.jar"),
                label: "shared.jar".into(),
            },
            DownloadSpec {
                url,
                sha1: Some(sha1.clone()),
                size: Some(payload.len() as u64),
                dest: f.root.path().join("two/shared.jar"),
                label: "shared.jar".into(),
            },
        ];

        download_all(&f.ctx(2), specs).await.expect("batch");
        assert_eq!(
            std::fs::read(f.root.path().join("one/shared.jar")).expect("first dest"),
            payload
        );
        assert_eq!(
            std::fs::read(f.root.path().join("two/shared.jar")).expect("second dest"),
            payload
        );
        assert_eq!(
            std::fs::read(f.root.object_path(&sha1).expect("object path")).expect("object"),
            payload
        );
        assert_eq!(count_parts(&f.root), 0);
    }

    #[tokio::test]
    async fn cancelled_token_stops_before_the_first_attempt() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 4]))
            .expect(0)
            .mount(&server)
            .await;

        let f = fixture();
        f.cancel.cancel();
        let err = download_one(
            &f.ctx(1),
            &DownloadSpec {
                url: format!("{}/x.bin", server.uri()),
                sha1: None,
                size: Some(4),
                dest: f.root.path().join("x.bin"),
                label: "x.bin".into(),
            },
        )
        .await
        .expect_err("cancelled");
        assert!(matches!(err, Error::Cancelled), "got {err:?}");
    }

    #[test]
    fn staging_stays_inside_the_cache_and_is_unique_per_attempt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        let mut spec = DownloadSpec {
            url: "http://example.invalid/x.bin".into(),
            sha1: None,
            size: Some(4),
            dest: dir.path().join("outside/x.bin"),
            label: "x.bin".into(),
        };

        let sized = staging_path(&root, &spec);
        assert!(
            sized.starts_with(root.cache_dir()),
            "{sized:?} escapes the cache, so cleanup_partials would miss it"
        );
        assert_ne!(sized, staging_path(&root, &spec));

        spec.sha1 = Some("abcdef0123456789abcdef0123456789abcdef01".into());
        let hashed = staging_path(&root, &spec);
        assert!(hashed.starts_with(root.objects_dir()));
        assert_ne!(hashed, staging_path(&root, &spec));
        assert!(hashed.extension().is_some_and(|e| e == "part"));

        // A sha1 that is not a hash cannot steer the staging path out of the cache.
        spec.sha1 = Some("../../escape".into());
        let bogus = staging_path(&root, &spec);
        assert!(
            bogus.starts_with(root.objects_dir().join("tmp")),
            "{bogus:?} escaped the object store"
        );
    }

    #[test]
    fn cleanup_partials_removes_planted_part_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        let keep = root.libraries_dir().join("keep.jar");
        std::fs::write(&keep, b"keep").expect("write");
        std::fs::write(root.libraries_dir().join("a.jar.part"), b"x").expect("write");
        let nested = root.object_path(&"ab12".repeat(10)).expect("object path");
        std::fs::create_dir_all(nested.parent().expect("parent")).expect("mkdir");
        std::fs::write(nested.with_extension("part"), b"y").expect("write");

        assert_eq!(cleanup_partials(&root).expect("cleanup"), 2);
        assert!(keep.is_file(), "non-part file was removed");
        assert_eq!(cleanup_partials(&root).expect("cleanup again"), 0);
    }

    #[test]
    fn link_or_copy_creates_parents_and_replaces_dest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src.bin");
        std::fs::write(&src, b"payload").expect("write");
        let other = dir.path().join("other.bin");
        std::fs::write(&other, b"replacement").expect("write");
        let dest = dir.path().join("deep/nest/dest.bin");
        link_or_copy(&src, &dest).expect("first link");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"payload");
        link_or_copy(&other, &dest).expect("relink over existing dest");
        assert_eq!(std::fs::read(&dest).expect("dest"), b"replacement");
        assert_eq!(std::fs::read(&src).expect("src"), b"payload");
    }

    #[tokio::test]
    async fn task_events_are_emitted_for_a_download() {
        let payload = vec![5u8; 256];
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .mount(&server)
            .await;

        let f = fixture();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = DownloadCtx {
            http: &f.http,
            root: &f.root,
            sink: &tx,
            cancel: &f.cancel,
            parallel: 1,
        };
        download_one(
            &ctx,
            &DownloadSpec {
                url: format!("{}/e.bin", server.uri()),
                sha1: Some(sha1_of(&payload)),
                size: Some(256),
                dest: f.root.path().join("e.bin"),
                label: "e.bin".into(),
            },
        )
        .await
        .expect("download");
        drop(tx);

        let mut started: Option<TaskId> = None;
        let mut finished = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                Event::TaskStarted { id, label, .. } => {
                    assert_eq!(label, "e.bin");
                    started = Some(id);
                }
                Event::TaskFinished { id } => {
                    assert_eq!(Some(id), started);
                    finished = true;
                }
                _ => {}
            }
        }
        assert!(started.is_some() && finished);
    }

    /// Counts `.part` files left anywhere under the root.
    fn count_parts(root: &Root) -> usize {
        fn walk(dir: &std::path::Path, n: &mut usize) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, n);
                } else if p.extension().is_some_and(|e| e == "part") {
                    *n += 1;
                }
            }
        }
        let mut n = 0;
        walk(root.path(), &mut n);
        n
    }
}
