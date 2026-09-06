---
name: rust-conventions
description: Rust rules for GRID Craft Launcher — workspace layout, error types, async rules, logging, dependency list. Read before writing any Rust in gcl-core or gcl-cli.
---

## Layout

- `gcl-core` is a library. One directory per module from `ARCHITECTURE.md`, `mod.rs` documents the public interface.
- `gcl-cli` maps subcommands to `gcl_core::Launcher` methods. One file per top-level subcommand under `src/commands/`.
- No logic in `gcl-ui` beyond model adapters.

## Errors

- Each core module has `pub enum Error` with `thiserror`. Variants carry context: URL and status for network, path for IO.
- A crate-level `gcl_core::Error` wraps module errors with `#[from]`.
- Binaries use `anyhow` and print `{:#}`.
- No `unwrap`, `expect`, or `panic!` outside tests and `build.rs`. Use `?`.

## Async

- One tokio runtime, owned by `Launcher`. Binaries never build one.
- Blocking work (zip, hashing, running Java processors, big file copies) goes through `tokio::task::spawn_blocking`.
- Network code takes `&HttpClient` from `gcl_core::http`, never builds its own `reqwest::Client`.
- Cancellation: long tasks take a `CancellationToken` (tokio-util) and check it between downloads.

## Logging

- `tracing` with `#[instrument(skip(client))]` on public async functions. Spans name the instance or version.
- Never log secrets or tokens. Log URLs without query strings that carry keys.

## Serde

- Remote JSON structs live in the owning module, derive `Deserialize`, use `#[serde(rename_all = "camelCase")]` where the API does, and `#[serde(default)]` on optional arrays.
- Unknown fields are ignored (serde default). Do not add `deny_unknown_fields`.

## Dependencies

Add to `[workspace.dependencies]` first, then `dep.workspace = true` in the crate. Versions
checked 2026-09-06:

| Need | Crate |
|---|---|
| async | tokio 1.53, tokio-util 0.7 (`CancellationToken`) |
| HTTP | reqwest 0.13 with `rustls`, `stream`, `json`, `gzip`; `default-features = false` (0.13 renamed `rustls-tls` to `rustls`) |
| hashing | sha1 0.11, sha2 0.11, md-5 0.11, murmur2 0.1 |
| zip | zip 8 (latest stable, not the 9.0 pre-release) |
| config | serde 1, serde_json 1, toml 1.1 |
| paths | directories 6 |
| secrets | keyring 4 (check feature flags in docs.rs before adding; Linux needs a Secret Service backend) |
| ids | uuid 1.26 with `v3`, `v4`, `serde` |
| RFC 3339 timestamps | time 0.3 with `formatting`, `macros`; add `parsing` to read timestamps back |
| errors | thiserror 2, anyhow 1 |
| logs | tracing 0.1, tracing-subscriber 0.3 (`env-filter`), tracing-appender 0.2 |
| CLI | clap 4.6 derive |
| UI | slint 1.17, slint-build 1.17 |
| tests | cargo-nextest, wiremock 0.6, insta 1.48, tempfile 3, assert_cmd 2, predicates 3 |

Prefer no new dependency when std or an existing one does the job.

## Style

- `cargo fmt` settings in `rustfmt.toml`. Clippy runs with `-D warnings`.
- Public items get a one-line doc comment saying what, not how.
- Prefer small functions and files. Split a module file above 400 lines.
