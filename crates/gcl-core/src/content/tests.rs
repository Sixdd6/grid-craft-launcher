//! Tests for content orchestration, driven by an in-memory [`FakeSource`].
//!
//! File bytes come from a wiremock server, so `add` exercises the real download cache
//! and the real placing code in `instances::content`.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::download::DownloadCtx;
use crate::download::hash::sha1_hex;
use crate::events::{Event, EventSink};
use crate::http::HttpClient;
use crate::instances::Instances;
use crate::instances::model::Loader;
use crate::paths::Root;
use crate::sources::fingerprint::curseforge_fingerprint;
use crate::sources::{
    Dependency, DependencyKind, Project, ReleaseKind, SearchPage, SearchQuery, Source, Version,
    VersionFile, VersionFilter,
};

// ---------------------------------------------------------------------------
// FakeSource
// ---------------------------------------------------------------------------

/// An in-memory [`Source`]: it answers from the projects and versions it was built with.
struct FakeSource {
    id: SourceId,
    projects: Vec<Project>,
    versions: Vec<Version>,
}

impl FakeSource {
    fn new(id: SourceId) -> Self {
        FakeSource {
            id,
            projects: Vec::new(),
            versions: Vec::new(),
        }
    }

    fn with(mut self, project: Project, versions: Vec<Version>) -> Self {
        self.projects.push(project);
        self.versions.extend(versions);
        self
    }

    fn boxed(self) -> BoxSource {
        Arc::new(self)
    }
}

#[async_trait]
impl Source for FakeSource {
    fn id(&self) -> SourceId {
        self.id
    }

    fn supported_kinds(&self) -> &[ContentKind] {
        &[
            ContentKind::Mod,
            ContentKind::ResourcePack,
            ContentKind::Shader,
            ContentKind::DataPack,
            ContentKind::World,
        ]
    }

    async fn search(&self, _q: &SearchQuery) -> Result<SearchPage, crate::sources::Error> {
        Ok(SearchPage {
            hits: Vec::new(),
            total: 0,
            offset: 0,
        })
    }

    async fn project(&self, id_or_slug: &str) -> Result<Project, crate::sources::Error> {
        self.projects
            .iter()
            .find(|p| p.id == id_or_slug || p.slug == id_or_slug)
            .cloned()
            .ok_or_else(|| crate::sources::Error::NotFound {
                source_id: self.id,
                id: id_or_slug.to_string(),
            })
    }

    async fn versions(
        &self,
        project_id: &str,
        _f: &VersionFilter,
    ) -> Result<Vec<Version>, crate::sources::Error> {
        Ok(self
            .versions
            .iter()
            .filter(|v| v.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn version(&self, version_id: &str) -> Result<Version, crate::sources::Error> {
        self.versions
            .iter()
            .find(|v| v.id == version_id)
            .cloned()
            .ok_or_else(|| crate::sources::Error::NotFound {
                source_id: self.id,
                id: version_id.to_string(),
            })
    }

    async fn resolve_by_hash(
        &self,
        _sha1: &[String],
    ) -> Result<Vec<Version>, crate::sources::Error> {
        Ok(Vec::new())
    }

    async fn resolve_by_fingerprint(
        &self,
        _fps: &[u32],
    ) -> Result<Vec<Version>, crate::sources::Error> {
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn project(id: &str, kind: ContentKind) -> Project {
    Project {
        source: SourceId::Modrinth,
        id: id.to_string(),
        slug: id.to_string(),
        title: id.to_string(),
        description: String::new(),
        kind,
        page_url: format!("https://modrinth.com/mod/{id}"),
    }
}

fn version(project_id: &str, id: &str, number: &str) -> Version {
    Version {
        source: SourceId::Modrinth,
        project_id: project_id.to_string(),
        id: id.to_string(),
        name: id.to_string(),
        number: number.to_string(),
        kind: ReleaseKind::Release,
        game_versions: vec!["1.20.1".to_string()],
        loaders: vec!["fabric".to_string()],
        published: "2026-01-01T00:00:00Z".to_string(),
        files: Vec::new(),
        dependencies: Vec::new(),
    }
}

fn file(url: Option<String>, name: &str, bytes: &[u8]) -> VersionFile {
    VersionFile {
        url,
        file_name: name.to_string(),
        size: Some(bytes.len() as u64),
        sha1: Some(sha1_hex(bytes)),
        sha512: None,
        fingerprint: None,
        primary: true,
    }
}

/// A tempdir root holding one Fabric 1.20.1 instance.
fn fixture() -> (tempfile::TempDir, Root, Instance) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    root.ensure_layout().expect("layout");
    let instance = Instances::new(root.clone())
        .create("Test", "1.20.1", Loader::Fabric, None, &BTreeMap::new())
        .expect("create");
    (dir, root, instance)
}

/// Everything `ContentCtx` borrows, kept alive by the caller.
struct Harness {
    http: HttpClient,
    root: Root,
    sink: EventSink,
    rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    cancel: CancellationToken,
    sources: Vec<BoxSource>,
}

impl Harness {
    fn new(root: Root, sources: Vec<BoxSource>) -> Self {
        let (sink, rx) = tokio::sync::mpsc::unbounded_channel();
        Harness {
            http: HttpClient::new().expect("http"),
            root,
            sink,
            rx,
            cancel: CancellationToken::new(),
            sources,
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

    fn logs(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            if let Event::Log { message, .. } = event {
                out.push(message);
            }
        }
        out
    }
}

/// Runs `$body` with `$ctx` bound to a `ContentCtx` that borrows `$h`.
///
/// The `DownloadCtx` it points at is a local of the expanded block, so the whole use of
/// the context has to happen inside the macro.
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

// ---------------------------------------------------------------------------
// compatible_loaders
// ---------------------------------------------------------------------------

#[test]
fn compatible_loaders_table() {
    assert_eq!(compatible_loaders(Loader::Fabric, "1.20.1"), ["fabric"]);
    assert_eq!(
        compatible_loaders(Loader::Quilt, "1.20.1"),
        ["quilt", "fabric"]
    );
    assert_eq!(compatible_loaders(Loader::Forge, "1.20.1"), ["forge"]);
    assert_eq!(
        compatible_loaders(Loader::NeoForge, "1.20.1"),
        ["neoforge", "forge"]
    );
    assert_eq!(compatible_loaders(Loader::NeoForge, "1.21.1"), ["neoforge"]);
    assert!(compatible_loaders(Loader::None, "1.20.1").is_empty());
}

// ---------------------------------------------------------------------------
// pick_version
// ---------------------------------------------------------------------------

#[test]
fn pick_version_prefers_release_then_newest() {
    let mut old = version("p", "v1", "1.0");
    old.published = "2026-01-01T00:00:00Z".to_string();
    let mut newer = version("p", "v2", "2.0");
    newer.published = "2026-05-01T00:00:00Z".to_string();
    let mut beta = version("p", "v3", "3.0-beta");
    beta.kind = ReleaseKind::Beta;
    beta.published = "2026-09-01T00:00:00Z".to_string();

    let picked = pick_version(
        &[old, newer, beta],
        ContentKind::Mod,
        "1.20.1",
        Loader::Fabric,
        None,
    )
    .expect("a version");
    assert_eq!(picked.id, "v2");
}

#[test]
fn pick_version_honors_want_by_id_or_number() {
    let a = version("p", "v1", "1.0");
    let b = version("p", "v2", "2.0");
    let by_id = pick_version(
        &[a.clone(), b.clone()],
        ContentKind::Mod,
        "1.20.1",
        Loader::Fabric,
        Some("v1"),
    )
    .expect("by id");
    assert_eq!(by_id.id, "v1");
    let by_number = pick_version(
        &[a, b],
        ContentKind::Mod,
        "1.20.1",
        Loader::Fabric,
        Some("1.0"),
    )
    .expect("by number");
    assert_eq!(by_number.id, "v1");
}

#[test]
fn pick_version_filters_minecraft_and_loader_for_mods() {
    let mut wrong_mc = version("p", "v1", "1.0");
    wrong_mc.game_versions = vec!["1.19.2".to_string()];
    let mut wrong_loader = version("p", "v2", "2.0");
    wrong_loader.loaders = vec!["forge".to_string()];
    assert!(
        pick_version(
            &[wrong_mc, wrong_loader],
            ContentKind::Mod,
            "1.20.1",
            Loader::Fabric,
            None
        )
        .is_none()
    );
}

#[test]
fn pick_version_ignores_loaders_for_resource_packs() {
    let mut pack = version("p", "v1", "1.0");
    pack.loaders = vec!["minecraft".to_string()];
    let picked = pick_version(
        &[pack],
        ContentKind::ResourcePack,
        "1.20.1",
        Loader::Fabric,
        None,
    )
    .expect("a version");
    assert_eq!(picked.id, "v1");
}

#[test]
fn pick_version_accepts_datapacks_with_or_without_a_datapack_loader() {
    let mut tagged = version("p", "v1", "1.0");
    tagged.loaders = vec!["datapack".to_string()];
    let mut bare = version("p", "v2", "2.0");
    bare.loaders = Vec::new();
    let mut mod_only = version("p", "v3", "3.0");
    mod_only.loaders = vec!["fabric".to_string()];

    for v in [tagged, bare] {
        assert!(
            pick_version(
                std::slice::from_ref(&v),
                ContentKind::DataPack,
                "1.20.1",
                Loader::Fabric,
                None
            )
            .is_some(),
            "{} should be accepted",
            v.id
        );
    }
    assert!(
        pick_version(
            &[mod_only],
            ContentKind::DataPack,
            "1.20.1",
            Loader::Fabric,
            None
        )
        .is_none()
    );
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

/// Two mods, `alpha` requiring `beta`, both served by one wiremock server.
async fn two_mod_source(server: &MockServer) -> BoxSource {
    let alpha_bytes = b"alpha jar bytes".as_slice();
    let beta_bytes = b"beta jar bytes".as_slice();
    for (route, bytes) in [("/alpha.jar", alpha_bytes), ("/beta.jar", beta_bytes)] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
            .mount(server)
            .await;
    }

    let mut alpha = version("alpha", "av1", "1.0");
    alpha.files = vec![file(
        Some(format!("{}/alpha.jar", server.uri())),
        "alpha.jar",
        alpha_bytes,
    )];
    alpha.dependencies = vec![Dependency {
        project_id: Some("beta".to_string()),
        version_id: None,
        kind: DependencyKind::Required,
    }];
    let mut beta = version("beta", "bv1", "1.0");
    beta.files = vec![file(
        Some(format!("{}/beta.jar", server.uri())),
        "beta.jar",
        beta_bytes,
    )];

    FakeSource::new(SourceId::Modrinth)
        .with(project("alpha", ContentKind::Mod), vec![alpha])
        .with(project("beta", ContentKind::Mod), vec![beta])
        .boxed()
}

fn request(project: &str) -> AddRequest {
    AddRequest {
        source: SourceId::Modrinth,
        project: project.to_string(),
        version: None,
        kind: None,
        world: None,
    }
}

#[tokio::test]
async fn add_installs_a_mod_and_its_required_dependency() {
    let server = MockServer::start().await;
    let (_dir, root, mut instance) = fixture();
    let mut h = Harness::new(root, vec![two_mod_source(&server).await]);

    let out = with_ctx!(h, |ctx| {
        add(&ctx, &mut instance, request("alpha"))
            .await
            .expect("add")
    });

    assert_eq!(out.installed.len(), 2, "{:?}", out.installed);
    assert!(out.skipped.is_empty());
    assert!(out.manual.is_empty());
    let mods = instance.game_dir().join("mods");
    assert!(mods.join("alpha.jar").is_file());
    assert!(mods.join("beta.jar").is_file());
    assert_eq!(instance.config.content.len(), 2);
    let entry = &instance.config.content[0];
    assert_eq!(entry.project_id, "alpha");
    assert_eq!(entry.version_id, "av1");
    assert_eq!(
        entry.sha1.as_deref(),
        Some(sha1_hex(b"alpha jar bytes")).as_deref()
    );
    assert!(entry.enabled);
    assert!(
        entry.fingerprint.is_none(),
        "modrinth entries carry no fingerprint"
    );

    // The file was reloaded from disk, so the config really was saved.
    let reread = Instances::new(Root::from_path(_dir.path()))
        .get("test")
        .expect("reload");
    assert_eq!(reread.config.content.len(), 2);

    // Second add skips both: the project and its dependency are already installed.
    let again = with_ctx!(h, |ctx| {
        add(&ctx, &mut instance, request("alpha"))
            .await
            .expect("add")
    });
    assert!(again.installed.is_empty());
    assert_eq!(again.skipped, vec!["alpha".to_string(), "beta".to_string()]);
    assert!(h.logs().iter().any(|m| m.contains("alpha")));
}

#[tokio::test]
async fn add_returns_a_manual_download_when_the_file_has_no_url() {
    let (_dir, root, mut instance) = fixture();
    let mut only = version("gated", "gv1", "1.0");
    only.source = SourceId::CurseForge;
    only.files = vec![VersionFile {
        url: None,
        file_name: "gated.jar".to_string(),
        size: None,
        sha1: None,
        sha512: None,
        fingerprint: Some(42),
        primary: true,
    }];
    let mut gated = project("gated", ContentKind::Mod);
    gated.source = SourceId::CurseForge;
    gated.page_url = "https://www.curseforge.com/minecraft/mc-mods/gated".to_string();
    let source = FakeSource::new(SourceId::CurseForge)
        .with(gated, vec![only])
        .boxed();
    let mut h = Harness::new(root, vec![source]);

    let out = with_ctx!(h, |ctx| {
        add(
            &ctx,
            &mut instance,
            AddRequest {
                source: SourceId::CurseForge,
                ..request("gated")
            },
        )
        .await
        .expect("add")
    });

    assert!(out.installed.is_empty());
    assert_eq!(out.manual.len(), 1);
    let pending = &out.manual[0];
    assert_eq!(pending.file_name, "gated.jar");
    assert_eq!(pending.fingerprint, Some(42));
    assert_eq!(
        pending.page_url,
        "https://www.curseforge.com/minecraft/mc-mods/gated/files/gv1"
    );
    assert!(instance.config.content.is_empty());
    assert!(!instance.game_dir().join("mods").join("gated.jar").exists());
    assert!(h.logs().iter().any(|m| m.contains("by hand")));
}

#[tokio::test]
async fn add_rejects_a_mod_on_a_loaderless_instance() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    root.ensure_layout().expect("layout");
    let mut instance = Instances::new(root.clone())
        .create("Vanilla", "1.20.1", Loader::None, None, &BTreeMap::new())
        .expect("create");
    let h = Harness::new(root, vec![two_mod_source(&server).await]);

    let err = with_ctx!(h, |ctx| {
        add(&ctx, &mut instance, request("alpha"))
            .await
            .expect_err("no loader")
    });
    assert!(matches!(err, Error::NoCompatibleVersion { .. }), "{err:?}");
}

#[tokio::test]
async fn add_reports_an_unknown_source() {
    let (_dir, root, mut instance) = fixture();
    let h = Harness::new(root, Vec::new());
    let err = with_ctx!(h, |ctx| {
        add(&ctx, &mut instance, request("alpha"))
            .await
            .expect_err("no source")
    });
    assert!(
        matches!(err, Error::SourceUnavailable(SourceId::Modrinth)),
        "{err:?}"
    );
}

/// Serves `bytes` at `/<name>` and returns the file that points at it.
async fn served_file(server: &MockServer, name: &str, bytes: &'static [u8]) -> VersionFile {
    Mock::given(method("GET"))
        .and(path(format!("/{name}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
        .mount(server)
        .await;
    file(Some(format!("{}/{name}", server.uri())), name, bytes)
}

/// One required dependency on `project_id`.
fn requires(project_id: &str) -> Dependency {
    Dependency {
        project_id: Some(project_id.to_string()),
        version_id: None,
        kind: DependencyKind::Required,
    }
}

#[tokio::test]
async fn add_terminates_on_a_dependency_cycle_and_installs_each_project_once() {
    let server = MockServer::start().await;
    let (_dir, root, mut instance) = fixture();

    // A requires B, and B requires A right back.
    let mut a = version("cycle-a", "av1", "1.0");
    a.files = vec![served_file(&server, "cycle-a.jar", b"a bytes").await];
    a.dependencies = vec![requires("cycle-b")];
    let mut b = version("cycle-b", "bv1", "1.0");
    b.files = vec![served_file(&server, "cycle-b.jar", b"b bytes").await];
    b.dependencies = vec![requires("cycle-a")];
    let source = FakeSource::new(SourceId::Modrinth)
        .with(project("cycle-a", ContentKind::Mod), vec![a])
        .with(project("cycle-b", ContentKind::Mod), vec![b])
        .boxed();
    let h = Harness::new(root, vec![source]);

    // The guard turns a hang into a failure instead of a stuck suite.
    let out = with_ctx!(h, |ctx| tokio::time::timeout(
        std::time::Duration::from_secs(10),
        add(&ctx, &mut instance, request("cycle-a")),
    )
    .await
    .expect("the walk terminates")
    .expect("add"));

    assert_eq!(out.installed.len(), 2, "{:?}", out.installed);
    assert_eq!(instance.config.content.len(), 2);
    let mods = instance.game_dir().join("mods");
    assert!(mods.join("cycle-a.jar").is_file());
    assert!(mods.join("cycle-b.jar").is_file());
}

#[tokio::test]
async fn add_gives_up_on_a_chain_deeper_than_the_limit() {
    let server = MockServer::start().await;
    let (_dir, root, mut instance) = fixture();

    // `dep-0` needs `dep-1` needs ... needs `dep-12`: past MAX_DEPENDENCY_DEPTH.
    let last = MAX_DEPENDENCY_DEPTH + 2;
    let mut source = FakeSource::new(SourceId::Modrinth);
    for step in 0..=last {
        let id = format!("dep-{step}");
        let mut v = version(&id, &format!("{id}-v1"), "1.0");
        v.files = vec![served_file(&server, &format!("{id}.jar"), b"chain bytes").await];
        if step < last {
            v.dependencies = vec![requires(&format!("dep-{}", step + 1))];
        }
        source = source.with(project(&id, ContentKind::Mod), vec![v]);
    }
    let h = Harness::new(root, vec![source.boxed()]);

    let err = with_ctx!(h, |ctx| add(&ctx, &mut instance, request("dep-0"))
        .await
        .expect_err("too deep"));
    assert!(
        matches!(err, Error::DependencyDepth(ref p) if p == &format!("dep-{}", MAX_DEPENDENCY_DEPTH + 1)),
        "{err:?}"
    );
}

#[tokio::test]
async fn add_installs_a_resource_pack_dependency_into_resourcepacks() {
    let server = MockServer::start().await;
    let (_dir, root, mut instance) = fixture();

    let mut host = version("host-mod", "hv1", "1.0");
    host.files = vec![served_file(&server, "host-mod.jar", b"host bytes").await];
    host.dependencies = vec![requires("needed-pack")];
    let mut pack = version("needed-pack", "pv1", "1.0");
    // A resource pack lists no mod loader; `pick_version` ignores loaders for it.
    pack.loaders = vec!["minecraft".to_string()];
    pack.files = vec![served_file(&server, "needed-pack.zip", b"pack bytes").await];
    let source = FakeSource::new(SourceId::Modrinth)
        .with(project("host-mod", ContentKind::Mod), vec![host])
        .with(
            project("needed-pack", ContentKind::ResourcePack),
            vec![pack],
        )
        .boxed();
    let h = Harness::new(root, vec![source]);

    let out = with_ctx!(h, |ctx| add(&ctx, &mut instance, request("host-mod"))
        .await
        .expect("add"));

    assert_eq!(out.installed.len(), 2, "{:?}", out.installed);
    assert!(
        instance
            .game_dir()
            .join("resourcepacks/needed-pack.zip")
            .is_file()
    );
    let entry = instance
        .config
        .content
        .iter()
        .find(|e| e.project_id == "needed-pack")
        .expect("the pack is recorded");
    assert_eq!(entry.kind, ContentKind::ResourcePack);
}

// ---------------------------------------------------------------------------
// check_updates / apply_update
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_updates_finds_a_newer_version_and_applies_it() {
    let server = MockServer::start().await;
    let (_dir, root, mut instance) = fixture();
    let mut h = Harness::new(root, vec![two_mod_source(&server).await]);
    with_ctx!(h, |ctx| {
        add(&ctx, &mut instance, request("alpha"))
            .await
            .expect("add");
    });

    // Publish a newer alpha, served from a second route.
    let newer_bytes = b"alpha jar v2".as_slice();
    Mock::given(method("GET"))
        .and(path("/alpha2.jar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(newer_bytes.to_vec()))
        .mount(&server)
        .await;
    let mut av2 = version("alpha", "av2", "2.0");
    av2.published = "2026-06-01T00:00:00Z".to_string();
    av2.files = vec![file(
        Some(format!("{}/alpha2.jar", server.uri())),
        "alpha-2.jar",
        newer_bytes,
    )];
    let mut beta = version("beta", "bv1", "1.0");
    beta.files = vec![file(
        Some(format!("{}/beta.jar", server.uri())),
        "beta.jar",
        b"beta jar bytes",
    )];
    let mut alpha_old = version("alpha", "av1", "1.0");
    alpha_old.files = vec![file(
        Some(format!("{}/alpha.jar", server.uri())),
        "alpha.jar",
        b"alpha jar bytes",
    )];
    h.sources = vec![
        FakeSource::new(SourceId::Modrinth)
            .with(project("alpha", ContentKind::Mod), vec![alpha_old, av2])
            .with(project("beta", ContentKind::Mod), vec![beta])
            .boxed(),
    ];

    let candidates = with_ctx!(h, |ctx| {
        check_updates(&ctx, &instance).await.expect("check")
    });
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(candidates[0].entry.project_id, "alpha");
    assert_eq!(candidates[0].new.id, "av2");

    let applied = with_ctx!(h, |ctx| {
        apply_update(&ctx, &mut instance, &candidates[0])
            .await
            .expect("apply")
    });
    assert_eq!(applied.installed.len(), 1);
    let mods = instance.game_dir().join("mods");
    assert!(mods.join("alpha-2.jar").is_file());
    assert!(!mods.join("alpha.jar").exists(), "the old jar is replaced");
    assert_eq!(instance.config.content.len(), 2);
}

#[tokio::test]
async fn check_updates_skips_an_entry_whose_source_is_missing() {
    let (_dir, root, mut instance) = fixture();
    instance.config.content.push(ContentEntry {
        source: "curseforge".to_string(),
        project_id: "ghost".to_string(),
        version_id: "gv1".to_string(),
        file_name: "ghost.jar".to_string(),
        kind: ContentKind::Mod,
        ..ContentEntry::default()
    });
    let h = Harness::new(root, Vec::new());
    let candidates = with_ctx!(h, |ctx| {
        check_updates(&ctx, &instance).await.expect("check")
    });
    assert!(candidates.is_empty());
}

// ---------------------------------------------------------------------------
// import_manual
// ---------------------------------------------------------------------------

fn pending_for(bytes: &[u8]) -> ManualDownload {
    ManualDownload {
        source: SourceId::CurseForge,
        project_id: "gated".to_string(),
        version_id: "gv1".to_string(),
        file_name: "gated.jar".to_string(),
        page_url: "https://www.curseforge.com/minecraft/mc-mods/gated/files/gv1".to_string(),
        fingerprint: Some(curseforge_fingerprint(bytes)),
        sha1: None,
        world: None,
    }
}

#[tokio::test]
async fn import_manual_rejects_a_file_with_the_wrong_fingerprint() {
    let (dir, root, mut instance) = fixture();
    let dropped = dir.path().join("dropped.jar");
    std::fs::write(&dropped, b"not the right bytes").expect("write");
    let mut pending = pending_for(b"the right bytes");
    pending.fingerprint = Some(curseforge_fingerprint(b"the right bytes"));
    let h = Harness::new(root, Vec::new());

    let err = with_ctx!(h, |ctx| {
        import_manual(&ctx, &mut instance, &pending, &dropped, ContentKind::Mod)
            .await
            .expect_err("mismatch")
    });
    assert!(
        matches!(err, Error::VerificationFailed { ref file, .. } if file.as_path() == dropped.as_path()),
        "{err:?}"
    );
    assert!(instance.config.content.is_empty());
}

#[tokio::test]
async fn import_manual_places_a_file_with_the_right_fingerprint() {
    let (dir, root, mut instance) = fixture();
    let bytes = b"the right bytes".as_slice();
    let dropped = dir.path().join("dropped.jar");
    std::fs::write(&dropped, bytes).expect("write");
    let pending = pending_for(bytes);
    let h = Harness::new(root, Vec::new());

    let entry = with_ctx!(h, |ctx| {
        import_manual(&ctx, &mut instance, &pending, &dropped, ContentKind::Mod)
            .await
            .expect("import")
    });

    assert_eq!(entry.file_name, "gated.jar");
    assert_eq!(entry.sha1.as_deref(), Some(sha1_hex(bytes)).as_deref());
    assert_eq!(entry.fingerprint, Some(curseforge_fingerprint(bytes)));
    assert!(instance.game_dir().join("mods").join("gated.jar").is_file());
    assert_eq!(instance.config.content.len(), 1);
    // The object store gained the file, keyed by its sha1.
    let object = Root::from_path(dir.path())
        .object_path(&sha1_hex(bytes))
        .expect("object path");
    assert!(object.is_file());
}

#[tokio::test]
async fn import_manual_accepts_a_file_when_nothing_can_be_verified() {
    let (dir, root, mut instance) = fixture();
    let dropped = dir.path().join("dropped.jar");
    std::fs::write(&dropped, b"whatever").expect("write");
    let mut pending = pending_for(b"whatever");
    pending.fingerprint = None;
    pending.sha1 = None;
    let h = Harness::new(root, Vec::new());

    let entry = with_ctx!(h, |ctx| {
        import_manual(&ctx, &mut instance, &pending, &dropped, ContentKind::Mod)
            .await
            .expect("import")
    });
    assert_eq!(entry.project_id, "gated");
}

#[tokio::test]
async fn import_manual_puts_a_data_pack_in_the_pending_world() {
    let (dir, root, mut instance) = fixture();
    let bytes = b"data pack bytes".as_slice();
    let dropped = dir.path().join("dropped.zip");
    std::fs::write(&dropped, bytes).expect("write");
    let mut pending = pending_for(bytes);
    pending.file_name = "pack.zip".to_string();
    pending.world = Some("w".to_string());
    let h = Harness::new(root, Vec::new());

    let entry = with_ctx!(h, |ctx| {
        import_manual(
            &ctx,
            &mut instance,
            &pending,
            &dropped,
            ContentKind::DataPack,
        )
        .await
        .expect("import")
    });

    assert_eq!(entry.world.as_deref(), Some("w"));
    assert!(
        instance
            .game_dir()
            .join("saves/w/datapacks/pack.zip")
            .is_file()
    );
}
