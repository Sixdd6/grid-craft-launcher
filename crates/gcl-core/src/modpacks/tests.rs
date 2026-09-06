//! Tests for pack detection and for the two manifest parsers.
//!
//! Importing a pack needs a loader host, a Mojang host, and a file host, so it lives in
//! `crates/gcl-core/tests/modpack_import.rs` with the other integration tests.

use std::io::Write;

use super::*;
use crate::instances::model::Loader;

/// The CurseForge manifest fixture, which every CurseForge parse test reads.
const CF_MANIFEST: &str = include_str!("../../../../tests/fixtures/curseforge/manifest.json");

/// Builds an in-memory zip from `(name, bytes)` pairs and writes it into `dir`.
fn write_zip(dir: &std::path::Path, name: &str, entries: &[(&str, &str)]) -> std::path::PathBuf {
    let path = dir.join(name);
    let file = std::fs::File::create(&path).expect("create zip");
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    for (entry, body) in entries {
        zip.start_file(*entry, opts).expect("start entry");
        zip.write_all(body.as_bytes()).expect("write entry");
    }
    zip.finish().expect("finish zip");
    path
}

// ---------------------------------------------------------------------------
// detect
// ---------------------------------------------------------------------------

#[test]
fn detect_finds_an_mrpack_by_its_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = write_zip(
        dir.path(),
        "pack.mrpack",
        &[("modrinth.index.json", "{}"), ("overrides/a.txt", "a")],
    );
    assert_eq!(detect(&zip).expect("detect"), PackFormat::Mrpack);
}

#[test]
fn detect_finds_a_curseforge_pack_by_its_manifest_type() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = write_zip(dir.path(), "pack.zip", &[("manifest.json", CF_MANIFEST)]);
    assert_eq!(detect(&zip).expect("detect"), PackFormat::CurseForge);
}

#[test]
fn detect_rejects_a_manifest_that_is_not_a_modpack() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = write_zip(
        dir.path(),
        "pack.zip",
        &[("manifest.json", r#"{"manifestType":"something-else"}"#)],
    );
    assert!(matches!(detect(&zip), Err(Error::UnknownFormat)));
}

#[test]
fn detect_rejects_a_zip_with_neither_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = write_zip(dir.path(), "plain.zip", &[("readme.txt", "hello")]);
    assert!(matches!(detect(&zip), Err(Error::UnknownFormat)));
}

#[test]
fn detect_ignores_a_manifest_that_is_not_at_the_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = write_zip(
        dir.path(),
        "pack.zip",
        &[("nested/manifest.json", CF_MANIFEST)],
    );
    assert!(matches!(detect(&zip), Err(Error::UnknownFormat)));
}

// ---------------------------------------------------------------------------
// mrpack parse
// ---------------------------------------------------------------------------

/// A hand-written index with one installable file, one unsupported-on-client file, and
/// the two loader keys a valid pack may not have at once left out.
fn index(files: &str, dependencies: &str) -> String {
    format!(
        r#"{{
            "formatVersion": 1,
            "game": "minecraft",
            "name": "Test Pack",
            "versionId": "1.2.3",
            "dependencies": {dependencies},
            "files": [{files}]
        }}"#
    )
}

/// One `files[]` entry with every field the parser reads.
fn mrpack_file(path: &str, url: &str, client: &str) -> String {
    format!(
        r#"{{
            "path": "{path}",
            "hashes": {{ "sha1": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }},
            "env": {{ "client": "{client}", "server": "required" }},
            "downloads": ["{url}"],
            "fileSize": 12
        }}"#
    )
}

const FABRIC_DEPS: &str = r#"{ "minecraft": "1.20.1", "fabric-loader": "0.15.11" }"#;

#[test]
fn mrpack_parse_reads_the_plan_and_skips_an_unsupported_client_file() {
    let files = format!(
        "{}, {}",
        mrpack_file(
            "mods/sodium.jar",
            "https://cdn.modrinth.com/a.jar",
            "required"
        ),
        mrpack_file(
            "mods/server-only.jar",
            "https://cdn.modrinth.com/b.jar",
            "unsupported"
        ),
    );
    let plan = mrpack::parse(&index(&files, FABRIC_DEPS)).expect("parse");

    assert_eq!(plan.name, "Test Pack");
    assert_eq!(plan.version, "1.2.3");
    assert_eq!(plan.minecraft, "1.20.1");
    assert_eq!(plan.loader, Loader::Fabric);
    assert_eq!(plan.loader_version, "0.15.11");
    assert_eq!(plan.overrides, ["overrides/", "client-overrides/"]);
    assert_eq!(plan.files.len(), 1);

    let file = &plan.files[0];
    assert_eq!(file.path.as_deref(), Some("mods/sodium.jar"));
    assert_eq!(file.url.as_deref(), Some("https://cdn.modrinth.com/a.jar"));
    assert_eq!(
        file.sha1.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(file.size, Some(12));
    assert_eq!(file.source, None);
    assert!(file.required);
}

#[test]
fn mrpack_parse_reads_every_loader_key() {
    for (key, loader) in [
        ("fabric-loader", Loader::Fabric),
        ("quilt-loader", Loader::Quilt),
        ("forge", Loader::Forge),
        ("neoforge", Loader::NeoForge),
    ] {
        let deps = format!(r#"{{ "minecraft": "1.20.1", "{key}": "9.9" }}"#);
        let plan = mrpack::parse(&index("", &deps)).expect("parse");
        assert_eq!(plan.loader, loader, "{key}");
        assert_eq!(plan.loader_version, "9.9");
    }
}

#[test]
fn mrpack_parse_rejects_a_disallowed_download_host() {
    let files = mrpack_file(
        "mods/evil.jar",
        "https://evil.example.com/a.jar",
        "required",
    );
    let err = mrpack::parse(&index(&files, FABRIC_DEPS)).expect_err("disallowed");
    assert!(
        matches!(&err, Error::DisallowedHost(host) if host == "evil.example.com"),
        "{err:?}"
    );
}

#[test]
fn mrpack_parse_accepts_every_allowed_host_case_insensitively() {
    for host in mrpack::ALLOWED_HOSTS {
        let url = format!("https://{}/a.jar", host.to_ascii_uppercase());
        let files = mrpack_file("mods/a.jar", &url, "required");
        assert!(
            mrpack::parse(&index(&files, FABRIC_DEPS)).is_ok(),
            "{host} was rejected"
        );
    }
}

#[test]
fn mrpack_parse_rejects_a_path_that_escapes_the_game_directory() {
    for path in ["../evil.jar", "mods/../../evil.jar"] {
        let files = mrpack_file(path, "https://cdn.modrinth.com/a.jar", "required");
        let err = mrpack::parse(&index(&files, FABRIC_DEPS)).expect_err("unsafe");
        assert!(matches!(err, Error::UnsafePath(_)), "{path}: {err:?}");
    }
}

#[test]
fn mrpack_parse_rejects_a_file_without_a_sha1() {
    let json = index(
        r#"{ "path": "mods/a.jar", "downloads": ["https://cdn.modrinth.com/a.jar"] }"#,
        FABRIC_DEPS,
    );
    let err = mrpack::parse(&json).expect_err("no sha1");
    assert!(
        matches!(err, Error::Parse { what: "hashes", .. }),
        "{err:?}"
    );
}

#[test]
fn mrpack_parse_rejects_a_file_with_no_download_url() {
    let json = index(
        r#"{ "path": "mods/a.jar", "hashes": { "sha1": "aa" }, "downloads": [] }"#,
        FABRIC_DEPS,
    );
    let err = mrpack::parse(&json).expect_err("no downloads");
    assert!(
        matches!(
            err,
            Error::Parse {
                what: "downloads",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn mrpack_parse_rejects_dependencies_without_minecraft_or_a_loader() {
    let no_mc = mrpack::parse(&index("", r#"{ "fabric-loader": "1" }"#)).expect_err("no minecraft");
    assert!(
        matches!(&no_mc, Error::Parse { what: "dependencies", detail } if detail == "no minecraft"),
        "{no_mc:?}"
    );

    let no_loader = mrpack::parse(&index("", r#"{ "minecraft": "1.20.1" }"#)).expect_err("none");
    assert!(
        matches!(&no_loader, Error::Parse { what: "dependencies", detail } if detail == "no loader"),
        "{no_loader:?}"
    );

    let two = index(
        "",
        r#"{ "minecraft": "1.20.1", "forge": "1", "neoforge": "2" }"#,
    );
    let err = mrpack::parse(&two).expect_err("two loaders");
    assert!(
        matches!(
            &err,
            Error::Parse {
                what: "dependencies",
                ..
            }
        ),
        "{err:?}"
    );
}

// ---------------------------------------------------------------------------
// CurseForge manifest parse
// ---------------------------------------------------------------------------

#[test]
fn curseforge_parse_reads_the_fixture_manifest() {
    let plan = curseforge::parse(CF_MANIFEST).expect("parse");
    assert_eq!(plan.name, "GRID Test Pack");
    assert_eq!(plan.version, "1.0.0");
    assert_eq!(plan.minecraft, "1.20.1");
    assert_eq!(plan.loader, Loader::Fabric);
    assert_eq!(plan.loader_version, "0.15.11");
    assert_eq!(plan.overrides, ["overrides/"]);
    assert_eq!(plan.files.len(), 3);

    let first = &plan.files[0];
    assert_eq!(first.path, None);
    assert_eq!(first.url, None);
    assert_eq!(
        first.source,
        Some((
            SourceId::CurseForge,
            "394468".to_string(),
            "5230381".to_string()
        ))
    );
    assert!(first.required);
    assert!(!plan.files[1].required);
}

#[test]
fn curseforge_parse_prefers_the_primary_loader() {
    let json = r#"{
        "manifestType": "minecraftModpack",
        "name": "P", "version": "1",
        "minecraft": { "version": "1.21.1", "modLoaders": [
            { "id": "fabric-0.1.0", "primary": false },
            { "id": "neoforge-21.1.65", "primary": true }
        ]},
        "files": []
    }"#;
    let plan = curseforge::parse(json).expect("parse");
    assert_eq!(plan.loader, Loader::NeoForge);
    assert_eq!(plan.loader_version, "21.1.65");
}

#[test]
fn curseforge_parse_falls_back_to_the_first_loader_and_the_default_overrides() {
    let json = r#"{
        "manifestType": "minecraftModpack",
        "name": "P", "version": "1",
        "minecraft": { "version": "1.20.1", "modLoaders": [{ "id": "forge-47.2.0" }] },
        "files": []
    }"#;
    let plan = curseforge::parse(json).expect("parse");
    assert_eq!(plan.loader, Loader::Forge);
    assert_eq!(plan.loader_version, "47.2.0");
    assert_eq!(plan.overrides, ["overrides/"]);
}

#[test]
fn curseforge_parse_rejects_a_bad_manifest() {
    let not_a_pack = r#"{ "manifestType": "other", "minecraft": { "version": "1" } }"#;
    assert!(matches!(
        curseforge::parse(not_a_pack),
        Err(Error::Parse {
            what: "manifestType",
            ..
        })
    ));

    let no_loader = r#"{
        "manifestType": "minecraftModpack", "name": "P",
        "minecraft": { "version": "1.20.1", "modLoaders": [] }, "files": []
    }"#;
    assert!(matches!(
        curseforge::parse(no_loader),
        Err(Error::Parse {
            what: "modLoaders",
            ..
        })
    ));

    let bad_id = r#"{
        "manifestType": "minecraftModpack", "name": "P",
        "minecraft": { "version": "1.20.1", "modLoaders": [{ "id": "banana-1.0" }] }, "files": []
    }"#;
    assert!(matches!(
        curseforge::parse(bad_id),
        Err(Error::Parse {
            what: "modLoaders",
            ..
        })
    ));
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

#[test]
fn host_of_reads_the_host_out_of_a_url() {
    assert_eq!(
        host_of("https://cdn.modrinth.com/a.jar"),
        Some("cdn.modrinth.com")
    );
    assert_eq!(
        host_of("http://user:pw@github.com:8080/a"),
        Some("github.com")
    );
    assert_eq!(host_of("https://[::1]:9/a"), Some("::1"));
    assert_eq!(host_of("not a url"), None);
}

#[test]
fn kind_of_path_maps_only_the_three_recorded_folders() {
    assert_eq!(kind_of_path("mods/a.jar"), Some(ContentKind::Mod));
    assert_eq!(
        kind_of_path("resourcepacks/a.zip"),
        Some(ContentKind::ResourcePack)
    );
    assert_eq!(kind_of_path("shaderpacks/a.zip"), Some(ContentKind::Shader));
    assert_eq!(kind_of_path("config/a.toml"), None);
}

// ---------------------------------------------------------------------------
// fetch_pack and the CurseForge file branch
// ---------------------------------------------------------------------------

mod online {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::*;
    use crate::content::ContentCtx;
    use crate::download::DownloadCtx;
    use crate::download::hash::sha1_hex;
    use crate::events::{Event, EventSink};
    use crate::http::HttpClient;
    use crate::instances::Instances;
    use crate::instances::model::Loader;
    use crate::paths::Root;
    use crate::sources::{BoxSource, curseforge::CurseForge, modrinth::Modrinth};

    const CLASSES: &str =
        include_str!("../../../../tests/fixtures/curseforge/categories_classes.json");

    /// Bytes of the one file every test here downloads.
    const PACK_BYTES: &[u8] = b"pack archive bytes";

    /// CurseForge API key the mock server accepts.
    const KEY: &str = "test-api-key";

    /// Everything the [`ContentCtx`] borrows, kept alive by the caller.
    struct Harness {
        http: HttpClient,
        root: Root,
        sink: EventSink,
        cancel: CancellationToken,
        sources: Vec<BoxSource>,
        _rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn new(sources: Vec<BoxSource>) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = Root::from_path(dir.path());
            root.ensure_layout().expect("layout");
            let (sink, rx) = tokio::sync::mpsc::unbounded_channel();
            Harness {
                http: HttpClient::new().expect("http"),
                root,
                sink,
                cancel: CancellationToken::new(),
                sources,
                _rx: rx,
                _dir: dir,
            }
        }

        fn dl(&self) -> DownloadCtx<'_> {
            DownloadCtx {
                http: &self.http,
                root: &self.root,
                sink: &self.sink,
                cancel: &self.cancel,
                parallel: 4,
            }
        }
    }

    /// Runs `$body` with `$ctx` bound to a `ContentCtx` borrowing `$h`.
    macro_rules! with_ctx {
        ($h:expr, |$ctx:ident| $body:expr) => {{
            let dl = $h.dl();
            let $ctx = ContentCtx {
                sources: &$h.sources,
                dl: &dl,
                root: &$h.root,
                sink: &$h.sink,
            };
            $body
        }};
    }

    /// Serves `body` at `at` for every GET.
    async fn serve(server: &MockServer, at: &str, body: Vec<u8>) {
        Mock::given(method("GET"))
            .and(path(at.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(server)
            .await;
    }

    /// One Modrinth version of a modpack, whose primary file is the `.mrpack`.
    fn pack_version(
        server: &MockServer,
        id: &str,
        kind: &str,
        published: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "project_id": "PACKID",
            "name": id,
            "version_number": format!("{id}-number"),
            "version_type": kind,
            "date_published": published,
            "game_versions": ["1.20.1"],
            "loaders": ["fabric"],
            "files": [{
                "url": format!("{}/files/pack.mrpack", server.uri()),
                "filename": "pack.mrpack",
                "size": PACK_BYTES.len(),
                "primary": true,
                "hashes": { "sha1": sha1_hex(PACK_BYTES) },
            }],
            "dependencies": [],
        })
    }

    /// A Modrinth client and its mock server, serving two versions and the pack file.
    async fn modrinth_pack(server: &MockServer) -> BoxSource {
        let body = serde_json::json!([
            pack_version(server, "beta-newer", "beta", "2026-09-01T00:00:00Z"),
            pack_version(server, "release-older", "release", "2026-01-01T00:00:00Z"),
        ])
        .to_string();
        serve(server, "/project/my-pack/version", body.into_bytes()).await;
        serve(server, "/files/pack.mrpack", PACK_BYTES.to_vec()).await;
        Arc::new(Modrinth::with_base_url(
            HttpClient::new().expect("http"),
            server.uri(),
        ))
    }

    #[tokio::test]
    async fn fetch_pack_downloads_the_newest_release_and_records_its_source() {
        let server = MockServer::start().await;
        let source = modrinth_pack(&server).await;
        let harness = Harness::new(vec![source]);

        let (path, pack) = with_ctx!(harness, |ctx| fetch_pack(
            &ctx,
            SourceId::Modrinth,
            "my-pack",
            None
        )
        .await
        .expect("fetch"));

        assert_eq!(std::fs::read(&path).expect("object"), PACK_BYTES);
        assert_eq!(
            path,
            harness
                .root
                .object_path(&sha1_hex(PACK_BYTES))
                .expect("object path")
        );
        assert_eq!(pack.source, "modrinth");
        assert_eq!(pack.project_id, "PACKID");
        assert_eq!(
            pack.version_id, "release-older",
            "a release outranks a newer beta"
        );
    }

    #[tokio::test]
    async fn fetch_pack_honors_a_pinned_version() {
        let server = MockServer::start().await;
        let source = modrinth_pack(&server).await;
        let harness = Harness::new(vec![source]);

        let (_, pack) = with_ctx!(harness, |ctx| fetch_pack(
            &ctx,
            SourceId::Modrinth,
            "my-pack",
            Some("beta-newer")
        )
        .await
        .expect("fetch"));
        assert_eq!(pack.version_id, "beta-newer");
    }

    #[tokio::test]
    async fn fetch_pack_without_the_source_configured_is_disabled() {
        let harness = Harness::new(Vec::new());
        let err = with_ctx!(harness, |ctx| fetch_pack(
            &ctx,
            SourceId::Modrinth,
            "my-pack",
            None
        )
        .await
        .expect_err("no source"));
        assert!(
            matches!(err, Error::Sources(crate::sources::Error::Disabled { .. })),
            "{err:?}"
        );
    }

    /// One CurseForge file row, as `POST /v1/mods/files` answers it.
    fn cf_file(id: u32, mod_id: u32, url: Option<String>) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "modId": mod_id,
            "displayName": format!("file-{id}"),
            "fileName": format!("file-{id}.jar"),
            "releaseType": 1,
            "fileDate": "2026-06-01T12:00:00Z",
            "fileLength": PACK_BYTES.len(),
            "downloadUrl": url,
            "gameVersions": ["1.20.1", "Fabric"],
            "hashes": [{ "value": sha1_hex(PACK_BYTES), "algo": 1 }],
            "dependencies": [],
            "fileFingerprint": 4242u32,
        })
    }

    #[tokio::test]
    async fn curseforge_files_are_placed_and_a_null_url_becomes_a_manual_download() {
        let server = MockServer::start().await;
        serve(&server, "/v1/categories", CLASSES.as_bytes().to_vec()).await;
        serve(&server, "/files/one.jar", PACK_BYTES.to_vec()).await;

        let files_body = serde_json::json!({ "data": [
            cf_file(1001, 6001, Some(format!("{}/files/one.jar", server.uri()))),
            cf_file(1002, 6002, None),
        ]})
        .to_string();
        Mock::given(method("POST"))
            .and(path("/v1/mods/files"))
            .respond_with(ResponseTemplate::new(200).set_body_string(files_body))
            .mount(&server)
            .await;

        // Class 6 is `mc-mods` and class 12 is `texture-packs` in the fixture.
        let mods_body = serde_json::json!({ "data": [
            { "id": 6001, "name": "One", "slug": "one", "classId": 6 },
            { "id": 6002, "name": "Two", "slug": "two", "classId": 12 },
        ]})
        .to_string();
        Mock::given(method("POST"))
            .and(path("/v1/mods"))
            .respond_with(ResponseTemplate::new(200).set_body_string(mods_body))
            .mount(&server)
            .await;

        let client: BoxSource = Arc::new(CurseForge::with_base_url(
            HttpClient::new().expect("http"),
            KEY.to_string(),
            server.uri(),
        ));
        let harness = Harness::new(vec![client]);
        let mut instance = Instances::new(harness.root.clone())
            .create("Pack", "1.20.1", Loader::Fabric, None, &BTreeMap::new())
            .expect("create");

        let plan_files = vec![
            PackFile {
                path: None,
                url: None,
                sha1: None,
                size: None,
                source: Some((SourceId::CurseForge, "6001".to_string(), "1001".to_string())),
                required: true,
            },
            PackFile {
                path: None,
                url: None,
                sha1: None,
                size: None,
                source: Some((SourceId::CurseForge, "6002".to_string(), "1002".to_string())),
                required: true,
            },
        ];
        let (placed, manual) = with_ctx!(harness, |ctx| install_curseforge_files(
            &ctx,
            &mut instance,
            &plan_files
        )
        .await
        .expect("install"));

        assert_eq!(placed, 1);
        assert_eq!(
            std::fs::read(instance.game_dir().join("mods/file-1001.jar")).expect("mod"),
            PACK_BYTES
        );
        assert_eq!(instance.config.content.len(), 1);
        let entry = &instance.config.content[0];
        assert_eq!(entry.source, "curseforge");
        assert_eq!(entry.project_id, "6001");
        assert_eq!(entry.version_id, "1001");
        assert_eq!(entry.kind, ContentKind::Mod);
        assert_eq!(entry.fingerprint, Some(4242));

        assert_eq!(manual.len(), 1);
        assert_eq!(manual[0].version_id, "1002");
        assert_eq!(manual[0].file_name, "file-1002.jar");
        assert!(
            manual[0].page_url.ends_with("/files/1002"),
            "{}",
            manual[0].page_url
        );
    }

    #[tokio::test]
    async fn curseforge_files_without_the_source_configured_are_disabled() {
        let harness = Harness::new(Vec::new());
        let mut instance = Instances::new(harness.root.clone())
            .create("Pack", "1.20.1", Loader::Fabric, None, &BTreeMap::new())
            .expect("create");
        let err = with_ctx!(harness, |ctx| install_curseforge_files(
            &ctx,
            &mut instance,
            &[]
        )
        .await
        .expect_err("no source"));
        assert!(
            matches!(
                err,
                Error::Sources(crate::sources::Error::Disabled {
                    source_id: SourceId::CurseForge,
                    ..
                })
            ),
            "{err:?}"
        );
    }
}
