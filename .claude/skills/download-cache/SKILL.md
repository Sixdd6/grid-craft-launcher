---
name: download-cache
description: Content-addressed download cache, parallel queue, hash and size verification, retries, progress events, and dedupe across sources. Read before touching gcl-core download/ or http/.
---

## HttpClient

One `reqwest::Client` with `USER_AGENT`, rustls, gzip, 30 s connect timeout, no overall timeout
(large jars). `get_json<T>`, `get_bytes`, `stream_to_file`. Retries: 3 attempts with 500 ms, 2 s,
8 s backoff on connect errors, 5xx, and 429 (honor `Retry-After` and Modrinth `X-Ratelimit-Reset`).
The client holds no base URL; callers pass full URLs. Source clients own their base URL.

## Cache

- Every downloaded file lands in `cache/objects/<sha1[0..2]>/<sha1>` first, then is hard-linked (or copied) to its final path. Libraries and assets keep their own trees but are hard links into `objects/`.
- `DownloadSpec { url, sha1: Option<String>, size: Option<u64>, dest: PathBuf, label }`.
- Before downloading: if `sha1` is known and `objects/<sha1>` exists, link and return.
- After downloading: compute sha1 while streaming. Mismatch → delete, retry. Third mismatch → `Error::HashMismatch { url, expected, actual }`. No sha1 → check size when known.
- Write to `<object>.<uuid>.part` and rename on success. The uuid is unique per attempt, so
  concurrent or retried downloads of the same object never collide on one staging file. A
  download with no known sha1 (size-only verification) stages under `cache/objects/tmp/<uuid>.part`
  instead of beside a named object. `cleanup_partials` sweeps every `*.part` file under all of
  `cache/`, not just `cache/objects/`.

## Queue

`download_all(specs, sink, cancel)` runs up to `config.parallel_downloads` (default 8) at once
with a `Semaphore`. Emits `Event::Progress { task_id, label, done_bytes, total_bytes }` per
file and a summary task for the batch. Stops at the first hard error, cancels the rest, returns
that error.

## Dedupe

Modrinth and CurseForge may serve the same jar. The cache key is sha1, so the second source
links the existing object. CurseForge files without sha1 (only md5 or fingerprint) are
downloaded, hashed, then stored under their computed sha1.

## Do not

- Do not hold a whole file in memory; stream.
- Do not download inside a `spawn_blocking`; only hash or unzip there.
- Do not retry on 404 or 403.
