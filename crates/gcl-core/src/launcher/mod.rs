//! `Launcher`: the handle every binary goes through.
//!
//! It owns the one tokio runtime, the app root, the config, the HTTP client, the event
//! channel, and the cancellation token. Binaries never build a runtime of their own.

use std::future::Future;
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::download::{DownloadCtx, cleanup_partials};
use crate::events::{Event, EventSink};
use crate::http::HttpClient;
use crate::instances::Instances;
use crate::java::{
    JavaInstall, RUNTIME_MANIFEST, component_for_major, detect_all, install_runtime, pick,
};
use crate::mojang::{InstallPlan, Mojang, PISTON_META, VersionManifest, install_version};
use crate::paths::Root;

/// Test-only override for the Mojang metadata base URL, read by [`Launcher::mojang`].
pub const MOJANG_BASE_URL_ENV: &str = "GCL_MOJANG_BASE_URL";

/// Owns the runtime and every shared handle the rest of the launcher needs.
pub struct Launcher {
    runtime: tokio::runtime::Runtime,
    root: Root,
    config: Config,
    http: HttpClient,
    events: EventSink,
    cancel: CancellationToken,
}

impl Launcher {
    /// Builds the runtime, resolves the root, loads config, and prepares the app layout.
    ///
    /// Returns the handle and the receiving end of the event channel.
    pub fn new(
        root_override: Option<PathBuf>,
    ) -> Result<(Launcher, tokio::sync::mpsc::UnboundedReceiver<Event>), crate::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("gcl")
            .enable_all()
            .build()?;

        let overridden = root_override.is_some() || std::env::var_os("GCL_ROOT").is_some();
        let root = Root::resolve(root_override.as_deref())?;
        let config = Config::load(&root.config_file())?;
        let root = redirect_root(root, config.root.as_deref(), overridden);

        root.ensure_layout()?;
        match cleanup_partials(&root) {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "removed stale partial downloads"),
            Err(source) => tracing::warn!(%source, "could not sweep partial downloads"),
        }

        let http = HttpClient::new()?;
        let (events, receiver) = tokio::sync::mpsc::unbounded_channel();
        let launcher = Launcher {
            runtime,
            root,
            config,
            http,
            events,
            cancel: CancellationToken::new(),
        };
        Ok((launcher, receiver))
    }

    /// The resolved app root.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// The loaded config.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Mutable access to the config, for settings edits before [`Launcher::save_config`].
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Writes the config back to `config.toml` in the app root.
    pub fn save_config(&self) -> Result<(), crate::Error> {
        self.config.save(&self.root.config_file())?;
        Ok(())
    }

    /// The shared HTTP client.
    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    /// The sink long-running operations publish progress to.
    pub fn events(&self) -> &EventSink {
        &self.events
    }

    /// A clone of the token that cancels in-flight work.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Runs a future to completion on the owned runtime.
    pub fn block_on<F: Future>(&self, f: F) -> F::Output {
        self.runtime.block_on(f)
    }

    /// A Mojang metadata client over this root, honouring the test-only base URL override.
    pub fn mojang(&self) -> Mojang {
        let base = std::env::var(MOJANG_BASE_URL_ENV).unwrap_or_else(|_| PISTON_META.to_string());
        Mojang::with_base_url(self.http.clone(), self.root.clone(), base)
    }

    /// The instance store over this root.
    pub fn instances(&self) -> Instances {
        Instances::new(self.root.clone())
    }

    /// A download context bound to this root, client, event sink, and cancel token.
    pub fn download_ctx(&self) -> DownloadCtx<'_> {
        DownloadCtx {
            http: &self.http,
            root: &self.root,
            sink: &self.events,
            cancel: &self.cancel,
            parallel: self.config.parallel_downloads,
        }
    }

    /// Fetches the Mojang version manifest. Blocks.
    pub fn list_versions(&self) -> Result<VersionManifest, crate::Error> {
        let mojang = self.mojang();
        Ok(self.block_on(async move { mojang.manifest().await })?)
    }

    /// Installs a vanilla version: metadata, libraries, assets, and natives. Blocks.
    pub fn install_version(&self, id: &str) -> Result<InstallPlan, crate::Error> {
        let mojang = self.mojang();
        let ctx = self.download_ctx();
        Ok(self.block_on(async move { install_version(&mojang, &ctx, id).await })?)
    }

    /// Lists every Java runtime found on this machine. Blocks.
    pub fn detect_java(&self) -> Vec<JavaInstall> {
        let root = self.root.clone();
        self.block_on(async move { detect_all(&root).await })
    }

    /// Returns a Java runtime of the given major version, installing Mojang's if none is
    /// present. Blocks.
    pub fn ensure_java(&self, major: u32) -> Result<JavaInstall, crate::Error> {
        let root = self.root.clone();
        let http = self.http.clone();
        let ctx = self.download_ctx();
        Ok(self.block_on(async move {
            let found = detect_all(&root).await;
            if let Some(install) = pick(&found, major) {
                return Ok(install.clone());
            }
            let component = component_for_major(major);
            tracing::info!(major, component, "no local java found, installing one");
            install_runtime(&http, &ctx, RUNTIME_MANIFEST, component).await
        })?)
    }
}

impl std::fmt::Debug for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Launcher")
            .field("root", &self.root)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Applies the config's `root` override, unless the env var or a caller override already
/// decided the root.
fn redirect_root(resolved: Root, config_root: Option<&Path>, overridden: bool) -> Root {
    match config_root {
        Some(path) if !overridden && path != resolved.path() && !path.as_os_str().is_empty() => {
            Root::from_path(path)
        }
        _ => resolved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Clears the env vars that would otherwise redirect the root, for the test's lifetime.
    fn clean_env() -> std::sync::MutexGuard<'static, ()> {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by ENV_LOCK; no other thread reads the env while it is held.
        unsafe {
            std::env::remove_var("GCL_ROOT");
        }
        guard
    }

    #[test]
    fn new_creates_the_layout_and_a_default_config() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let (launcher, _rx) =
            Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
        assert_eq!(launcher.root().path(), dir.path());
        assert!(launcher.root().instances_dir().is_dir());
        assert!(launcher.root().objects_dir().is_dir());
        assert_eq!(launcher.config(), &crate::config::Config::default());
    }

    #[test]
    fn saved_config_is_read_back_by_a_later_launcher() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let (mut launcher, _rx) =
                Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
            launcher.config_mut().parallel_downloads = 3;
            launcher
                .config_mut()
                .game_defaults
                .insert("renderDistance".to_string(), "12".to_string());
            launcher.save_config().expect("save config");
        }
        let (launcher, _rx) =
            Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
        assert_eq!(launcher.config().parallel_downloads, 3);
        assert_eq!(
            launcher.config().game_defaults.get("renderDistance"),
            Some(&"12".to_string())
        );
    }

    #[test]
    fn config_root_redirects_the_root_when_nothing_else_does() {
        let resolved = Root::from_path("/resolved");
        let redirected = redirect_root(resolved, Some(Path::new("/from-config")), false);
        assert_eq!(redirected.path(), Path::new("/from-config"));
    }

    #[test]
    fn config_root_is_ignored_when_the_root_was_overridden() {
        let resolved = Root::from_path("/from-env");
        let kept = redirect_root(resolved, Some(Path::new("/from-config")), true);
        assert_eq!(kept.path(), Path::new("/from-env"));
    }

    #[test]
    fn env_wins_over_an_explicit_override() {
        let _guard = clean_env();
        let env_dir = tempfile::tempdir().expect("tempdir");
        let arg_dir = tempfile::tempdir().expect("tempdir");
        // SAFETY: guarded by ENV_LOCK through `clean_env`; removed before the assert.
        unsafe {
            std::env::set_var("GCL_ROOT", env_dir.path());
        }
        let built = Launcher::new(Some(arg_dir.path().to_path_buf()));
        unsafe {
            std::env::remove_var("GCL_ROOT");
        }
        let (launcher, _rx) = built.expect("build launcher");
        assert_eq!(launcher.root().path(), env_dir.path());
    }

    #[test]
    fn cleanup_partials_runs_on_startup() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let stale = dir
            .path()
            .join("cache")
            .join("libraries")
            .join("a.jar.part");
        std::fs::create_dir_all(stale.parent().expect("parent")).expect("mkdir");
        std::fs::write(&stale, b"partial").expect("write");
        let (_launcher, _rx) =
            Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
        assert!(!stale.exists());
    }

    #[test]
    fn events_reach_the_returned_receiver() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let (launcher, mut rx) =
            Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
        launcher
            .events()
            .send(crate::events::Event::Warning("hi".to_string()))
            .expect("send");
        assert_eq!(
            rx.try_recv().expect("event"),
            crate::events::Event::Warning("hi".to_string())
        );
    }

    #[test]
    fn cancel_token_is_shared_with_the_download_context() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let (launcher, _rx) =
            Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
        let ctx = launcher.download_ctx();
        assert_eq!(ctx.parallel, launcher.config().parallel_downloads);
        assert!(!ctx.cancel.is_cancelled());
        launcher.cancel_token().cancel();
        assert!(launcher.download_ctx().cancel.is_cancelled());
    }

    #[test]
    fn block_on_runs_a_future_on_the_owned_runtime() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let (launcher, _rx) =
            Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
        assert_eq!(launcher.block_on(async { 2 + 2 }), 4);
    }

    #[test]
    fn instances_are_created_under_the_launcher_root() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let (launcher, _rx) =
            Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
        let created = launcher
            .instances()
            .create(
                "Pack",
                "1.20.1",
                crate::instances::model::Loader::None,
                None,
                &launcher.config().game_defaults,
            )
            .expect("create");
        assert_eq!(created.dir, launcher.root().instance_dir("pack"));
    }
}
