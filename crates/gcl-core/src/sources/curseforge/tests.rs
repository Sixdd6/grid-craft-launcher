//! Tests for the CurseForge client. Every request is served by wiremock with the
//! synthetic fixtures under `tests/fixtures/curseforge/` (see that directory's
//! `README.md`: no live key was available, so the bodies were built from the public
//! docs schema). Every mock matches the `x-api-key` header, so a request that forgets
//! the key does not match and the test fails.

use std::time::Duration;

use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const CLASSES: &str =
    include_str!("../../../../../tests/fixtures/curseforge/categories_classes.json");
const SEARCH: &str = include_str!("../../../../../tests/fixtures/curseforge/search_mods.json");
const MOD: &str = include_str!("../../../../../tests/fixtures/curseforge/get_mod.json");
const MOD_FILES: &str = include_str!("../../../../../tests/fixtures/curseforge/get_mod_files.json");
const FILES_BATCH: &str =
    include_str!("../../../../../tests/fixtures/curseforge/get_files_batch.json");
const FINGERPRINTS: &str =
    include_str!("../../../../../tests/fixtures/curseforge/fingerprints.json");

const KEY: &str = "test-api-key";

/// A client with no backoff, so a retry in a failing test does not stall the suite.
fn client() -> HttpClient {
    HttpClient::new()
        .expect("client builds")
        .with_backoff(vec![Duration::ZERO; 3])
}

/// Mounts the class-id call, which nearly every method needs first.
async fn mount_classes(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/categories"))
        .and(header("x-api-key", KEY))
        .and(query_param("gameId", "432"))
        .and(query_param("classesOnly", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_string(CLASSES))
        .mount(server)
        .await;
}

fn source(server: &MockServer) -> CurseForge {
    CurseForge::with_base_url(client(), KEY.to_string(), server.uri())
}

#[test]
fn constants_are_the_live_api() {
    assert_eq!(BASE, "https://api.curseforge.com");
    assert_eq!(GAME_ID, 432);
    let cf = CurseForge::new(client(), KEY.to_string());
    assert_eq!(cf.id(), SourceId::CurseForge);
    assert_eq!(
        cf.supported_kinds(),
        &[
            ContentKind::Mod,
            ContentKind::ResourcePack,
            ContentKind::Shader,
            ContentKind::DataPack,
            ContentKind::World,
        ]
    );
}

#[test]
fn debug_never_prints_the_api_key() {
    let text = format!(
        "{:?}",
        CurseForge::new(client(), "super-secret".to_string())
    );
    assert!(!text.contains("super-secret"), "got {text}");
    assert!(text.contains("api_key: <set>"), "got {text}");
}

#[test]
fn file_page_url_appends_the_file_id() {
    assert_eq!(
        file_page_url(
            "https://www.curseforge.com/minecraft/mc-mods/sodium",
            "5230381"
        ),
        "https://www.curseforge.com/minecraft/mc-mods/sodium/files/5230381"
    );
    assert_eq!(
        file_page_url("https://www.curseforge.com/minecraft/mc-mods/sodium/", "1"),
        "https://www.curseforge.com/minecraft/mc-mods/sodium/files/1"
    );
}

#[tokio::test]
async fn class_ids_parse_by_slug_and_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/categories"))
        .and(header("x-api-key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_string(CLASSES))
        .expect(1)
        .mount(&server)
        .await;
    let cf = source(&server);

    let first = cf.class_ids().await.expect("classes parse");
    assert_eq!(first.mods, 6);
    assert_eq!(first.modpacks, 4471);
    assert_eq!(first.resource_packs, 12);
    assert_eq!(first.worlds, 17);
    assert_eq!(first.shaders, Some(6552));
    assert_eq!(first.data_packs, Some(6945));

    // Second call is served from the OnceCell; `.expect(1)` above proves it.
    let second = cf.class_ids().await.expect("cached");
    assert_eq!(second.mods, first.mods);
}

#[tokio::test]
async fn class_ids_without_shaders_leave_it_none() {
    let server = MockServer::start().await;
    let body = serde_json::json!({ "data": [
        { "id": 6, "slug": "mc-mods", "isClass": true },
        { "id": 4471, "slug": "modpacks", "isClass": true },
        { "id": 12, "slug": "texture-packs", "isClass": true },
        { "id": 17, "slug": "worlds", "isClass": true },
    ] })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/v1/categories"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let classes = source(&server).class_ids().await.expect("classes parse");
    assert_eq!(classes.shaders, None);
    assert_eq!(classes.data_packs, None);
}

#[tokio::test]
async fn class_ids_missing_a_required_class_is_a_bad_response() {
    let server = MockServer::start().await;
    let body = serde_json::json!({ "data": [
        { "id": 6, "slug": "mc-mods", "isClass": true },
    ] })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/v1/categories"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let err = source(&server)
        .class_ids()
        .await
        .expect_err("modpacks is missing");
    match err {
        Error::BadResponse { what, detail, .. } => {
            assert_eq!(what, "categories");
            assert!(detail.contains("modpacks"), "got {detail}");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn missing_api_key_header_makes_the_request_fail() {
    // The mock only answers with the header, so a client that never sends it 404s.
    let server = MockServer::start().await;
    mount_classes(&server).await;
    let cf = CurseForge::with_base_url(client(), "wrong-key".to_string(), server.uri());
    assert!(cf.class_ids().await.is_err());
}

#[tokio::test]
async fn search_builds_the_query_and_maps_hits() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .and(header("x-api-key", KEY))
        .and(query_param("gameId", "432"))
        .and(query_param("classId", "6"))
        .and(query_param("searchFilter", "sodium"))
        .and(query_param("gameVersion", "1.20.1"))
        .and(query_param("modLoaderType", "4"))
        .and(query_param("pageSize", "3"))
        .and(query_param("index", "20"))
        .and(query_param("sortField", "2"))
        .and(query_param("sortOrder", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .expect(1)
        .mount(&server)
        .await;

    let q = SearchQuery {
        text: "sodium".into(),
        kind: Some(ContentKind::Mod),
        minecraft: Some("1.20.1".into()),
        loader: Some(Loader::Fabric),
        offset: 20,
        limit: 3,
    };
    let page = source(&server).search(&q).await.expect("search");
    assert_eq!(page.total, 3);
    assert_eq!(page.offset, 0);
    assert_eq!(page.hits.len(), 3);

    let hit = &page.hits[0];
    assert_eq!(hit.source, SourceId::CurseForge);
    assert_eq!(hit.project_id, "394468");
    assert_eq!(hit.slug, "sodium");
    assert_eq!(hit.title, "Sodium");
    assert_eq!(hit.author, "jellysquid3");
    assert_eq!(hit.kind, ContentKind::Mod);
    assert_eq!(hit.downloads, 182_345_123);
    assert_eq!(
        hit.icon_url.as_deref(),
        Some(
            "https://media.forgecdn.net/avatars/thumbnails/305/563/256/256/636663849169721676.png"
        )
    );
    assert_eq!(
        hit.page_url,
        "https://www.curseforge.com/minecraft/mc-mods/sodium"
    );
}

#[tokio::test]
async fn search_without_a_kind_sends_no_class_id() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .and(header("x-api-key", KEY))
        .and(query_param("searchFilter", "sodium"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .expect(1)
        .mount(&server)
        .await;

    let q = SearchQuery {
        text: "sodium".into(),
        ..SearchQuery::default()
    };
    let page = source(&server).search(&q).await.expect("search");
    // The class of every hit is 6 (mods), resolved through class_ids().
    assert!(page.hits.iter().all(|h| h.kind == ContentKind::Mod));
    let requests = server.received_requests().await.unwrap_or_default();
    let search = requests
        .iter()
        .find(|r| r.url.path() == "/v1/mods/search")
        .expect("search request");
    assert!(
        !search.url.query_pairs().any(|(k, _)| k == "classId"),
        "got {}",
        search.url
    );
}

#[tokio::test]
async fn search_drops_hits_whose_class_is_unknown() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    // classId 4471 is Modpacks, which is not a ContentKind, so the hit drops out.
    let body = serde_json::json!({
        "data": [{
            "id": 1, "gameId": 432, "name": "Pack", "slug": "pack", "classId": 4471,
            "summary": "", "downloadCount": 0, "authors": [], "links": {}
        }],
        "pagination": { "index": 0, "pageSize": 20, "resultCount": 1, "totalCount": 1 }
    })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let page = source(&server)
        .search(&SearchQuery::default())
        .await
        .expect("search");
    assert!(page.hits.is_empty());
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn search_for_shaders_is_unsupported_when_the_class_is_absent() {
    let server = MockServer::start().await;
    let body = serde_json::json!({ "data": [
        { "id": 6, "slug": "mc-mods", "isClass": true },
        { "id": 4471, "slug": "modpacks", "isClass": true },
        { "id": 12, "slug": "texture-packs", "isClass": true },
        { "id": 17, "slug": "worlds", "isClass": true },
    ] })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/v1/categories"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let q = SearchQuery {
        kind: Some(ContentKind::Shader),
        ..SearchQuery::default()
    };
    let err = source(&server)
        .search(&q)
        .await
        .expect_err("no shader class");
    assert!(
        matches!(
            err,
            Error::UnsupportedKind {
                source_id: SourceId::CurseForge,
                kind: ContentKind::Shader
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn search_sends_no_loader_for_a_resource_pack() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .and(query_param("classId", "12"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .mount(&server)
        .await;

    let q = SearchQuery {
        kind: Some(ContentKind::ResourcePack),
        loader: Some(Loader::Fabric),
        ..SearchQuery::default()
    };
    source(&server).search(&q).await.expect("search");
    let requests = server.received_requests().await.unwrap_or_default();
    let search = requests
        .iter()
        .find(|r| r.url.path() == "/v1/mods/search")
        .expect("search request");
    assert!(
        !search.url.query_pairs().any(|(k, _)| k == "modLoaderType"),
        "got {}",
        search.url
    );
}

#[tokio::test]
async fn project_by_numeric_id_hits_the_mod_endpoint() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/394468"))
        .and(header("x-api-key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_string(MOD))
        .expect(1)
        .mount(&server)
        .await;

    let project = source(&server).project("394468").await.expect("project");
    assert_eq!(project.id, "394468");
    assert_eq!(project.slug, "sodium");
    assert_eq!(project.title, "Sodium");
    assert_eq!(project.kind, ContentKind::Mod);
    assert!(project.description.starts_with("A modern rendering engine"));
    assert_eq!(
        project.page_url,
        "https://www.curseforge.com/minecraft/mc-mods/sodium"
    );
}

#[tokio::test]
async fn project_by_slug_searches_and_takes_the_exact_match() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .and(header("x-api-key", KEY))
        .and(query_param("gameId", "432"))
        .and(query_param("slug", "sodium"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .expect(1)
        .mount(&server)
        .await;

    let project = source(&server).project("sodium").await.expect("project");
    assert_eq!(project.id, "394468");
    assert_eq!(project.slug, "sodium");
}

#[tokio::test]
async fn project_by_slug_matches_case_insensitively() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .and(query_param("slug", "Sodium"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .expect(1)
        .mount(&server)
        .await;

    // The fixture's slug is `sodium`; the user typed `Sodium`.
    let project = source(&server).project("Sodium").await.expect("project");
    assert_eq!(project.id, "394468");
    assert_eq!(project.slug, "sodium");
}

#[tokio::test]
async fn search_clamps_the_page_size_to_the_api_maximum() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .and(query_param("pageSize", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .expect(1)
        .mount(&server)
        .await;

    let q = SearchQuery {
        text: "sodium".into(),
        limit: 100,
        ..SearchQuery::default()
    };
    source(&server).search(&q).await.expect("search");
    let requests = server.received_requests().await.unwrap_or_default();
    let search = requests
        .iter()
        .find(|r| r.url.path() == "/v1/mods/search")
        .expect("search request");
    assert!(
        search
            .url
            .query_pairs()
            .any(|(k, v)| k == "pageSize" && v == "50"),
        "asked for 100, the API caps at {PAGE_SIZE}: {}",
        search.url
    );
}

#[tokio::test]
async fn project_by_unknown_slug_is_not_found() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH))
        .mount(&server)
        .await;

    let err = source(&server)
        .project("not-a-real-mod")
        .await
        .expect_err("no exact slug");
    assert!(
        matches!(err, Error::NotFound { ref id, .. } if id == "not-a-real-mod"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn project_404_is_not_found() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/1"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = source(&server).project("1").await.expect_err("404");
    assert!(
        matches!(err, Error::NotFound { ref id, .. } if id == "1"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn project_of_modpack_class_is_a_bad_response() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    let body = serde_json::json!({ "data": {
        "id": 2, "gameId": 432, "name": "Pack", "slug": "pack", "classId": 4471,
        "summary": "", "downloadCount": 0, "authors": [], "links": {}
    } })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/v1/mods/2"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let err = source(&server)
        .project("2")
        .await
        .expect_err("modpack class");
    match err {
        Error::BadResponse { what, detail, .. } => {
            assert_eq!(what, "classId");
            assert_eq!(detail, "modpack");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn versions_filter_by_game_version_and_loader() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/394468/files"))
        .and(header("x-api-key", KEY))
        .and(query_param("gameVersion", "1.20.1"))
        .and(query_param("modLoaderType", "4"))
        .and(query_param("pageSize", "50"))
        .and(query_param("index", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MOD_FILES))
        .expect(1)
        .mount(&server)
        .await;

    let filter = VersionFilter {
        minecraft: Some("1.20.1".into()),
        loaders: vec!["fabric".into()],
    };
    let versions = source(&server)
        .versions("394468", &filter)
        .await
        .expect("versions");
    assert_eq!(versions.len(), 3);
    // Newest first by fileDate.
    assert_eq!(versions[0].id, "5230381");
    assert_eq!(versions[1].id, "5109221");
    assert_eq!(versions[2].id, "4998877");

    let newest = &versions[0];
    assert_eq!(newest.source, SourceId::CurseForge);
    assert_eq!(newest.project_id, "394468");
    assert_eq!(newest.name, "sodium-fabric-0.5.13+mc1.20.1.jar");
    assert_eq!(newest.number, "sodium-fabric-0.5.13+mc1.20.1.jar");
    assert_eq!(newest.kind, ReleaseKind::Release);
    assert_eq!(newest.game_versions, vec!["1.20.1".to_string()]);
    assert_eq!(newest.loaders, vec!["fabric".to_string()]);
    assert_eq!(newest.published, "2026-06-01T12:00:00Z");
    assert_eq!(newest.files.len(), 1);
    let file = &newest.files[0];
    assert_eq!(
        file.url.as_deref(),
        Some("https://edge.forgecdn.net/files/5230/381/sodium-fabric-0.5.13+mc1.20.1.jar")
    );
    assert_eq!(file.file_name, "sodium-fabric-0.5.13+mc1.20.1.jar");
    assert_eq!(file.size, Some(1_542_332));
    assert_eq!(
        file.sha1.as_deref(),
        Some("9f3c1a2b7e4d5f60718293a4b5c6d7e8f9012345")
    );
    assert_eq!(file.sha512, None);
    assert_eq!(file.fingerprint, Some(3_735_928_559));
    assert!(file.primary);
    assert_eq!(newest.dependencies.len(), 1);
    assert_eq!(newest.dependencies[0].kind, DependencyKind::Optional);
    assert_eq!(newest.dependencies[0].project_id.as_deref(), Some("306612"));
}

#[tokio::test]
async fn a_null_download_url_maps_to_none_and_keeps_the_required_dependency() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/394468/files"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MOD_FILES))
        .mount(&server)
        .await;

    let versions = source(&server)
        .mod_files(394468, &VersionFilter::default())
        .await
        .expect("files");
    let manual = versions
        .iter()
        .find(|v| v.id == "4998877")
        .expect("the opted-out file");
    assert_eq!(manual.kind, ReleaseKind::Alpha);
    assert_eq!(manual.files.len(), 1);
    let file = &manual.files[0];
    assert_eq!(file.url, None, "a null downloadUrl must not be invented");
    assert_eq!(
        file.sha1.as_deref(),
        Some("4d5f60718293a4b5c6d7e8f901234abc1237e2b")
    );
    assert_eq!(file.fingerprint, Some(1_088_490_427));
    // relationType 3 is Required; relationType 4 (Tool) is skipped.
    assert_eq!(manual.dependencies.len(), 1);
    assert_eq!(manual.dependencies[0].kind, DependencyKind::Required);
    assert_eq!(manual.dependencies[0].project_id.as_deref(), Some("306612"));
    assert_eq!(manual.dependencies[0].version_id, None);
}

#[tokio::test]
async fn pack_files_send_no_loader_filter() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/mods/394468/files"))
        .and(query_param("gameVersion", "1.20.1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MOD_FILES))
        .mount(&server)
        .await;

    source(&server)
        .pack_files(394468, Some("1.20.1"))
        .await
        .expect("pack files");
    let requests = server.received_requests().await.unwrap_or_default();
    let files = requests
        .iter()
        .find(|r| r.url.path() == "/v1/mods/394468/files")
        .expect("files request");
    assert!(
        !files.url.query_pairs().any(|(k, _)| k == "modLoaderType"),
        "got {}",
        files.url
    );
}

#[tokio::test]
async fn version_posts_one_file_id() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/mods/files"))
        .and(header("x-api-key", KEY))
        .and(body_json(serde_json::json!({ "fileIds": [5230381] })))
        .respond_with(ResponseTemplate::new(200).set_body_string(FILES_BATCH))
        .expect(1)
        .mount(&server)
        .await;

    let version = source(&server).version("5230381").await.expect("version");
    assert_eq!(version.id, "5230381");
    assert_eq!(version.project_id, "394468");
}

#[tokio::test]
async fn version_that_the_batch_omits_is_not_found() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/mods/files"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
        .mount(&server)
        .await;

    let err = source(&server)
        .version("77")
        .await
        .expect_err("no such file");
    assert!(
        matches!(err, Error::NotFound { ref id, .. } if id == "77"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn version_with_a_non_numeric_id_is_not_found() {
    let server = MockServer::start().await;
    let err = source(&server)
        .version("abc")
        .await
        .expect_err("not a number");
    assert!(
        matches!(err, Error::NotFound { ref id, .. } if id == "abc"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn files_batch_chunks_by_fifty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/mods/files"))
        .and(header("x-api-key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_string(FILES_BATCH))
        .expect(2)
        .mount(&server)
        .await;

    let ids: Vec<u32> = (1..=51).collect();
    let versions = source(&server).files_batch(&ids).await.expect("batch");
    // Two chunks, each answered with the two-file fixture.
    assert_eq!(versions.len(), 4);

    let requests = server.received_requests().await.unwrap_or_default();
    let bodies: Vec<serde_json::Value> = requests
        .iter()
        .filter_map(|r| serde_json::from_slice(&r.body).ok())
        .collect();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["fileIds"].as_array().map(Vec::len), Some(50));
    assert_eq!(bodies[1]["fileIds"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn files_batch_of_nothing_makes_no_request() {
    let server = MockServer::start().await;
    let versions = source(&server).files_batch(&[]).await.expect("empty");
    assert!(versions.is_empty());
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn mods_batch_maps_projects_and_chunks_by_fifty() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    let body = serde_json::json!({ "data": [
        {
            "id": 394468, "gameId": 432, "name": "Sodium", "slug": "sodium", "classId": 6,
            "summary": "Fast", "downloadCount": 1, "authors": [], "links": {}
        },
        {
            "id": 9, "gameId": 432, "name": "Pack", "slug": "pack", "classId": 4471,
            "summary": "", "downloadCount": 0, "authors": [], "links": {}
        }
    ] })
    .to_string();
    Mock::given(method("POST"))
        .and(path("/v1/mods"))
        .and(header("x-api-key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(2)
        .mount(&server)
        .await;

    let ids: Vec<u32> = (1..=51).collect();
    let projects = source(&server).mods_batch(&ids).await.expect("batch");
    // Two chunks, one mappable project each; the modpack-class row drops out.
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].id, "394468");
    assert_eq!(projects[0].kind, ContentKind::Mod);
    assert_eq!(
        projects[0].page_url,
        "https://www.curseforge.com/minecraft/mc-mods/sodium"
    );
}

#[tokio::test]
async fn resolve_by_hash_is_always_empty() {
    let server = MockServer::start().await;
    let out = source(&server)
        .resolve_by_hash(&["deadbeef".to_string()])
        .await
        .expect("no hash endpoint");
    assert!(out.is_empty());
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn resolve_by_fingerprint_maps_exact_matches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/fingerprints"))
        .and(header("x-api-key", KEY))
        .and(body_json(
            serde_json::json!({ "fingerprints": [3735928559u32, 1122334455u32] }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(FINGERPRINTS))
        .expect(1)
        .mount(&server)
        .await;

    let out = source(&server)
        .resolve_by_fingerprint(&[3_735_928_559, 1_122_334_455])
        .await
        .expect("fingerprints");
    // Only the exact match comes back; the unmatched fingerprint is dropped.
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].id, "5230381");
    assert_eq!(out[0].project_id, "394468");
    assert_eq!(out[0].files[0].fingerprint, Some(3_735_928_559));
}

#[tokio::test]
async fn resolve_by_fingerprint_of_nothing_makes_no_request() {
    let server = MockServer::start().await;
    let out = source(&server)
        .resolve_by_fingerprint(&[])
        .await
        .expect("empty");
    assert!(out.is_empty());
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn an_unknown_release_type_is_a_bad_response() {
    let server = MockServer::start().await;
    mount_classes(&server).await;
    let body = serde_json::json!({ "data": [{
        "id": 1, "modId": 2, "displayName": "x.jar", "fileName": "x.jar",
        "releaseType": 9, "fileDate": "2026-01-01T00:00:00Z", "fileLength": 1,
        "hashes": [], "gameVersions": [], "dependencies": []
    }] })
    .to_string();
    Mock::given(method("GET"))
        .and(path("/v1/mods/2/files"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let err = source(&server)
        .mod_files(2, &VersionFilter::default())
        .await
        .expect_err("releaseType 9");
    match err {
        Error::BadResponse { what, detail, .. } => {
            assert_eq!(what, "releaseType");
            assert_eq!(detail, "9");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn a_body_that_does_not_parse_is_a_bad_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/categories"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let err = source(&server).class_ids().await.expect_err("bad json");
    assert!(
        matches!(
            err,
            Error::BadResponse {
                what: "categories",
                ..
            }
        ),
        "got {err:?}"
    );
}
