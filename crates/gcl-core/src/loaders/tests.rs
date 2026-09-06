//! Wiremock tests for the loader contract, Fabric, and Quilt.

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::download::DownloadCtx;
use crate::events::null_sink;
use crate::mojang::{Mojang, RuleContext, plan_install};

const FABRIC_LIST: &str = include_str!("../../../../tests/fixtures/fabric/loader_1.20.1.json");
const FABRIC_PROFILE: &str = include_str!("../../../../tests/fixtures/fabric/profile_1.20.1.json");
const QUILT_LIST: &str = include_str!("../../../../tests/fixtures/quilt/loader_1.20.1.json");
const QUILT_PROFILE: &str = include_str!("../../../../tests/fixtures/quilt/profile_1.20.1.json");
const VANILLA: &str = include_str!("../../../../tests/fixtures/mojang/1.20.1.json");

/// Endpoints with both fabric-like bases pointed at one mock server.
fn endpoints(uri: &str) -> LoaderEndpoints {
    LoaderEndpoints {
        fabric: uri.to_string(),
        quilt: uri.to_string(),
        ..LoaderEndpoints::default()
    }
}

/// Owns the values `LoaderCtx` borrows for the length of a test.
struct Harness {
    _dir: TempDir,
    root: Root,
    http: HttpClient,
    sink: crate::events::EventSink,
    cancel: CancellationToken,
}

impl Harness {
    fn new() -> Harness {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        Harness {
            _dir: dir,
            root,
            http: HttpClient::new()
                .expect("client")
                .with_backoff(vec![std::time::Duration::ZERO]),
            sink: null_sink(),
            cancel: CancellationToken::new(),
        }
    }

    fn dl(&self) -> DownloadCtx<'_> {
        DownloadCtx {
            http: &self.http,
            root: &self.root,
            sink: &self.sink,
            cancel: &self.cancel,
            parallel: 2,
        }
    }

    fn ctx<'a>(&'a self, dl: &'a DownloadCtx<'a>) -> LoaderCtx<'a> {
        LoaderCtx {
            http: &self.http,
            root: &self.root,
            dl,
            java: None,
            runner: None,
        }
    }

    /// Plants the vanilla 1.20.1 version JSON in the cache so `resolve` can find it.
    fn plant_vanilla(&self) {
        crate::paths::write_atomic(
            &self.root.versions_dir().join("1.20.1.json"),
            VANILLA.as_bytes(),
        )
        .expect("plant vanilla");
    }
}

/// Serves only the loader list for one fabric-like API version.
async fn mock_list(api: &str, list: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{api}/versions/loader/1.20.1")))
        .respond_with(ResponseTemplate::new(200).set_body_string(list))
        .expect(1)
        .mount(&server)
        .await;
    server
}

/// Serves the loader list and the profile JSON for one fabric-like API version.
async fn mock_meta(api: &str, list: &str, profile: &str, loader_version: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/{api}/versions/loader/1.20.1")))
        .respond_with(ResponseTemplate::new(200).set_body_string(list))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/{api}/versions/loader/1.20.1/{loader_version}/profile/json"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_string(profile))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[test]
fn version_ids_follow_the_loader_scheme() {
    assert_eq!(
        version_id(Loader::Fabric, "1.20.1", "0.19.5"),
        "fabric-loader-0.19.5-1.20.1"
    );
    assert_eq!(
        version_id(Loader::Quilt, "1.20.1", "0.20.0-beta.9"),
        "quilt-loader-0.20.0-beta.9-1.20.1"
    );
    assert_eq!(
        version_id(Loader::Forge, "1.20.1", "47.4.0"),
        "1.20.1-forge-47.4.0"
    );
    assert_eq!(
        version_id(Loader::NeoForge, "1.21.1", "21.1.65"),
        "neoforge-21.1.65"
    );
    assert_eq!(version_id(Loader::None, "1.20.1", ""), "1.20.1");
}

#[test]
fn only_forge_like_loaders_keep_both_libraries() {
    assert!(keep_both_libraries(Loader::Forge));
    assert!(keep_both_libraries(Loader::NeoForge));
    assert!(!keep_both_libraries(Loader::Fabric));
    assert!(!keep_both_libraries(Loader::Quilt));
    assert!(!keep_both_libraries(Loader::None));
}

#[test]
fn default_endpoints_are_the_production_hosts() {
    let ep = LoaderEndpoints::default();
    assert_eq!(ep.fabric, "https://meta.fabricmc.net");
    assert_eq!(ep.quilt, "https://meta.quiltmc.org");
    assert_eq!(ep.forge_meta, "https://files.minecraftforge.net");
    assert_eq!(ep.forge_maven, "https://maven.minecraftforge.net");
    assert_eq!(ep.neoforge, "https://maven.neoforged.net");
}

#[tokio::test]
async fn fabric_list_versions_marks_the_first_stable_recommended() {
    let server = mock_list("v2", FABRIC_LIST).await;
    let h = Harness::new();
    let dl = h.dl();
    let ctx = h.ctx(&dl);

    let versions = list_versions(&ctx, &endpoints(&server.uri()), Loader::Fabric, "1.20.1")
        .await
        .expect("list");

    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].version, "0.19.5");
    assert!(versions[0].stable);
    assert!(versions[0].recommended);
    assert!(!versions[1].recommended);
    assert!(!versions[2].recommended);
    server.verify().await;
}

#[tokio::test]
async fn fabric_install_writes_the_profile_and_is_cached() {
    let server = mock_meta("v2", FABRIC_LIST, FABRIC_PROFILE, "0.19.5").await;
    let h = Harness::new();
    let dl = h.dl();
    let ctx = h.ctx(&dl);
    let ep = endpoints(&server.uri());

    let id = install(&ctx, &ep, Loader::Fabric, "1.20.1", "0.19.5")
        .await
        .expect("install");
    assert_eq!(id, "fabric-loader-0.19.5-1.20.1");

    let file = h.root.versions_dir().join(format!("{id}.json"));
    let written: crate::mojang::VersionJson =
        serde_json::from_str(&std::fs::read_to_string(&file).expect("read")).expect("parse");
    assert_eq!(written.id, id);
    assert_eq!(written.inherits_from.as_deref(), Some("1.20.1"));
    assert_eq!(
        written.main_class.as_deref(),
        Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
    );

    // A second install reuses the cache: the profile mock stays at exactly one request.
    let again = install(&ctx, &ep, Loader::Fabric, "1.20.1", "0.19.5")
        .await
        .expect("second install");
    assert_eq!(again, id);
    server.verify().await;
}

#[tokio::test]
async fn fabric_install_of_an_unknown_version_is_no_such_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v2/versions/loader/1.20.1/9.9.9/profile/json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let h = Harness::new();
    let dl = h.dl();
    let ctx = h.ctx(&dl);

    let err = install(
        &ctx,
        &endpoints(&server.uri()),
        Loader::Fabric,
        "1.20.1",
        "9.9.9",
    )
    .await
    .expect_err("404 must not install");

    match err {
        Error::NoSuchVersion {
            loader,
            mc,
            version,
        } => {
            assert_eq!(loader, Loader::Fabric);
            assert_eq!(mc, "1.20.1");
            assert_eq!(version, "9.9.9");
        }
        other => panic!("expected NoSuchVersion, got {other:?}"),
    }
}

#[tokio::test]
async fn fabric_profile_resolves_to_a_knot_classpath() {
    let server = mock_meta("v2", FABRIC_LIST, FABRIC_PROFILE, "0.19.5").await;
    let h = Harness::new();
    h.plant_vanilla();
    let dl = h.dl();
    let ctx = h.ctx(&dl);

    let id = install(
        &ctx,
        &endpoints(&server.uri()),
        Loader::Fabric,
        "1.20.1",
        "0.19.5",
    )
    .await
    .expect("install");

    let mojang = Mojang::with_base_url(h.http.clone(), h.root.clone(), server.uri());
    let profile = mojang
        .load_cached_version(&id)
        .expect("load")
        .expect("present");
    let resolved = mojang
        .resolve(profile, keep_both_libraries(Loader::Fabric))
        .expect("resolve");
    assert_eq!(
        resolved.main_class.as_deref(),
        Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
    );

    let plan = plan_install(&resolved, &h.root, &RuleContext::current(), None).expect("plan");
    let cp: Vec<String> = plan
        .classpath
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        cp.iter().any(|p| p.contains("fabric-loader")),
        "classpath has no fabric-loader jar: {cp:?}"
    );
    assert!(
        cp.iter().any(|p| p.contains("intermediary")),
        "classpath has no intermediary jar: {cp:?}"
    );
    assert!(
        cp.iter().any(|p| p.contains("lwjgl")),
        "classpath lost the vanilla libraries: {cp:?}"
    );
}

#[tokio::test]
async fn quilt_list_and_install_use_the_v3_api() {
    let server = mock_meta("v3", QUILT_LIST, QUILT_PROFILE, "0.20.0-beta.9").await;
    let h = Harness::new();
    h.plant_vanilla();
    let dl = h.dl();
    let ctx = h.ctx(&dl);
    let ep = endpoints(&server.uri());

    let versions = list_versions(&ctx, &ep, Loader::Quilt, "1.20.1")
        .await
        .expect("list");
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].version, "0.20.0-beta.9");
    // Quilt omits `stable`, so the newest build is the recommended one.
    assert!(versions.iter().all(|v| !v.stable));
    assert!(versions[0].recommended);
    assert!(versions[1..].iter().all(|v| !v.recommended));

    let id = install(&ctx, &ep, Loader::Quilt, "1.20.1", "0.20.0-beta.9")
        .await
        .expect("install");
    assert_eq!(id, "quilt-loader-0.20.0-beta.9-1.20.1");

    let mojang = Mojang::with_base_url(h.http.clone(), h.root.clone(), server.uri());
    let profile = mojang
        .load_cached_version(&id)
        .expect("load")
        .expect("present");
    assert_eq!(profile.inherits_from.as_deref(), Some("1.20.1"));
    let resolved = mojang
        .resolve(profile, keep_both_libraries(Loader::Quilt))
        .expect("resolve");
    assert_eq!(
        resolved.main_class.as_deref(),
        Some("org.quiltmc.loader.impl.launch.knot.KnotClient")
    );

    let plan = plan_install(&resolved, &h.root, &RuleContext::current(), None).expect("plan");
    let cp: Vec<String> = plan
        .classpath
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        cp.iter().any(|p| p.contains("quilt-loader")),
        "classpath has no quilt-loader jar: {cp:?}"
    );
    assert!(
        cp.iter().any(|p| p.contains("intermediary")),
        "classpath has no intermediary jar: {cp:?}"
    );
}

#[tokio::test]
async fn forge_like_and_vanilla_loaders_are_not_supported_yet() {
    let h = Harness::new();
    let dl = h.dl();
    let ctx = h.ctx(&dl);
    let ep = LoaderEndpoints::default();

    for loader in [Loader::Forge, Loader::NeoForge, Loader::None] {
        let err = list_versions(&ctx, &ep, loader, "1.20.1")
            .await
            .expect_err("not supported yet");
        assert!(matches!(err, Error::Unsupported(_, l) if l == loader));
        let err = install(&ctx, &ep, loader, "1.20.1", "1.0")
            .await
            .expect_err("not supported yet");
        assert!(matches!(err, Error::Unsupported(_, l) if l == loader));
    }
}

#[tokio::test]
async fn fabric_like_urls_percent_encode_the_version_segments() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&server)
        .await;
    let h = Harness::new();
    let dl = h.dl();
    let ctx = h.ctx(&dl);

    let versions = list_versions(
        &ctx,
        &endpoints(&server.uri()),
        Loader::Fabric,
        "1.20.1/../../evil",
    )
    .await
    .expect("list");
    assert!(versions.is_empty());

    let requests = server
        .received_requests()
        .await
        .expect("mock server records requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url.path(),
        "/v2/versions/loader/1.20.1%2F..%2F..%2Fevil"
    );
}

#[tokio::test]
async fn fabric_like_install_urls_percent_encode_the_loader_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let h = Harness::new();
    let dl = h.dl();
    let ctx = h.ctx(&dl);

    let err = install(
        &ctx,
        &endpoints(&server.uri()),
        Loader::Quilt,
        "1.20.1",
        "0.20.0 beta9",
    )
    .await
    .expect_err("404");
    assert!(matches!(err, Error::NoSuchVersion { .. }), "{err:?}");

    let requests = server
        .received_requests()
        .await
        .expect("mock server records requests");
    assert_eq!(
        requests[0].url.path(),
        "/v3/versions/loader/1.20.1/0.20.0%20beta9/profile/json"
    );
}
