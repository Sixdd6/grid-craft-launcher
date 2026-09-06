//! `Launcher`: the handle every binary goes through.
//!
//! It owns the one tokio runtime, the app root, the config, the HTTP client, the event
//! channel, and the cancellation token. Binaries never build a runtime of their own.

use std::future::Future;
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use crate::auth::offline::offline_account;
use crate::auth::store::Accounts;
use crate::config::Config;
use crate::download::{DownloadCtx, cleanup_partials};
use crate::events::{Event, EventSink};
use crate::http::HttpClient;
use crate::instances::model::Loader;
use crate::instances::{Instance, Instances, now_rfc3339};
use crate::java::{
    JavaInstall, RUNTIME_MANIFEST, component_for_major, detect_all, install_runtime, pick,
};
use crate::launch::{JvmSettings, LaunchCommand, LaunchInputs};
use crate::loaders::{JavaRunner, LoaderCtx, LoaderEndpoints, LoaderVersion, keep_both_libraries};
use crate::mojang::assets::RESOURCES_BASE;
use crate::mojang::rules::RuleContext;
use crate::mojang::{InstallPlan, Mojang, PISTON_META, VersionManifest, install_version};
use crate::paths::Root;

/// Test-only override for the Mojang metadata base URL, read by [`Launcher::mojang`].
pub const MOJANG_BASE_URL_ENV: &str = "GCL_MOJANG_BASE_URL";

/// Test-only override for the Fabric meta base URL, read by [`Launcher::loader_endpoints`].
pub const FABRIC_BASE_URL_ENV: &str = "GCL_FABRIC_BASE_URL";

/// Test-only override for the Quilt meta base URL, read by [`Launcher::loader_endpoints`].
pub const QUILT_BASE_URL_ENV: &str = "GCL_QUILT_BASE_URL";

/// Test-only override for the Forge metadata host, read by [`Launcher::loader_endpoints`].
pub const FORGE_META_BASE_URL_ENV: &str = "GCL_FORGE_META_BASE_URL";

/// Test-only override for the Forge maven host, read by [`Launcher::loader_endpoints`].
pub const FORGE_MAVEN_BASE_URL_ENV: &str = "GCL_FORGE_MAVEN_BASE_URL";

/// Test-only override for the NeoForge maven host, read by [`Launcher::loader_endpoints`].
pub const NEOFORGE_BASE_URL_ENV: &str = "GCL_NEOFORGE_BASE_URL";

/// Every metadata host the launcher talks to.
///
/// [`Launcher::new`] fills it from the test-only environment overrides;
/// [`Launcher::open_with_endpoints`] takes one whole, and reads no environment at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Mojang metadata base URL.
    pub mojang: String,
    /// Loader metadata and maven hosts.
    pub loaders: LoaderEndpoints,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            mojang: PISTON_META.to_string(),
            loaders: LoaderEndpoints::default(),
        }
    }
}

impl Endpoints {
    /// Reads every test-only base URL override, falling back to the production hosts.
    pub fn from_env() -> Endpoints {
        Endpoints {
            mojang: env_base(MOJANG_BASE_URL_ENV, PISTON_META),
            loaders: LoaderEndpoints {
                fabric: env_base(FABRIC_BASE_URL_ENV, crate::loaders::fabric::BASE),
                quilt: env_base(QUILT_BASE_URL_ENV, crate::loaders::quilt::BASE),
                forge_meta: env_base(FORGE_META_BASE_URL_ENV, crate::loaders::forge::META),
                forge_maven: env_base(FORGE_MAVEN_BASE_URL_ENV, crate::loaders::forge::MAVEN),
                neoforge: env_base(NEOFORGE_BASE_URL_ENV, crate::loaders::neoforge::MAVEN),
            },
        }
    }
}

/// The `${launcher_name}` every launch command reports.
pub const LAUNCHER_NAME: &str = "grid-craft-launcher";

/// What [`Launcher::launch_instance`] produced.
#[derive(Debug, Clone, PartialEq)]
pub enum LaunchOutcome {
    /// The command line that would have been run. Nothing was started.
    DryRun(LaunchCommand),
    /// The game ran to completion.
    Exited {
        /// Exit code the game returned. A process killed by a signal reports `-1`.
        code: i32,
        /// File both output streams were written to.
        log_path: PathBuf,
    },
}

/// Owns the runtime and every shared handle the rest of the launcher needs.
pub struct Launcher {
    runtime: tokio::runtime::Runtime,
    root: Root,
    config: Config,
    http: HttpClient,
    events: EventSink,
    cancel: CancellationToken,
    endpoints: Endpoints,
}

impl Launcher {
    /// Builds the runtime, resolves the root, loads config, and prepares the app layout.
    ///
    /// Returns the handle and the receiving end of the event channel.
    pub fn new(
        root_override: Option<PathBuf>,
    ) -> Result<(Launcher, tokio::sync::mpsc::UnboundedReceiver<Event>), crate::Error> {
        let overridden = root_override.is_some() || std::env::var_os("GCL_ROOT").is_some();
        let resolved = Root::resolve(root_override.as_deref())?;
        Self::open(resolved, overridden, Endpoints::from_env())
    }

    /// Builds a launcher over an already-resolved root.
    ///
    /// `overridden` says whether `GCL_ROOT` or a caller override decided the root, in which
    /// case the config's own `root` is ignored. `endpoints` are used as given: this function
    /// reads no environment of its own.
    fn open(
        resolved: Root,
        overridden: bool,
        endpoints: Endpoints,
    ) -> Result<(Launcher, tokio::sync::mpsc::UnboundedReceiver<Event>), crate::Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("gcl")
            .enable_all()
            .build()?;

        let mut config = Config::load(&resolved.config_file())?;
        let root = redirect_root(resolved.clone(), config.root.as_deref(), overridden);
        if root != resolved && root.config_file().is_file() {
            // The redirected root has its own config.toml. It is the one the user edits and
            // the one `save_config` writes, so it wins over the config that pointed here.
            config = Config::load(&root.config_file())?;
        }

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
            endpoints,
        };
        Ok((launcher, receiver))
    }

    /// Test seam: a launcher over `root` with the given metadata hosts.
    ///
    /// Nothing here reads the environment, so a test points one launcher at a mock server
    /// without touching process env and without disturbing another test. The root is treated
    /// as overridden, exactly as an explicit root passed to [`Launcher::new`] is.
    pub fn open_with_endpoints(
        root: PathBuf,
        endpoints: Endpoints,
    ) -> Result<(Launcher, tokio::sync::mpsc::UnboundedReceiver<Event>), crate::Error> {
        Self::open(Root::from_path(root), true, endpoints)
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
        Mojang::with_base_url(
            self.http.clone(),
            self.root.clone(),
            self.endpoints.mojang.clone(),
        )
    }

    /// The loader metadata and maven hosts this launcher talks to.
    pub fn loader_endpoints(&self) -> LoaderEndpoints {
        self.endpoints.loaders.clone()
    }

    /// The account store over this root.
    pub fn accounts(&self) -> Accounts {
        Accounts::new(&self.root)
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
        self.ensure_java_component(major, None)
    }

    /// Returns a Java runtime for an install plan, installing Mojang's if none is present.
    ///
    /// It asks for the runtime component the version JSON names, and only guesses from the
    /// major version when the version JSON names none. Blocks.
    pub fn ensure_java_for(&self, plan: &InstallPlan) -> Result<JavaInstall, crate::Error> {
        self.ensure_java_component(plan.java_major, plan.java_component.as_deref())
    }

    /// Returns a Java runtime for a Minecraft version, without installing the game.
    ///
    /// It fetches and plans the version to read its `javaVersion`, which downloads nothing,
    /// then hands the plan to [`Launcher::ensure_java_for`]. Blocks.
    pub fn java_for_version(&self, mc: &str) -> Result<JavaInstall, crate::Error> {
        let mojang = self.mojang();
        let root = self.root.clone();
        let plan = self.block_on(async move {
            let resolved = mojang.resolve_auto(mojang.version(mc).await?)?;
            crate::mojang::plan_install(&resolved, &root, &RuleContext::current(), None)
        })?;
        self.ensure_java_for(&plan)
    }

    /// Rewrites this instance's `options.txt` keys. Returns how many lines changed.
    pub fn apply_settings_overrides(&self, instance: &Instance) -> Result<usize, crate::Error> {
        Ok(crate::settings::apply_overrides_to(
            &instance.game_dir(),
            &instance.config.settings_overrides,
        )?)
    }

    /// Lists the loader builds available for one Minecraft version, newest first. Blocks.
    pub fn list_loader_versions(
        &self,
        loader: Loader,
        mc: &str,
    ) -> Result<Vec<LoaderVersion>, crate::Error> {
        let endpoints = self.loader_endpoints();
        let dl = self.download_ctx();
        let ctx = self.loader_ctx(&dl, None, None);
        Ok(self.block_on(async move {
            crate::loaders::list_versions(&ctx, &endpoints, loader, mc).await
        })?)
    }

    /// Installs the instance's loader and returns the version id a launch resolves.
    ///
    /// A vanilla instance installs nothing and returns its Minecraft id. Otherwise an unset,
    /// `recommended`, or `latest` `loader_version` is resolved to a concrete build and written
    /// back to `instance.toml`, so a later launch is reproducible. Blocks.
    #[tracing::instrument(skip(self))]
    pub fn install_loader(&self, slug: &str) -> Result<String, crate::Error> {
        let mut instance = self.instances().get(slug)?;
        let loader = instance.config.loader;
        let mc = instance.config.minecraft.clone();
        if loader == Loader::None {
            return Ok(mc);
        }
        let version =
            self.resolve_loader_version(loader, &mc, instance.config.loader_version.as_deref())?;

        // Forge and NeoForge run installer processors, which need a JVM. The vanilla files
        // themselves are fetched by the installer, so only the metadata is needed here.
        let java = match loader {
            Loader::Forge | Loader::NeoForge => Some(self.java_for_version(&mc)?),
            _ => None,
        };
        let endpoints = self.loader_endpoints();
        let mojang = self.mojang();
        let dl = self.download_ctx();
        let runner = JavaRunner;
        let ctx = self.loader_ctx(&dl, java.as_ref(), Some(&runner));
        let ctx = LoaderCtx {
            mojang: Some(&mojang),
            ..ctx
        };
        let requested = version.clone();
        let id = self.block_on(async move {
            crate::loaders::install(&ctx, &endpoints, loader, &mc, &requested).await
        })?;

        if instance.config.loader_version.as_deref() != Some(version.as_str()) {
            instance.config.loader_version = Some(version);
            instance.save()?;
        }
        Ok(id)
    }

    /// Installs everything the instance needs to start: loader, libraries, assets, natives.
    ///
    /// Blocks.
    #[tracing::instrument(skip(self))]
    pub fn install_instance(&self, slug: &str) -> Result<InstallPlan, crate::Error> {
        let instance = self.instances().get(slug)?;
        let loader = instance.config.loader;
        let mc = instance.config.minecraft.clone();
        let id = self.install_loader(slug)?;

        let mojang = self.mojang();
        let dl = self.download_ctx();
        Ok(self.block_on(async move {
            // A loader profile inherits from the vanilla version, so that JSON has to be in
            // the cache before `resolve` can merge the chain.
            if id != mc {
                mojang.version(&mc).await?;
            }
            let profile = mojang.version(&id).await?;
            let resolved = mojang.resolve(profile, keep_both_libraries(loader))?;
            crate::mojang::install_resolved(&dl, resolved, None, RESOURCES_BASE).await
        })?)
    }

    /// Installs the instance if needed and either prints or runs its command line.
    ///
    /// The account is chosen in this order: `offline_user` creates or reuses an offline
    /// account, `account` selects a saved one by id or name, and otherwise the active account
    /// is used. With none of the three this is [`crate::auth::Error::NoAccount`]. Blocks.
    #[tracing::instrument(skip(self))]
    pub fn launch_instance(
        &self,
        slug: &str,
        account: Option<&str>,
        offline_user: Option<&str>,
        dry_run: bool,
    ) -> Result<LaunchOutcome, crate::Error> {
        let accounts = self.accounts();
        let account = match (offline_user, account) {
            (Some(name), _) => accounts.add(offline_account(name))?,
            (None, Some(id_or_name)) => accounts.select(id_or_name)?,
            (None, None) => accounts.active()?.ok_or(crate::auth::Error::NoAccount)?,
        };

        let plan = self.install_instance(slug)?;
        let mut instance = self.instances().get(slug)?;
        let java = match instance
            .config
            .jvm
            .java_path
            .clone()
            .or_else(|| self.config.jvm.java_path.clone())
        {
            Some(path) => path,
            None => self.ensure_java_for(&plan)?.path,
        };
        let changed = self.apply_settings_overrides(&instance)?;
        if changed > 0 {
            tracing::info!(changed, "rewrote options.txt keys");
        }

        let identity = account.launch_identity();
        let rules = RuleContext::current();
        let jvm = JvmSettings {
            min_mib: instance
                .config
                .jvm
                .min_mib
                .unwrap_or(self.config.jvm.min_mib),
            max_mib: instance
                .config
                .jvm
                .max_mib
                .unwrap_or(self.config.jvm.max_mib),
            extra_args: instance.config.jvm.extra_args.clone(),
        };
        let cmd = crate::launch::build(
            &LaunchInputs {
                plan: &plan,
                instance: &instance,
                identity: &identity,
                java: &java,
                root: &self.root,
                jvm,
                rules: &rules,
                launcher_name: LAUNCHER_NAME,
                launcher_version: crate::VERSION,
                resolution: None,
            },
            Some(&self.events),
        )?;
        if dry_run {
            return Ok(LaunchOutcome::DryRun(cmd));
        }

        let log_path = self
            .root
            .logs_dir()
            .join(format!("{slug}-{}.log", now_rfc3339().replace(':', "-")));
        let sink = self.events.clone();
        let target = log_path.clone();
        let game = self.block_on(async move { crate::launch::spawn(&cmd, target, sink).await })?;
        instance.config.last_launched = Some(now_rfc3339());
        instance.save()?;
        let code = self.block_on(async move { crate::launch::wait(game).await })?;
        Ok(LaunchOutcome::Exited { code, log_path })
    }

    /// Turns an unset, `recommended`, or `latest` loader version into a concrete build.
    fn resolve_loader_version(
        &self,
        loader: Loader,
        mc: &str,
        requested: Option<&str>,
    ) -> Result<String, crate::Error> {
        match requested {
            Some(v) if !v.is_empty() && v != "recommended" && v != "latest" => Ok(v.to_string()),
            other => {
                let versions = self.list_loader_versions(loader, mc)?;
                let picked = match other {
                    Some("latest") => versions.first(),
                    _ => versions
                        .iter()
                        .find(|v| v.recommended)
                        .or_else(|| versions.first()),
                }
                .ok_or_else(|| crate::loaders::Error::Unsupported(mc.to_string(), loader))?;
                Ok(picked.version.clone())
            }
        }
    }

    /// A loader context over this root, download context, and optional JVM.
    fn loader_ctx<'a>(
        &'a self,
        dl: &'a DownloadCtx<'a>,
        java: Option<&'a JavaInstall>,
        runner: Option<&'a dyn crate::loaders::ProcessRunner>,
    ) -> LoaderCtx<'a> {
        LoaderCtx {
            http: &self.http,
            root: &self.root,
            dl,
            java,
            runner,
            mojang: None,
        }
    }

    /// Shared body of [`Launcher::ensure_java`] and [`Launcher::ensure_java_for`].
    fn ensure_java_component(
        &self,
        major: u32,
        component: Option<&str>,
    ) -> Result<JavaInstall, crate::Error> {
        let root = self.root.clone();
        let http = self.http.clone();
        let ctx = self.download_ctx();
        let component = component
            .map(str::to_string)
            .unwrap_or_else(|| component_for_major(major).to_string());
        Ok(self.block_on(async move {
            let found = detect_all(&root).await;
            if let Some(install) = pick(&found, major) {
                return Ok(install.clone());
            }
            tracing::info!(major, component, "no local java found, installing one");
            install_runtime(&http, &ctx, RUNTIME_MANIFEST, &component).await
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

/// Reads a test-only base URL override, falling back to the production endpoint.
fn env_base(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
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
    fn a_redirected_root_keeps_the_settings_saved_there() {
        let outer = tempfile::tempdir().expect("tempdir");
        let target = tempfile::tempdir().expect("tempdir");
        let pointer = crate::config::Config {
            root: Some(target.path().to_path_buf()),
            ..crate::config::Config::default()
        };
        pointer
            .save(&outer.path().join("config.toml"))
            .expect("save pointer config");

        let (mut launcher, _rx) =
            Launcher::open(Root::from_path(outer.path()), false, Endpoints::default())
                .expect("first launcher");
        assert_eq!(launcher.root().path(), target.path());
        launcher.config_mut().jvm.max_mib = 8192;
        launcher.save_config().expect("save config");

        let (again, _rx) =
            Launcher::open(Root::from_path(outer.path()), false, Endpoints::default())
                .expect("second launcher");
        assert_eq!(again.root().path(), target.path());
        assert_eq!(again.config().jvm.max_mib, 8192);
    }

    #[test]
    fn a_redirected_root_without_a_config_keeps_the_pointing_config() {
        let outer = tempfile::tempdir().expect("tempdir");
        let target = tempfile::tempdir().expect("tempdir");
        let pointer = crate::config::Config {
            root: Some(target.path().to_path_buf()),
            parallel_downloads: 2,
            ..crate::config::Config::default()
        };
        pointer
            .save(&outer.path().join("config.toml"))
            .expect("save pointer config");

        let (launcher, _rx) =
            Launcher::open(Root::from_path(outer.path()), false, Endpoints::default())
                .expect("launcher");
        assert_eq!(launcher.root().path(), target.path());
        assert_eq!(launcher.config().parallel_downloads, 2);
        assert_eq!(launcher.config().root.as_deref(), Some(target.path()));
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

    /// A launcher over a fresh root with explicit, non-production loader hosts.
    fn seamed(dir: &tempfile::TempDir) -> Launcher {
        let endpoints = Endpoints {
            mojang: "http://mojang.invalid".to_string(),
            loaders: LoaderEndpoints {
                fabric: "http://fabric.invalid".to_string(),
                quilt: "http://quilt.invalid".to_string(),
                forge_meta: "http://forge-meta.invalid".to_string(),
                forge_maven: "http://forge-maven.invalid".to_string(),
                neoforge: "http://neoforge.invalid".to_string(),
            },
        };
        let (launcher, _rx) = Launcher::open_with_endpoints(dir.path().to_path_buf(), endpoints)
            .expect("build launcher");
        launcher
    }

    #[test]
    fn open_with_endpoints_ignores_the_environment() {
        let _guard = clean_env();
        // SAFETY: guarded by ENV_LOCK; removed before the assert.
        unsafe {
            std::env::set_var(FABRIC_BASE_URL_ENV, "http://from-env.invalid");
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let built = Launcher::open_with_endpoints(dir.path().to_path_buf(), Endpoints::default());
        unsafe {
            std::env::remove_var(FABRIC_BASE_URL_ENV);
        }
        let (launcher, _rx) = built.expect("build launcher");
        assert_eq!(launcher.loader_endpoints(), LoaderEndpoints::default());
    }

    #[test]
    fn from_env_reads_a_loader_override_and_defaults_the_rest() {
        let _guard = clean_env();
        // SAFETY: guarded by ENV_LOCK; both variables are restored before the assert.
        unsafe {
            std::env::remove_var(FABRIC_BASE_URL_ENV);
            std::env::set_var(QUILT_BASE_URL_ENV, "http://quilt-from-env.invalid");
        }
        let endpoints = Endpoints::from_env();
        unsafe {
            std::env::remove_var(QUILT_BASE_URL_ENV);
        }
        assert_eq!(endpoints.loaders.quilt, "http://quilt-from-env.invalid");
        assert_eq!(endpoints.loaders.fabric, crate::loaders::fabric::BASE);
    }

    #[test]
    fn env_base_falls_back_when_the_variable_is_unset_or_blank() {
        assert_eq!(env_base("GCL_DEFINITELY_UNSET_BASE_URL", "prod"), "prod");
    }

    #[test]
    fn open_with_endpoints_replaces_the_metadata_hosts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        assert_eq!(launcher.loader_endpoints().fabric, "http://fabric.invalid");
        assert_eq!(
            launcher.loader_endpoints().neoforge,
            "http://neoforge.invalid"
        );
        assert_eq!(launcher.root().path(), dir.path());
    }

    #[test]
    fn accounts_live_under_the_launcher_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        let added = launcher
            .accounts()
            .add(crate::auth::offline::offline_account("tester"))
            .expect("add account");
        assert_eq!(
            launcher.accounts().active().expect("active"),
            Some(added.clone())
        );
        assert_eq!(added.name, "tester");
    }

    #[test]
    fn install_loader_of_a_vanilla_instance_installs_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        let instance = launcher
            .instances()
            .create(
                "Pack",
                "1.20.1",
                crate::instances::model::Loader::None,
                None,
                &std::collections::BTreeMap::new(),
            )
            .expect("create");
        // The hosts above are unreachable, so this can only pass without a request.
        assert_eq!(
            launcher.install_loader(&instance.slug).expect("install"),
            "1.20.1"
        );
    }

    #[test]
    fn a_pinned_loader_version_is_used_as_it_stands() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        let picked = launcher
            .resolve_loader_version(Loader::Fabric, "1.20.1", Some("0.16.0"))
            .expect("pinned version needs no request");
        assert_eq!(picked, "0.16.0");
    }

    #[test]
    fn apply_settings_overrides_rewrites_options_txt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        let mut instance = launcher
            .instances()
            .create(
                "Pack",
                "1.20.1",
                crate::instances::model::Loader::None,
                None,
                &std::collections::BTreeMap::new(),
            )
            .expect("create");
        instance
            .config
            .settings_overrides
            .insert("renderDistance".to_string(), "12".to_string());
        assert_eq!(
            launcher.apply_settings_overrides(&instance).expect("apply"),
            1
        );
        let options =
            std::fs::read_to_string(instance.game_dir().join("options.txt")).expect("options.txt");
        assert!(options.contains("renderDistance:12"), "{options}");
        // A second pass changes nothing.
        assert_eq!(
            launcher.apply_settings_overrides(&instance).expect("apply"),
            0
        );
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
