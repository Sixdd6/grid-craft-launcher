---
name: testing
description: How to test GRID Craft Launcher — nextest, wiremock fixtures, insta snapshots, CLI tests, and the e2e recipe. Read before writing or running any test.
---

## Commands

- `just check`: fmt, clippy, all tests. Run before reporting done.
- `just test 'test(name)'`: one filter. Nextest filter syntax: `test(substring)`, `package(gcl-core)`.
- `cargo insta review`: accept or reject snapshot changes. Pending snapshots fail CI.

## Unit tests

- Live next to the code in `#[cfg(test)] mod tests`.
- Pure functions first: rule evaluation, argument templating, path building, hash checks.

## Fixtures

- Real responses under `tests/fixtures/<source>/<name>.json`. Record with
  `just record-fixture <source> <name> '<url>'`. Keep them small: trim arrays to two or three items by hand if over 50 KB.
- Parser tests read the fixture and assert key fields. Add an insta snapshot of the parsed struct
  with `insta::assert_json_snapshot!`.

## HTTP mocking

```rust
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

let server = MockServer::start().await;
Mock::given(method("GET")).and(path("/v2/search"))
    .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../../tests/fixtures/modrinth/search-sodium.json")))
    .mount(&server).await;
let client = HttpClient::new();
let source = ModrinthSource::with_base_url(client.clone(), server.uri());
```

`HttpClient` is host-agnostic. Every source client (`ModrinthSource`, `CurseForgeSource`) and the
`mojang` module take a base URL so tests can point them at wiremock.

`Launcher::mojang()` reads `GCL_MOJANG_BASE_URL` (`launcher::MOJANG_BASE_URL_ENV`) and uses it
instead of `piston-meta`. That env var is test-only: set it in CLI tests to point `gcl` at a
wiremock server. Never set it in production or document it as a user setting.

### Fixture URL rewriting

A fixture JSON still has real Mojang hosts baked into its URLs (`piston-meta.mojang.com`,
`libraries.minecraft.net`, `resources.download.minecraft.net`, `launchermeta.mojang.com`).
Before serving a fixture from wiremock, string-replace the host with the mock server's URI so
the code under test fetches from wiremock for every follow-up request, not just the first:

```rust
let server = MockServer::start().await;
let body = MANIFEST.replace("https://piston-meta.mojang.com", &server.uri());
Mock::given(method("GET")).and(path("/mc/game/version_manifest_v2.json"))
    .respond_with(ResponseTemplate::new(200).set_body_string(body))
    .mount(&server).await;
```

See `crates/gcl-cli/tests/cli.rs::mock_mojang` and `crates/gcl-core/src/java/runtime.rs` tests
for worked examples. Rewrite every host the fixture references, not only the one the current
test happens to hit.

### Flaky timing test

`download_all_keeps_only_two_files_in_flight` (in `crates/gcl-core/src/download/mod.rs`)
asserts on wall-clock timing of concurrent transfers. On a loaded machine it can fail spuriously.
Rerun it once before treating a failure as a real regression.

## Filesystem

- Use `tempfile::tempdir()` for a root. Set `GCL_ROOT` when testing the CLI.
- Never touch the real user root in tests.

## CLI tests

`assert_cmd::Command::cargo_bin("gcl")` with `.env("GCL_ROOT", dir.path())`. Assert exit code and
key output lines. Use `--json` and parse with serde_json for structure.

## e2e

`just e2e` runs `scripts/e2e.sh`: temp root, create instance, install Fabric, add Sodium from
Modrinth, dry-run launch, check every classpath jar exists. It uses the network. Only the
e2e-runner agent and humans run it. Set `GCL_E2E_MC_VERSION` to change the version.

## What not to do

- No network in unit or CLI tests.
- No `sleep` to wait for async work; await the future.
- No tests that depend on ordering or shared global state.
