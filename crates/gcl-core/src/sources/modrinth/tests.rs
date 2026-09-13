//! Tests for the Modrinth client. Every request is served by wiremock; the bodies are
//! the recorded fixtures under `tests/fixtures/modrinth/`, or a small hand-written body
//! when the fixture does not cover the case (dependencies, modpacks, unmapped types).

use std::time::Duration;

use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const SEARCH_SODIUM: &str =
    include_str!("../../../../../tests/fixtures/modrinth/search_sodium.json");
const PROJECT_SODIUM: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_sodium.json");
const VERSIONS_SODIUM: &str =
    include_str!("../../../../../tests/fixtures/modrinth/versions_sodium_1.20.1_fabric.json");
const VERSION_FILES: &str =
    include_str!("../../../../../tests/fixtures/modrinth/version_files_lookup.json");
const PROJECT_DESCRIPTION: &str =
    include_str!("../../../../../tests/fixtures/modrinth/project_description.json");
const SEARCH_TYPES: &str = include_str!("../../../../../tests/fixtures/modrinth/search_types.json");
const SEARCH_PACKS: &str = include_str!("../../../../../tests/fixtures/modrinth/search_packs.json");

/// A client with no backoff, so a retry in a failing test does not stall the suite.
fn client() -> HttpClient {
    HttpClient::new()
        .expect("client builds")
        .with_backoff(vec![Duration::ZERO; 3])
}

/// Mounts one GET mock and returns a Modrinth client pointed at the server.
async fn serve(server: &MockServer, at: &str, body: &str) -> Modrinth {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
    Modrinth::with_base_url(client(), server.uri())
}

fn mod_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        kind: Some(ContentKind::Mod),
        ..SearchQuery::default()
    }
}

#[test]
fn base_url_is_the_v2_api() {
    assert_eq!(BASE, "https://api.modrinth.com/v2");
}

#[test]
fn supported_kinds_omit_world() {
    // search_types.json records a live `project_type:world` search: 0 hits.
    let types: serde_json::Value = serde_json::from_str(SEARCH_TYPES).expect("fixture parses");
    assert_eq!(types["world"]["total_hits"], 0);
    assert!(types["mod"]["total_hits"].as_u64().unwrap_or(0) > 0);

    let source = Modrinth::new(client());
    assert_eq!(
        source.supported_kinds(),
        &[
            ContentKind::Mod,
            ContentKind::ResourcePack,
            ContentKind::Shader,
            ContentKind::DataPack
        ]
    );
    assert_eq!(source.id(), SourceId::Modrinth);
}

#[tokio::test]
async fn search_for_worlds_is_unsupported() {
    // No request is mounted: the kind is rejected before any HTTP call.
    let server = MockServer::start().await;
    let source = Modrinth::with_base_url(client(), server.uri());
    let q = SearchQuery {
        kind: Some(ContentKind::World),
        ..SearchQuery::default()
    };
    let err = source.search(&q).await.expect_err("world is unsupported");
    assert!(
        matches!(
            err,
            Error::UnsupportedKind {
                source_id: SourceId::Modrinth,
                kind: ContentKind::World
            }
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn search_sends_facets_for_kind_minecraft_and_loader() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("query", "sodium"))
        .and(query_param("index", "relevance"))
        .and(query_param("limit", "3"))
        .and(query_param("offset", "20"))
        .and(query_param(
            "facets",
            r#"[["project_type:mod"],["versions:1.20.1"],["categories:fabric"]]"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_SODIUM))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let q = SearchQuery {
        text: "sodium".into(),
        kind: Some(ContentKind::Mod),
        minecraft: Some("1.20.1".into()),
        loader: Some(Loader::Fabric),
        offset: 20,
        limit: 3,
    };
    let page = source.search(&q).await.expect("search succeeds");

    assert_eq!(page.total, 100);
    assert_eq!(page.offset, 0);
    assert_eq!(page.hits.len(), 3);
    let hit = &page.hits[0];
    assert_eq!(hit.source, SourceId::Modrinth);
    assert_eq!(hit.project_id, "AANobbMI");
    assert_eq!(hit.slug, "sodium");
    assert_eq!(hit.title, "Sodium");
    assert_eq!(hit.author, "jellysquid3");
    assert_eq!(hit.kind, ContentKind::Mod);
    assert_eq!(hit.downloads, 220_759_648);
    // `date_modified` as the fixture carries it, RFC 3339, unparsed.
    assert_eq!(hit.updated, "2026-09-02T16:13:16.493666+00:00");
    assert_eq!(hit.page_url, "https://modrinth.com/mod/sodium");
    assert!(hit.icon_url.is_some());
    assert!(hit.description.starts_with("A high-performance rendering"));
}

#[tokio::test]
async fn search_omits_the_loader_facet_unless_the_kind_is_mod() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("facets", r#"[["project_type:shader"]]"#))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_SODIUM))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let q = SearchQuery {
        kind: Some(ContentKind::Shader),
        loader: Some(Loader::Fabric),
        ..SearchQuery::default()
    };
    source.search(&q).await.expect("search succeeds");
}

#[tokio::test]
async fn search_omits_the_loader_facet_for_loader_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("facets", r#"[["project_type:mod"]]"#))
        .respond_with(ResponseTemplate::new(200).set_body_string(SEARCH_SODIUM))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let q = SearchQuery {
        loader: Some(Loader::None),
        ..mod_query("")
    };
    source.search(&q).await.expect("search succeeds");
}

/// Two hits whose `project_type` disagrees with the requested kind: this is what a
/// `project_type:datapack` search really answers (see `project_types_sample.json`,
/// where the datapack `veinminer` reports `project_type: "mod"`).
const MIXED_HITS: &str = r#"{
  "hits": [
    {"project_id":"OhduvhIc","slug":"veinminer","title":"VeinMiner","description":"d",
     "author":"a","project_type":"mod","downloads":1,"icon_url":null},
    {"project_id":"AAA","slug":"a-pack","title":"Pack","description":"d",
     "author":"a","project_type":"modpack","downloads":2,"icon_url":null},
    {"project_id":"BBB","slug":"fresh-animations","title":"Fresh","description":"d",
     "author":"a","project_type":"resourcepack","downloads":3,"icon_url":null}
  ],
  "offset": 0,
  "limit": 10,
  "total_hits": 3
}"#;

#[tokio::test]
async fn search_with_a_kind_labels_every_hit_with_that_kind() {
    let server = MockServer::start().await;
    let source = serve(&server, "/search", MIXED_HITS).await;

    let q = SearchQuery {
        kind: Some(ContentKind::DataPack),
        ..SearchQuery::default()
    };
    let page = source.search(&q).await.expect("search succeeds");

    // No hit is dropped and none keeps its own `project_type`: the query's kind wins.
    assert_eq!(page.hits.len(), 3);
    assert!(page.hits.iter().all(|h| h.kind == ContentKind::DataPack));
    assert_eq!(
        page.hits[0].page_url,
        "https://modrinth.com/datapack/veinminer"
    );
}

#[tokio::test]
async fn search_without_a_kind_maps_project_type_and_drops_modpacks() {
    let server = MockServer::start().await;
    let source = serve(&server, "/search", MIXED_HITS).await;

    let page = source
        .search(&SearchQuery::default())
        .await
        .expect("search succeeds");

    assert_eq!(page.hits.len(), 2, "the modpack hit is dropped");
    assert_eq!(page.hits[0].kind, ContentKind::Mod);
    assert_eq!(page.hits[1].kind, ContentKind::ResourcePack);
    assert_eq!(
        page.hits[1].page_url,
        "https://modrinth.com/resourcepack/fresh-animations"
    );
    assert_eq!(page.total, 3, "the total still counts every hit");
}

#[tokio::test]
async fn search_without_facets_sends_no_facets_param() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MIXED_HITS))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());
    source
        .search(&SearchQuery::default())
        .await
        .expect("search succeeds");

    let sent = &server.received_requests().await.expect("recorded")[0];
    let query = sent.url.query().unwrap_or_default();
    assert!(!query.contains("facets"), "got {query}");
}

#[tokio::test]
async fn project_parses_sodium() {
    let server = MockServer::start().await;
    let source = serve(&server, "/project/sodium", PROJECT_SODIUM).await;

    let project = source.project("sodium").await.expect("project succeeds");

    assert_eq!(project.source, SourceId::Modrinth);
    assert_eq!(project.id, "AANobbMI");
    assert_eq!(project.slug, "sodium");
    assert_eq!(project.title, "Sodium");
    assert_eq!(project.kind, ContentKind::Mod);
    assert_eq!(project.page_url, "https://modrinth.com/mod/sodium");
    assert!(project.description.starts_with("A high-performance"));
}

#[tokio::test]
async fn project_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/nope"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let err = source.project("nope").await.expect_err("404 fails");
    match err {
        Error::NotFound { source_id, id } => {
            assert_eq!(source_id, SourceId::Modrinth);
            assert_eq!(id, "nope");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn project_500_bubbles_as_http() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/sodium"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let err = source.project("sodium").await.expect_err("500 fails");
    assert!(matches!(err, Error::Http(_)), "got {err:?}");
}

#[tokio::test]
async fn project_rejects_a_modpack() {
    let body = r#"{"id":"1KVo5zza","slug":"fabulously-optimized","title":"FO",
                   "description":"d","project_type":"modpack"}"#;
    let server = MockServer::start().await;
    let source = serve(&server, "/project/fabulously-optimized", body).await;

    let err = source
        .project("fabulously-optimized")
        .await
        .expect_err("a modpack is not content");
    match err {
        Error::BadResponse { what, detail, .. } => {
            assert_eq!(what, "project_type");
            assert_eq!(detail, "modpack");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn project_reports_an_unparsable_body() {
    let server = MockServer::start().await;
    let source = serve(&server, "/project/sodium", "not json").await;

    let err = source.project("sodium").await.expect_err("bad body fails");
    match err {
        Error::BadResponse { what, .. } => assert_eq!(what, "project"),
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn versions_filter_by_minecraft_and_loaders_and_parse_files() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/AANobbMI/version"))
        .and(query_param("game_versions", r#"["1.20.1"]"#))
        .and(query_param("loaders", r#"["fabric","quilt"]"#))
        .and(query_param("include_changelog", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_string(VERSIONS_SODIUM))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let filter = VersionFilter {
        minecraft: Some("1.20.1".into()),
        loaders: vec!["fabric".into(), "quilt".into()],
    };
    let versions = source
        .versions("AANobbMI", &filter)
        .await
        .expect("versions succeed");

    assert_eq!(versions.len(), 2);
    // Newest first.
    assert_eq!(versions[0].id, "OihdIimA");
    assert_eq!(versions[1].id, "ryOMVRuG");
    assert!(versions[0].published > versions[1].published);

    let v = &versions[0];
    assert_eq!(v.source, SourceId::Modrinth);
    assert_eq!(v.project_id, "AANobbMI");
    assert_eq!(v.name, "Sodium 0.5.13 for Fabric");
    assert_eq!(v.number, "mc1.20.1-0.5.13-fabric");
    assert_eq!(v.kind, ReleaseKind::Release);
    assert_eq!(v.game_versions, vec!["1.20.1"]);
    assert_eq!(v.loaders, vec!["fabric", "quilt"]);
    assert_eq!(v.published, "2025-03-03T17:45:49.132919Z");
    assert!(v.dependencies.is_empty(), "the fixture has no dependencies");

    assert_eq!(v.files.len(), 1);
    let f = &v.files[0];
    assert!(f.primary);
    assert_eq!(f.file_name, "sodium-fabric-0.5.13+mc1.20.1.jar");
    assert_eq!(f.size, Some(971_552));
    assert_eq!(
        f.sha1.as_deref(),
        Some("bcdbf37d9494e405cba210d7a80ed58e756297b5")
    );
    assert!(f.sha512.as_deref().is_some_and(|h| h.len() == 128));
    assert!(f.url.as_deref().is_some_and(|u| u.ends_with(".jar")));
    assert_eq!(f.fingerprint, None, "Modrinth has no fingerprints");

    // The beta in the fixture maps to the beta channel.
    assert_eq!(versions[1].kind, ReleaseKind::Beta);
}

#[tokio::test]
async fn versions_send_no_filter_params_when_the_filter_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/AANobbMI/version"))
        .respond_with(ResponseTemplate::new(200).set_body_string(VERSIONS_SODIUM))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    source
        .versions("AANobbMI", &VersionFilter::default())
        .await
        .expect("versions succeed");

    let sent = &server.received_requests().await.expect("recorded")[0];
    let query = sent.url.query().unwrap_or_default().to_string();
    assert!(!query.contains("loaders"), "got {query}");
    assert!(!query.contains("game_versions"), "got {query}");
    assert!(query.contains("include_changelog=false"), "got {query}");
}

/// One version carrying every dependency kind. The recorded Sodium versions have none.
const VERSION_WITH_DEPS: &str = r#"[{
  "id":"abc","project_id":"XYZ","name":"Indium 1.0","version_number":"1.0",
  "version_type":"alpha","date_published":"2025-01-01T00:00:00Z",
  "game_versions":["1.20.1"],"loaders":["fabric"],
  "files":[{"url":"https://cdn.modrinth.com/x.jar","filename":"x.jar","size":10,
            "primary":true,"hashes":{"sha1":"aa","sha512":"bb"}}],
  "dependencies":[
    {"project_id":"P4GkI93","version_id":null,"dependency_type":"required"},
    {"project_id":"1IjD5062","version_id":"vvv","dependency_type":"optional"},
    {"project_id":"BAD","version_id":null,"dependency_type":"incompatible"},
    {"project_id":"EMB","version_id":null,"dependency_type":"embedded"}
  ]
}]"#;

#[tokio::test]
async fn versions_map_every_dependency_kind() {
    let server = MockServer::start().await;
    let source = serve(&server, "/project/XYZ/version", VERSION_WITH_DEPS).await;

    let versions = source
        .versions("XYZ", &VersionFilter::default())
        .await
        .expect("versions succeed");

    let deps = &versions[0].dependencies;
    assert_eq!(deps.len(), 4);
    assert_eq!(deps[0].kind, DependencyKind::Required);
    assert_eq!(deps[0].project_id.as_deref(), Some("P4GkI93"));
    assert_eq!(deps[0].version_id, None);
    assert_eq!(deps[1].kind, DependencyKind::Optional);
    assert_eq!(deps[1].version_id.as_deref(), Some("vvv"));
    assert_eq!(deps[2].kind, DependencyKind::Incompatible);
    assert_eq!(deps[3].kind, DependencyKind::Embedded);
    assert_eq!(versions[0].kind, ReleaseKind::Alpha);
}

#[tokio::test]
async fn versions_reject_an_unknown_version_type() {
    let body = VERSION_WITH_DEPS.replace("\"alpha\"", "\"nightly\"");
    let server = MockServer::start().await;
    let source = serve(&server, "/project/XYZ/version", &body).await;

    let err = source
        .versions("XYZ", &VersionFilter::default())
        .await
        .expect_err("unknown channel fails");
    match err {
        Error::BadResponse { what, detail, .. } => {
            assert_eq!(what, "version_type");
            assert_eq!(detail, "nightly");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn versions_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/nope/version"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let err = source
        .versions("nope", &VersionFilter::default())
        .await
        .expect_err("404 fails");
    assert!(
        matches!(err, Error::NotFound { ref id, .. } if id == "nope"),
        "got {err:?}"
    );
}

/// A modpack version: the primary file is the `.mrpack`.
const PACK_VERSIONS: &str = r#"[{
  "id":"pv1","project_id":"1KVo5zza","name":"FO 5.0","version_number":"5.0",
  "version_type":"release","date_published":"2025-05-01T00:00:00Z",
  "game_versions":["1.20.1"],"loaders":["fabric"],
  "files":[{"url":"https://cdn.modrinth.com/fo.mrpack","filename":"fo.mrpack",
            "size":99,"primary":true,"hashes":{"sha1":"aa","sha512":"bb"}}],
  "dependencies":[]
}]"#;

#[tokio::test]
async fn pack_versions_list_the_mrpack_and_send_no_loader_filter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/1KVo5zza/version"))
        .and(query_param("game_versions", r#"["1.20.1"]"#))
        .respond_with(ResponseTemplate::new(200).set_body_string(PACK_VERSIONS))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let versions = source
        .pack_versions("1KVo5zza", Some("1.20.1"))
        .await
        .expect("pack versions succeed");

    assert_eq!(versions.len(), 1);
    let primary = versions[0]
        .files
        .iter()
        .find(|f| f.primary)
        .expect("a primary file");
    assert_eq!(primary.file_name, "fo.mrpack");

    let sent = &server.received_requests().await.expect("recorded")[0];
    assert!(
        !sent.url.query().unwrap_or_default().contains("loaders"),
        "a modpack version is not filtered by loader"
    );
}

#[tokio::test]
async fn version_fetches_one_by_id() {
    let one = VERSION_WITH_DEPS
        .trim_start_matches('[')
        .trim_end_matches(']');
    let server = MockServer::start().await;
    let source = serve(&server, "/version/abc", one).await;

    let version = source.version("abc").await.expect("version succeeds");
    assert_eq!(version.id, "abc");
    assert_eq!(version.project_id, "XYZ");
    assert_eq!(version.files.len(), 1);
}

#[tokio::test]
async fn version_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/version/zz"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let err = source.version("zz").await.expect_err("404 fails");
    assert!(
        matches!(err, Error::NotFound { ref id, .. } if id == "zz"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn resolve_by_hash_posts_the_hashes_and_maps_the_answer() {
    let hash = "bcdbf37d9494e405cba210d7a80ed58e756297b5";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/version_files"))
        .and(body_json(serde_json::json!({
            "hashes": [hash],
            "algorithm": "sha1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(VERSION_FILES))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let versions = source
        .resolve_by_hash(&[hash.to_string()])
        .await
        .expect("lookup succeeds");

    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].id, "OihdIimA");
    assert_eq!(versions[0].project_id, "AANobbMI");
    assert_eq!(versions[0].files[0].sha1.as_deref(), Some(hash));
}

#[tokio::test]
async fn resolve_by_hash_keeps_the_caller_order_and_drops_unknown_hashes() {
    let known = "bcdbf37d9494e405cba210d7a80ed58e756297b5";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/version_files"))
        .respond_with(ResponseTemplate::new(200).set_body_string(VERSION_FILES))
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let versions = source
        .resolve_by_hash(&["deadbeef".into(), known.into()])
        .await
        .expect("lookup succeeds");

    assert_eq!(versions.len(), 1, "the unknown hash has no version");
    assert_eq!(versions[0].id, "OihdIimA");
}

#[tokio::test]
async fn resolve_by_hash_with_no_hashes_makes_no_request() {
    let server = MockServer::start().await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let versions = source.resolve_by_hash(&[]).await.expect("no-op succeeds");

    assert!(versions.is_empty());
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

#[tokio::test]
async fn resolve_by_fingerprint_is_always_empty() {
    let server = MockServer::start().await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let versions = source
        .resolve_by_fingerprint(&[123, 456])
        .await
        .expect("no-op succeeds");

    assert!(versions.is_empty(), "Modrinth has no fingerprint endpoint");
    assert!(
        server
            .received_requests()
            .await
            .expect("recorded")
            .is_empty()
    );
}

#[tokio::test]
async fn a_trailing_slash_in_the_base_url_is_trimmed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/sodium"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PROJECT_SODIUM))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), format!("{}/", server.uri()));

    source.project("sodium").await.expect("project succeeds");
}

#[tokio::test]
async fn search_text_is_url_encoded() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("query", "a b&c=d"))
        .respond_with(ResponseTemplate::new(200).set_body_string(MIXED_HITS))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    source
        .search(&mod_query("a b&c=d"))
        .await
        .expect("search succeeds");
}

#[tokio::test]
async fn search_packs_asks_for_the_modpack_project_type() {
    let server = MockServer::start().await;
    let source = serve(&server, "/search", SEARCH_PACKS).await;
    let q = SearchQuery {
        text: "fabulously".to_string(),
        minecraft: Some("1.20.1".to_string()),
        // A loader on a pack query is ignored: a pack states its loader in the pack index.
        loader: Some(Loader::Fabric),
        limit: 3,
        ..SearchQuery::default()
    };

    let page = source.search_packs(&q).await.expect("search packs");

    assert_eq!(page.total, 92);
    assert_eq!(page.offset, 0);
    assert_eq!(page.hits.len(), 3);
    assert!(page.hits.iter().all(|h| h.is_pack), "{:?}", page.hits);
    let hit = &page.hits[0];
    assert_eq!(hit.source, SourceId::Modrinth);
    assert_eq!(hit.project_id, "1KVo5zza");
    assert_eq!(hit.slug, "fabulously-optimized");
    assert_eq!(hit.title, "Fabulously Optimized");
    assert_eq!(hit.author, "robotkoer");
    assert_eq!(hit.downloads, 16_857_076);
    assert_eq!(hit.updated, "2026-08-30T14:07:56.116361+00:00");
    assert_eq!(
        hit.page_url,
        "https://modrinth.com/modpack/fabulously-optimized"
    );

    let requests = server.received_requests().await.unwrap_or_default();
    let search = requests.first().expect("one search request");
    let facets = search
        .url
        .query_pairs()
        .find(|(k, _)| k == "facets")
        .map(|(_, v)| v.to_string())
        .expect("a facets parameter");
    assert_eq!(
        facets, r#"[["project_type:modpack"],["versions:1.20.1"]]"#,
        "no loader facet on a pack search"
    );
}

#[tokio::test]
async fn a_mod_search_hit_is_not_a_pack() {
    let server = MockServer::start().await;
    let source = serve(&server, "/search", SEARCH_SODIUM).await;
    let page = source.search(&mod_query("sodium")).await.expect("search");
    assert!(page.hits.iter().all(|h| !h.is_pack), "{:?}", page.hits);
}

#[tokio::test]
async fn description_returns_the_project_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/example-mod"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PROJECT_DESCRIPTION))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let body = source.description("example-mod").await.expect("body");

    assert!(body.starts_with("# Example Mod"), "got {body:?}");
    assert!(body.contains("[Docs](https://example.com/docs)"));
}

#[tokio::test]
async fn description_of_a_project_without_a_body_is_empty() {
    let server = MockServer::start().await;
    let source = serve(
        &server,
        "/project/bare",
        r#"{"id":"a","slug":"bare","title":"Bare","project_type":"mod"}"#,
    )
    .await;

    assert_eq!(source.description("bare").await.expect("body"), "");
}

#[tokio::test]
async fn description_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/project/nope"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    match source.description("nope").await.expect_err("404 fails") {
        Error::NotFound { source_id, id } => {
            assert_eq!(source_id, SourceId::Modrinth);
            assert_eq!(id, "nope");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn changelog_returns_the_string_stored_on_the_version() {
    let one: serde_json::Value = serde_json::from_str(
        VERSION_WITH_DEPS
            .trim_start_matches('[')
            .trim_end_matches(']'),
    )
    .expect("one version");
    let mut with_notes = one.clone();
    with_notes["changelog"] = serde_json::json!("## Fixed\n\n- a crash on load\n");
    let server = MockServer::start().await;
    let source = serve(&server, "/version/abc", &with_notes.to_string()).await;

    let notes = source.changelog("XYZ", "abc").await.expect("changelog");
    assert!(notes.starts_with("## Fixed"), "got {notes:?}");
    assert!(notes.contains("a crash on load"));

    // The same GET answers `version`, so the field lands on the mapped struct too.
    let version = source.version("abc").await.expect("version");
    assert_eq!(version.changelog.as_deref(), Some(notes.as_str()));
}

#[tokio::test]
async fn changelog_of_a_version_without_one_is_empty() {
    let one = VERSION_WITH_DEPS
        .trim_start_matches('[')
        .trim_end_matches(']');
    let server = MockServer::start().await;
    let source = serve(&server, "/version/abc", one).await;

    assert_eq!(source.changelog("XYZ", "abc").await.expect("changelog"), "");
    assert_eq!(
        source.version("abc").await.expect("version").changelog,
        None
    );
}

#[tokio::test]
async fn changelog_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/version/zz"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let source = Modrinth::with_base_url(client(), server.uri());

    let err = source.changelog("XYZ", "zz").await.expect_err("404 fails");
    assert!(
        matches!(err, Error::NotFound { ref id, .. } if id == "zz"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn a_hit_without_a_date_modified_has_an_empty_updated() {
    let server = MockServer::start().await;
    let body = r#"{"hits":[{"project_id":"AAA","slug":"a","title":"A",
        "project_type":"mod","downloads":1}],"total_hits":1,"offset":0}"#;
    let source = serve(&server, "/search", body).await;

    let page = source.search(&mod_query("a")).await.expect("search");

    assert_eq!(page.hits[0].updated, "");
}

#[tokio::test]
async fn project_gallery_is_sorted_by_ordering() {
    // The fixture already lists its gallery in `ordering` order, so the body is served
    // reversed: the client, not the server, decides the order.
    let mut body: serde_json::Value = serde_json::from_str(PROJECT_SODIUM).expect("fixture parses");
    let gallery = body["gallery"]
        .as_array_mut()
        .expect("fixture has a gallery");
    gallery.reverse();
    let body = body.to_string();

    let server = MockServer::start().await;
    let source = serve(&server, "/project/sodium", &body).await;

    let project = source.project("sodium").await.expect("project succeeds");

    let titles: Vec<&str> = project
        .gallery
        .iter()
        .map(|g| g.title.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(
        titles,
        vec![
            "Underwater Lighting Improvements",
            "Biome Blending Improvements",
            "Fluid Rendering Improvements",
            "Block Shading Improvements",
            "Sodium 0.5.2",
            "Sodium 0.4.1",
        ]
    );
    let first = &project.gallery[0];
    assert_eq!(
        first.url,
        "https://cdn.modrinth.com/data/AANobbMI/images/d84313e6f57dc9e7896961dbd2dfc2689d482758_350.webp"
    );
    // Modrinth serves no separate thumbnail.
    assert_eq!(first.thumbnail_url, None);
    assert!(!first.featured);
    assert_eq!(
        first.description.as_deref(),
        Some("Sodium fixes many graphical issues with smooth lighting while underwater.")
    );
}

#[tokio::test]
async fn project_without_gallery_has_an_empty_one() {
    let server = MockServer::start().await;
    let source = serve(&server, "/project/sodium", PROJECT_DESCRIPTION).await;

    let project = source.project("sodium").await.expect("project succeeds");

    assert!(project.gallery.is_empty(), "got {:?}", project.gallery);
}
