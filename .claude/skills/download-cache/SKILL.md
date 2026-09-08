---
name: download-cache
description: Content-addressed download cache, parallel queue, hash and size verification, retries, progress events, and dedupe across sources. Read before touching gcl-core download/ or http/.
---

## HttpClient

One `reqwest::Client` with `USER_AGENT`, rustls, gzip, 30 s connect timeout, no overall timeout
(large jars). `get_json<T>`, `get_bytes`, `stream_to_file`.
`stream_to_file(url, dest, expected_size, max_bytes, on_chunk)` takes an `Option<u64>` cap: the
transfer stops **inside the chunk loop** with `http::Error::TooLarge` as soon as the next chunk
would run past it, before that chunk is written, so nothing over the cap ever reaches the disk
and a server that streams forever cannot fill it. The partial file is removed either way.
`None` is what a hash-verified download passes.
`HttpClient::with_redirect_guard(guard)` rebuilds the client with
`reqwest::redirect::Policy::custom`: every hop is handed to `guard` and at most
`http::MAX_REDIRECTS` (5) hops are followed. A refused hop, and one past the budget, fail the
request with a redirect error, which `http::Error::is_redirect()` answers for. Retries: 3 attempts with 500 ms, 2 s,
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

## Icons (`download::icons`)

Project icons have no published hash, so `cache/icons/<sha1(url)>.<ext>` is keyed by the URL,
not by the bytes. `IconCache::fetch` refuses a URL whose scheme is not `https` and a
host outside `ALLOWED_HOSTS` (`cdn.modrinth.com`, `media.forgecdn.net`, `edge.forgecdn.net`)
with `Error::DisallowedHost`, before any request. `extra_hosts` is the test seam
(`Launcher::with_icon_hosts`, empty in a shipped launcher): a host named there may be plain
HTTP, so wiremock can serve an icon.

The 2 MiB cap (`MAX_BYTES`) is a streaming cap, not a `Content-Length` check: a host may send
no length, so the transfer is stopped mid-body and the part file deleted with
`Error::TooLarge`. One request per URL at a time; a second caller waits and then finds the
file.

**`cache/icons` has no eviction.** Nothing prunes it — not `cleanup_partials`, which only
sweeps `*.part` files. The directory grows with the number of distinct icon URLs a user
browses. A cap or an age sweep is still to be written.

## Description images (`download::images`)

`ImageCache::fetch` is the icon cache's twin for the images a project description points at:
same URL key (`cache/images/<sha1(url)>.<ext>`), same single request per URL, same
after-the-transfer size check. Two things differ, on purpose:

- **No host allowlist.** A description names whatever image host its author chose, so any
  `https://` URL is fetched. The scheme is the only host rule, and `extra_hosts`
  (`Launcher::with_image_hosts`, empty in a shipped launcher) excuses it for a wiremock host in
  a test — there is no list to extend. This is a deliberate widening over icons, approved in
  `docs/superpowers/specs/2026-09-08-description-rendering-design.md`.
- **A 5 MiB cap** (`images::MAX_BYTES`), not 2 MiB.

Both caches are one type underneath: `download::media::MediaCache::new(policy)` with a
`Policy { allowed_hosts: Option<Vec<String>>, max_bytes, refuse_private, dir }` — `None`
allowlist means any host, and `dir` is the `Root` accessor the file lands under (`icons_dir` or
`images_dir`). `icons.rs` and `images.rs` are the two policies over it; the streaming, staging,
capping, and single-flight code lives in `media.rs` once. `cache/images` has no eviction, the
same gap `cache/icons` carries.

### The three media rules

`media::url_allowed(url, extra_hosts, policy)` is the one check, and it runs twice: before the
first request, and again on **every redirect hop**, through the guarded client the cache builds
with `HttpClient::with_redirect_guard`. An allowlist checked only on the URL a caller passed is
no allowlist: the first host can answer 302 and send the transfer anywhere. A refused hop comes
back as `Error::DisallowedHost`, the same answer the up-front check gives, and so does a chain
past five hops. The guarded client is built on first use and kept while callers pass the same
`extra_hosts`.

1. **Scheme.** `https` only, unless the host is one of `extra_hosts` — the test seam
   (`Launcher::with_icon_hosts` / `with_image_hosts`, empty in a shipped launcher) that lets a
   wiremock server speak plain HTTP.
2. **Host.** On the icon policy, `ALLOWED_HOSTS` plus `extra_hosts`. The image policy has no
   list.
3. **Private hosts** (`Policy.refuse_private`, true for both): every IP literal, v4 and v6,
   plus `localhost`, `*.localhost`, `*.internal`, and `*.local`. A host named in `extra_hosts`
   is exempt, which is the only reason wiremock on `127.0.0.1` still answers in a test.

The URL is parsed with the `url` crate, not split by hand. That is what refuses
`https://evil.example\@cdn.modrinth.com/a.png`: the backslash ends the authority the way a
browser reads it, so the host is `evil.example` and the CDN name is only path text. A
hand-rolled "everything after the last `@`" split reads the same string as the CDN.

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
