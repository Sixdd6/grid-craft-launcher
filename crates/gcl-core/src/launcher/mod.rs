//! `Launcher`: the handle every binary goes through.
//!
//! It owns the one tokio runtime, the app root, the config, the HTTP client, the event
//! channel, and the cancellation token. Binaries never build a runtime of their own.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard};
use std::time::Duration;

use futures_util::future::BoxFuture;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;

use serde::{Deserialize, Serialize};

use crate::auth::msa::{Msa, MsaEndpoints};
use crate::auth::offline::offline_account;
use crate::auth::secrets::{SecretStore, open_default};
use crate::auth::session::{
    LoginCtx, OnCodeFn, REFRESH_MARGIN, SleepFn, ensure_fresh, login_device_code, refresh_account,
};
use crate::auth::store::Accounts;
use crate::auth::{Account, AccountKind, token_expires_soon};
use crate::config::Config;
use crate::content::{AddOutcome, AddRequest, ContentCtx, ManualDownload, UpdateCandidate};
use crate::download::{DownloadCtx, cleanup_partials};
use crate::events::{Event, EventSink};
use crate::http::HttpClient;
use crate::instances::model::{ContentEntry, ContentKind, InstanceJvm, Loader, PackSource};
use crate::instances::{Instance, Instances, now_rfc3339};
use crate::java::{
    JavaInstall, JavaSource, RUNTIME_MANIFEST, component_for_major, detect_all, install_runtime,
    pick,
};
use crate::launch::{JvmSettings, LaunchCommand, LaunchInputs};
use crate::loaders::{
    LoaderCtx, LoaderEndpoints, LoaderVersion, ProcessRunner, keep_both_libraries,
};
use crate::modpacks::{ImportOutcome, ImportRequest};
use crate::mojang::assets::RESOURCES_BASE;
use crate::mojang::rules::RuleContext;
use crate::mojang::{InstallPlan, Mojang, PISTON_META, VersionManifest, install_version};
use crate::paths::Root;
use crate::sources::curseforge::CurseForge;
use crate::sources::modrinth::Modrinth;
use crate::sources::{BoxSource, SearchPage, SearchQuery, SourceId};

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

/// Test-only override for the Modrinth API base URL, read by [`Endpoints::from_env`].
pub const MODRINTH_BASE_URL_ENV: &str = "GCL_MODRINTH_BASE_URL";

/// Test-only override for the CurseForge API base URL, read by [`Endpoints::from_env`].
pub const CURSEFORGE_BASE_URL_ENV: &str = "GCL_CURSEFORGE_BASE_URL";

/// Test-only override for the Microsoft device-code endpoint, read by [`Endpoints::from_env`].
pub const MSA_DEVICE_URL_ENV: &str = "GCL_MSA_DEVICE_URL";

/// Test-only override for the Microsoft OAuth token endpoint, read by [`Endpoints::from_env`].
pub const MSA_TOKEN_URL_ENV: &str = "GCL_MSA_TOKEN_URL";

/// Test-only override for the Xbox Live authentication endpoint, read by
/// [`Endpoints::from_env`].
pub const MSA_XBL_URL_ENV: &str = "GCL_MSA_XBL_URL";

/// Test-only override for the XSTS authorization endpoint, read by [`Endpoints::from_env`].
pub const MSA_XSTS_URL_ENV: &str = "GCL_MSA_XSTS_URL";

/// Test-only override for the Minecraft services login endpoint, read by
/// [`Endpoints::from_env`].
pub const MSA_MC_URL_ENV: &str = "GCL_MSA_MC_URL";

/// Test-only override for the Minecraft services profile endpoint, read by
/// [`Endpoints::from_env`].
pub const MSA_PROFILE_URL_ENV: &str = "GCL_MSA_PROFILE_URL";

/// File under an instance directory that lists the downloads the user must fetch by hand.
pub const PENDING_MANUAL_FILE: &str = "pending-manual.json";

/// Why [`Launcher::source`] refuses CurseForge when no API key is configured.
const NO_CURSEFORGE_KEY: &str = "no CURSEFORGE_API_KEY";

/// Name of the settings file inside a game directory, as [`crate::settings`] writes it.
const OPTIONS_FILE: &str = "options.txt";

/// Name of the directory holding an instance's worlds, inside its game directory.
const SAVES_DIR: &str = "saves";

/// Every metadata host the launcher talks to.
///
/// [`Launcher::new`] fills it from the test-only environment overrides;
/// [`Launcher::open_with_endpoints`] takes one whole, and reads no environment at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Mojang metadata base URL.
    pub mojang: String,
    /// Modrinth API base URL.
    pub modrinth: String,
    /// CurseForge API base URL.
    pub curseforge: String,
    /// Loader metadata and maven hosts.
    pub loaders: LoaderEndpoints,
    /// The six hosts of the Microsoft login chain.
    pub msa: MsaEndpoints,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            mojang: PISTON_META.to_string(),
            modrinth: crate::sources::modrinth::BASE.to_string(),
            curseforge: crate::sources::curseforge::BASE.to_string(),
            loaders: LoaderEndpoints::default(),
            msa: MsaEndpoints::default(),
        }
    }
}

impl Endpoints {
    /// Reads every test-only base URL override, falling back to the production hosts.
    ///
    /// The overrides are read in a debug build only. A release build returns
    /// [`Endpoints::default`] whatever the environment holds, so no `GCL_*_BASE_URL`
    /// variable can redirect a shipped launcher at another host. Tests and both e2e
    /// scripts run debug builds, so nothing that relies on the overrides changes.
    #[cfg(debug_assertions)]
    pub fn from_env() -> Endpoints {
        Endpoints {
            mojang: env_base(MOJANG_BASE_URL_ENV, PISTON_META),
            modrinth: env_base(MODRINTH_BASE_URL_ENV, crate::sources::modrinth::BASE),
            curseforge: env_base(CURSEFORGE_BASE_URL_ENV, crate::sources::curseforge::BASE),
            loaders: LoaderEndpoints {
                fabric: env_base(FABRIC_BASE_URL_ENV, crate::loaders::fabric::BASE),
                quilt: env_base(QUILT_BASE_URL_ENV, crate::loaders::quilt::BASE),
                forge_meta: env_base(FORGE_META_BASE_URL_ENV, crate::loaders::forge::META),
                forge_maven: env_base(FORGE_MAVEN_BASE_URL_ENV, crate::loaders::forge::MAVEN),
                neoforge: env_base(NEOFORGE_BASE_URL_ENV, crate::loaders::neoforge::MAVEN),
            },
            msa: MsaEndpoints {
                device_code: env_base(MSA_DEVICE_URL_ENV, crate::auth::msa::DEVICE_CODE_URL),
                token: env_base(MSA_TOKEN_URL_ENV, crate::auth::msa::TOKEN_URL),
                xbl: env_base(MSA_XBL_URL_ENV, crate::auth::msa::XBL_URL),
                xsts: env_base(MSA_XSTS_URL_ENV, crate::auth::msa::XSTS_URL),
                mc_login: env_base(MSA_MC_URL_ENV, crate::auth::msa::MC_LOGIN_URL),
                profile: env_base(MSA_PROFILE_URL_ENV, crate::auth::msa::PROFILE_URL),
            },
        }
    }

    /// The production hosts. A release build reads no environment override at all.
    #[cfg(not(debug_assertions))]
    pub fn from_env() -> Endpoints {
        Endpoints::default()
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
        /// One-line reason for a non-zero exit, read out of the log. `None` on a clean exit.
        hint: Option<String>,
    },
}

/// Read access to the launcher's config, for as long as the guard lives.
///
/// It derefs to [`Config`], so `launcher.config().parallel_downloads` reads as it always did.
///
/// It holds the config's read lock. Read what you need and let it drop at the end of the
/// statement: never hold one across another [`Launcher`] call. Most of them read the config
/// themselves, and [`Launcher::update_config`] waits for every reader, so a guard held across
/// one on the same thread deadlocks.
#[derive(Debug)]
pub struct ConfigRead<'a>(RwLockReadGuard<'a, Config>);

impl std::ops::Deref for ConfigRead<'_> {
    type Target = Config;

    fn deref(&self) -> &Config {
        &self.0
    }
}

/// A launch that is ready to start: the command line, its instance, and its log file.
struct PreparedLaunch {
    /// The command line [`crate::launch::build`] produced.
    cmd: LaunchCommand,
    /// The instance being launched, saved again when the game exits.
    instance: Instance,
    /// The file both of the game's output streams are written to.
    log_path: PathBuf,
}

/// A game [`Launcher::launch_instance_async`] started, and the task waiting for it.
#[derive(Debug)]
pub struct RunningLaunch {
    /// Process id of the game, or `None` if it already exited.
    pub pid: Option<u32>,
    /// File both of the game's output streams are written to.
    pub log_path: PathBuf,
    /// Slug of the instance that was launched.
    pub slug: String,
    /// Finishes with the [`LaunchOutcome::Exited`] the game produced.
    pub wait: tokio::task::JoinHandle<Result<LaunchOutcome, crate::Error>>,
}

impl RunningLaunch {
    /// Waits for the game to exit on `launcher`'s runtime. Blocks.
    ///
    /// The launcher must be the one that started this launch: its runtime owns the waiting
    /// task. A task that panicked is reported as an I/O error, since there is nothing better
    /// to say about it.
    pub fn wait_blocking(self, launcher: &Launcher) -> Result<LaunchOutcome, crate::Error> {
        launcher
            .block_on(self.wait)
            .map_err(|err| std::io::Error::other(err.to_string()))?
    }
}

/// Everything one instance's detail view shows, as [`Launcher::instance_summary`] read it.
#[derive(Debug, Clone)]
pub struct InstanceSummary {
    /// The instance as `instance.toml` holds it.
    pub instance: Instance,
    /// Version id a launch resolves, when its version JSON is already cached.
    pub installed_version_id: Option<String>,
    /// Downloads the user still has to fetch by hand.
    pub pending_manual: Vec<ManualDownload>,
    /// The `java` a launch would run: the instance's own, else the configured one.
    pub java: Option<PathBuf>,
}

/// Owns the runtime and every shared handle the rest of the launcher needs.
pub struct Launcher {
    runtime: tokio::runtime::Runtime,
    root: Root,
    /// The loaded config. Read through [`Launcher::config`], changed through
    /// [`Launcher::update_config`], which is the only thing that writes it back to disk.
    config: RwLock<Config>,
    http: HttpClient,
    events: EventSink,
    cancel: CancellationToken,
    endpoints: Endpoints,
    /// The content sources, built on first use and cleared by [`Launcher::update_config`].
    sources: RwLock<Option<Vec<BoxSource>>>,
    process_runner: Option<Arc<dyn ProcessRunner>>,
    /// The refresh-token store, opened on first use by [`Launcher::secrets`].
    secrets: OnceLock<Box<dyn SecretStore>>,
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
            // the one `update_config` writes, so it wins over the config that pointed here.
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
            config: RwLock::new(config),
            http,
            sources: RwLock::new(None),
            events,
            cancel: CancellationToken::new(),
            endpoints,
            process_runner: None,
            secrets: OnceLock::new(),
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

    /// Test seam: runs installer processors through `runner` instead of a real JVM.
    ///
    /// Chain it onto [`Launcher::open_with_endpoints`]. Without it, Forge and NeoForge
    /// installs spawn `java` through [`crate::loaders::JavaRunner`].
    #[must_use]
    pub fn with_process_runner(mut self, runner: Arc<dyn ProcessRunner>) -> Self {
        self.process_runner = Some(runner);
        self
    }

    /// Test seam: keeps refresh tokens in `store` instead of the keyring or a file.
    ///
    /// Chain it onto [`Launcher::open_with_endpoints`] before anything asks for
    /// [`Launcher::secrets`]. A store set after the first use is ignored, because the cell is
    /// already filled.
    #[must_use]
    pub fn with_secret_store(self, store: Box<dyn SecretStore>) -> Self {
        let _ = self.secrets.set(store);
        self
    }

    /// The resolved app root.
    pub fn root(&self) -> &Root {
        &self.root
    }

    /// Read access to the loaded config.
    ///
    /// The returned guard holds the config's read lock and derefs to [`Config`]. Hold it no
    /// longer than the read needs, and never across another [`Launcher`] call: see
    /// [`ConfigRead`].
    pub fn config(&self) -> ConfigRead<'_> {
        ConfigRead(self.read_config())
    }

    /// Changes the config, writes it to `config.toml`, and clears the source cache.
    ///
    /// `f` runs under the write lock, so a whole edit lands at once. The file is saved before
    /// the lock is released, so a reader never sees a change that is not on disk. The cache
    /// behind [`Launcher::sources`] is cleared either way, because an edit may have added or
    /// removed the CurseForge API key: the edit stands in memory even when the save fails, so
    /// a kept cache would disagree with the config the rest of the launcher reads.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<(), crate::Error> {
        let saved = {
            let mut config = self.config.write().unwrap_or_else(|err| err.into_inner());
            f(&mut config);
            config.save(&self.root.config_file())
        };
        self.clear_sources();
        saved?;
        Ok(())
    }

    /// The config's read guard, taking a poisoned lock's value rather than panicking.
    ///
    /// A panic while the config was locked leaves the value itself intact: it is a plain
    /// struct, and every write finishes before the guard drops.
    fn read_config(&self) -> RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(|err| err.into_inner())
    }

    /// Drops the cached source list, so the next [`Launcher::sources`] builds it again.
    fn clear_sources(&self) {
        *self.sources.write().unwrap_or_else(|err| err.into_inner()) = None;
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

    /// The refresh-token store, opened on first use.
    ///
    /// The first call opens the OS keyring, or falls back to a restricted file in the app root
    /// and warns once on the event sink; every later call returns the same store.
    /// [`Launcher::with_secret_store`] fills the cell instead, for tests.
    pub fn secrets(&self) -> &dyn SecretStore {
        self.secrets
            .get_or_init(|| open_default(&self.root, &self.events))
            .as_ref()
    }

    /// The six Microsoft login hosts this launcher talks to.
    pub fn msa_endpoints(&self) -> MsaEndpoints {
        self.endpoints.msa.clone()
    }

    /// True when a Microsoft client id is configured, so signing in is possible.
    pub fn msa_available(&self) -> bool {
        self.read_config().msa_client_id().is_some()
    }

    /// Signs in with the device-code flow and saves the account. Blocks.
    ///
    /// `on_code` is called once with the code and link to show the user, then the token
    /// endpoint is polled until the sign-in is approved, waiting the interval it asks for.
    /// Without a configured client id this is [`crate::auth::Error::Disabled`] and no request
    /// is made.
    ///
    /// The poll loop watches this launcher's own cancel token ([`Launcher::cancel_token`]),
    /// the one downloads use. Cancelling it while a sign-in is waiting for the user aborts
    /// that sign-in with [`crate::auth::Error::Cancelled`] rather than polling until the
    /// code expires.
    #[tracing::instrument(skip_all)]
    pub fn msa_login(&self, on_code: &OnCodeFn) -> Result<Account, crate::Error> {
        self.msa_login_with_cancel(on_code, self.cancel_token().child_token())
    }

    /// Signs in with the device-code flow, watching `cancel` instead of the shared token.
    ///
    /// The same sign-in [`Launcher::msa_login`] runs, with the caller deciding what cancels
    /// it. A GUI passes a child of [`Launcher::cancel_token`] and keeps it, so a "cancel
    /// sign-in" button ends this one login without cancelling every download in flight;
    /// cancelling the parent still cancels this too. Blocks.
    #[tracing::instrument(skip_all)]
    pub fn msa_login_with_cancel(
        &self,
        on_code: &OnCodeFn,
        cancel: CancellationToken,
    ) -> Result<Account, crate::Error> {
        let msa = self.msa_client()?;
        let accounts = self.accounts();
        let ctx = LoginCtx {
            msa: &msa,
            secrets: self.secrets(),
            accounts: &accounts,
            sink: &self.events,
            cancel: &cancel,
        };
        let sleep: &SleepFn = &real_sleep;
        Ok(self.block_on(async move { login_device_code(&ctx, on_code, sleep).await })?)
    }

    /// Signs a saved Microsoft account in again from its stored refresh token. Blocks.
    ///
    /// `id_or_name` names the account the same way `--account` does. An unknown one is
    /// [`crate::auth::Error::NotFound`]; an offline account is
    /// [`crate::auth::Error::NotMicrosoft`], since it has nothing to refresh whatever the
    /// configuration says; no configured client id is [`crate::auth::Error::Disabled`].
    #[tracing::instrument(skip(self))]
    pub fn msa_refresh(&self, id_or_name: &str) -> Result<Account, crate::Error> {
        let accounts = self.accounts();
        let account = accounts
            .find(id_or_name)?
            .ok_or_else(|| crate::auth::Error::NotFound(id_or_name.to_string()))?;
        if account.kind != AccountKind::Msa {
            return Err(crate::auth::Error::NotMicrosoft(account.name).into());
        }
        let msa = self.msa_client()?;
        let cancel = self.cancel_token();
        let ctx = LoginCtx {
            msa: &msa,
            secrets: self.secrets(),
            accounts: &accounts,
            sink: &self.events,
            cancel: &cancel,
        };
        Ok(self.block_on(async move { refresh_account(&ctx, &account).await })?)
    }

    /// A login client over this launcher's client id and endpoints.
    ///
    /// [`crate::auth::Error::Disabled`] when no client id is configured, so every caller
    /// reports the same thing when Microsoft login is turned off.
    fn msa_client(&self) -> Result<Msa, crate::Error> {
        let client_id = self
            .read_config()
            .msa_client_id()
            .ok_or(crate::auth::Error::Disabled)?;
        Ok(Msa::new(
            self.http.clone(),
            self.endpoints.msa.clone(),
            client_id,
        ))
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
            parallel: self.read_config().parallel_downloads,
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

    /// Sets one `options.txt` override on an instance and saves `instance.toml`.
    ///
    /// The key and the value are checked first, by [`crate::settings::validate_key`] and
    /// [`crate::settings::validate_value`], so a pair that could not be written back as one
    /// `key:value` line is rejected before anything is saved. The override reaches
    /// `options.txt` on the next launch, through [`Launcher::apply_settings_overrides`].
    pub fn set_instance_override(
        &self,
        slug: &str,
        key: &str,
        value: &str,
    ) -> Result<(), crate::Error> {
        crate::settings::validate_key(key)?;
        crate::settings::validate_value(value)?;
        let mut instance = self.instances().get(slug)?;
        instance
            .config
            .settings_overrides
            .insert(key.to_string(), value.to_string());
        Ok(instance.save()?)
    }

    /// Drops one `options.txt` override from an instance and saves `instance.toml`.
    ///
    /// Returns whether the key was set. Dropping an override does not restore the line
    /// `options.txt` held before: it only stops the launcher rewriting that key.
    pub fn unset_instance_override(&self, slug: &str, key: &str) -> Result<bool, crate::Error> {
        let mut instance = self.instances().get(slug)?;
        let removed = instance.config.settings_overrides.remove(key).is_some();
        if removed {
            instance.save()?;
        }
        Ok(removed)
    }

    /// Replaces an instance's JVM overrides and saves `instance.toml`.
    ///
    /// A minimum heap larger than the maximum is rejected, because the JVM would refuse to
    /// start. Either bound may be `None`, which falls back to `config.toml` at launch time.
    pub fn set_instance_jvm(&self, slug: &str, jvm: InstanceJvm) -> Result<(), crate::Error> {
        if let (Some(min), Some(max)) = (jvm.min_mib, jvm.max_mib)
            && min > max
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("minimum heap {min} MiB is above the maximum {max} MiB"),
            )
            .into());
        }
        let mut instance = self.instances().get(slug)?;
        instance.config.jvm = jvm;
        Ok(instance.save()?)
    }

    /// Every `key:value` pair in this instance's `options.txt`, in file order.
    ///
    /// This is what the game last wrote, not what the overrides ask for. An instance whose
    /// game has never run has no `options.txt`, which reads as no pairs.
    pub fn instance_options(&self, slug: &str) -> Result<Vec<(String, String)>, crate::Error> {
        let instance = self.instances().get(slug)?;
        let file = crate::settings::read(&instance.game_dir().join(OPTIONS_FILE))?;
        Ok(file
            .pairs()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect())
    }

    /// The world folders under this instance's `saves/`, in name order.
    ///
    /// An instance with no `saves/` directory reads as no worlds. Entries that are not
    /// directories, and names that are not valid UTF-8, are skipped.
    pub fn list_worlds(&self, slug: &str) -> Result<Vec<String>, crate::Error> {
        let instance = self.instances().get(slug)?;
        let saves = instance.game_dir().join(SAVES_DIR);
        let entries = match std::fs::read_dir(&saves) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                names.push(name.to_string());
            }
        }
        names.sort();
        Ok(names)
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
            Loader::Forge | Loader::NeoForge => Some(self.installer_java(&instance, &mc)?),
            _ => None,
        };
        let endpoints = self.loader_endpoints();
        let mojang = self.mojang();
        let dl = self.download_ctx();
        let ctx = self.loader_ctx(&dl, java.as_ref(), self.process_runner.as_deref());
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
    /// account, `account` picks a saved one by id or name without making it active, and
    /// otherwise the active account is used. With none of the three this is
    /// [`crate::auth::Error::NoAccount`].
    ///
    /// A Microsoft account's Minecraft token is refreshed first when it is about to expire.
    /// A refresh failure is returned as [`crate::Error::Auth`]; the caller decides what to
    /// tell the user. Blocks until the game exits.
    #[tracing::instrument(skip(self))]
    pub fn launch_instance(
        &self,
        slug: &str,
        account: Option<&str>,
        offline_user: Option<&str>,
        dry_run: bool,
    ) -> Result<LaunchOutcome, crate::Error> {
        let prepared = self.prepare_launch(slug, account, offline_user)?;
        if dry_run {
            return Ok(LaunchOutcome::DryRun(prepared.cmd));
        }
        self.start(prepared)?.wait_blocking(self)
    }

    /// Starts the game and returns at once, with a handle to what is running.
    ///
    /// Everything [`Launcher::launch_instance`] does up to the spawn is done here, on the
    /// calling thread: the account is resolved and refreshed, the instance is installed, and
    /// the command line is built. Only the waiting is left, and it runs as a task on this
    /// launcher's runtime, so a GUI can show the game as running and keep working. There is
    /// no dry run: [`Launcher::launch_instance`] has that.
    #[tracing::instrument(skip(self))]
    pub fn launch_instance_async(
        &self,
        slug: &str,
        account: Option<&str>,
        offline_user: Option<&str>,
    ) -> Result<RunningLaunch, crate::Error> {
        let prepared = self.prepare_launch(slug, account, offline_user)?;
        self.start(prepared)
    }

    /// Resolves the account, installs the instance, and builds the command line.
    ///
    /// The shared body of the two launches: everything before the process is started.
    fn prepare_launch(
        &self,
        slug: &str,
        account: Option<&str>,
        offline_user: Option<&str>,
    ) -> Result<PreparedLaunch, crate::Error> {
        let accounts = self.accounts();
        let account = match (offline_user, account) {
            (Some(name), _) => accounts.add(offline_account(name))?,
            // A `--account` launch reads the store only: it never moves `active`.
            (None, Some(id_or_name)) => accounts
                .find(id_or_name)?
                .ok_or_else(|| crate::auth::Error::NotFound(id_or_name.to_string()))?,
            (None, None) => accounts.active()?.ok_or(crate::auth::Error::NoAccount)?,
        };
        let client_id = self.read_config().msa_client_id();
        let account = self.fresh_for_launch(account, client_id.as_deref())?;

        let plan = self.install_instance(slug)?;
        let instance = self.instances().get(slug)?;
        let java = match instance
            .config
            .jvm
            .java_path
            .clone()
            .or_else(|| self.read_config().jvm.java_path.clone())
        {
            Some(path) => path,
            None => self.ensure_java_for(&plan)?.path,
        };
        let changed = self.apply_settings_overrides(&instance)?;
        if changed > 0 {
            tracing::info!(changed, "rewrote options.txt keys");
        }

        let identity = account.launch_identity_with(client_id.as_deref().unwrap_or_default());
        let rules = RuleContext::current();
        let jvm = {
            let config = self.read_config();
            JvmSettings {
                min_mib: instance.config.jvm.min_mib.unwrap_or(config.jvm.min_mib),
                max_mib: instance.config.jvm.max_mib.unwrap_or(config.jvm.max_mib),
                extra_args: instance.config.jvm.extra_args.clone(),
            }
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
        let log_path = self
            .root
            .logs_dir()
            .join(format!("{slug}-{}.log", now_rfc3339().replace(':', "-")));
        Ok(PreparedLaunch {
            cmd,
            instance,
            log_path,
        })
    }

    /// Spawns the prepared command and the task that waits for it.
    ///
    /// `last_launched` is written as soon as the process starts, so a caller that lists
    /// instances while the game runs already sees the launch. The task writes it again when
    /// the game exits, and reads a crash hint out of the log when the exit code is not zero,
    /// so both launches report the same [`LaunchOutcome`].
    fn start(&self, prepared: PreparedLaunch) -> Result<RunningLaunch, crate::Error> {
        let PreparedLaunch {
            cmd,
            mut instance,
            log_path,
        } = prepared;
        let sink = self.events.clone();
        let target = log_path.clone();
        let game = self.block_on(async move { crate::launch::spawn(&cmd, target, sink).await })?;
        let pid = game.child.id();
        let slug = instance.slug.clone();
        let path = log_path.clone();
        instance.config.last_launched = Some(now_rfc3339());
        instance.save()?;
        let wait = self.runtime.spawn(async move {
            let code = crate::launch::wait(game).await?;
            // Written again on exit, so the timestamp survives an edit made while the game
            // ran and a reader can tell a finished launch from a running one by the log.
            instance.config.last_launched = Some(now_rfc3339());
            instance.save()?;
            let hint = (code != 0).then(|| crate::launch::crash_hint(&path));
            Ok(LaunchOutcome::Exited {
                code,
                log_path: path,
                hint,
            })
        });
        Ok(RunningLaunch {
            pid,
            log_path,
            slug,
            wait,
        })
    }

    /// Everything one instance's detail view needs, in one call.
    ///
    /// `installed_version_id` is the version id a launch would resolve — the loader build, or
    /// the Minecraft version for a vanilla instance — and it is `None` until that version
    /// JSON is in the cache, which is how the caller knows the instance still needs an
    /// install. `java` is the instance's own `java_path`, else the one in `config.toml`, and
    /// `None` when neither is set and a launch would find or install a runtime itself.
    pub fn instance_summary(&self, slug: &str) -> Result<InstanceSummary, crate::Error> {
        let instance = self.instances().get(slug)?;
        // A loader instance with no `loader_version` has never been installed: the build it
        // would resolve is not decided yet, so the id built here matches no cached file and
        // the answer is `None` by design. That is the state the caller has to report anyway.
        let id = crate::loaders::version_id(
            instance.config.loader,
            &instance.config.minecraft,
            instance
                .config
                .loader_version
                .as_deref()
                .unwrap_or_default(),
        );
        let installed_version_id = self
            .root
            .versions_dir()
            .join(format!("{id}.json"))
            .is_file()
            .then_some(id);
        let java = instance
            .config
            .jvm
            .java_path
            .clone()
            .or_else(|| self.read_config().jvm.java_path.clone());
        Ok(InstanceSummary {
            pending_manual: self.pending_manual(slug)?,
            instance,
            installed_version_id,
            java,
        })
    }

    /// Every configured content source, in preference order.
    ///
    /// Modrinth is always present. CurseForge is present only when
    /// [`Config::curseforge_api_key`] finds a key. The list is built on the first call and
    /// kept; [`Launcher::update_config`] clears it, so an edit that adds or removes the key
    /// takes effect on the next call. Two threads that race to build it get the same list,
    /// and the last one written wins.
    pub fn sources(&self) -> Vec<BoxSource> {
        if let Some(cached) = self
            .sources
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
        {
            return cached;
        }
        // The config guard is dropped at the end of this statement, before the write lock is
        // taken: `update_config` locks the config first and the sources second, and taking
        // them in the other order here could deadlock.
        let built = build_sources(&self.http, &self.read_config(), &self.endpoints);
        *self.sources.write().unwrap_or_else(|err| err.into_inner()) = Some(built.clone());
        built
    }

    /// The configured source with this id.
    ///
    /// A CurseForge lookup without an API key is [`crate::sources::Error::Disabled`].
    pub fn source(&self, id: SourceId) -> Result<BoxSource, crate::Error> {
        self.sources()
            .iter()
            .find(|source| source.id() == id)
            .cloned()
            .ok_or_else(|| {
                crate::Error::from(crate::sources::Error::Disabled {
                    source_id: id,
                    reason: NO_CURSEFORGE_KEY.to_string(),
                })
            })
    }

    /// Searches one source for projects matching `q`. Blocks.
    pub fn search(&self, id: SourceId, q: &SearchQuery) -> Result<SearchPage, crate::Error> {
        let source = self.source(id)?;
        Ok(self.block_on(async { source.search(q).await })?)
    }

    /// Installs a project into an instance, following its required dependencies. Blocks.
    ///
    /// The instance is saved by the placement itself. A file the author opted out of
    /// third-party distribution is appended to `pending-manual.json` under the instance
    /// directory, and reported in [`AddOutcome::manual`]; it never fails the call.
    #[tracing::instrument(skip(self))]
    pub fn add_content(&self, slug: &str, req: AddRequest) -> Result<AddOutcome, crate::Error> {
        let mut instance = self.instances().get(slug)?;
        let dl = self.download_ctx();
        let sources = self.sources();
        let ctx = self.content_ctx(&dl, &sources);
        let outcome = self.block_on(crate::content::add(&ctx, &mut instance, req))?;
        append_pending(&self.pending_path(slug), &outcome.manual)?;
        Ok(outcome)
    }

    /// The content `instance.toml` records, in install order.
    pub fn list_content(&self, slug: &str) -> Result<Vec<ContentEntry>, crate::Error> {
        Ok(self.instances().get(slug)?.config.content)
    }

    /// Deletes one installed project from disk and from `instance.toml`.
    pub fn remove_content(&self, slug: &str, project_id: &str) -> Result<(), crate::Error> {
        let mut instance = self.instances().get(slug)?;
        Ok(crate::instances::content::remove(
            &mut instance,
            project_id,
        )?)
    }

    /// Enables or disables one installed project. Returns the path its file now has.
    pub fn set_content_enabled(
        &self,
        slug: &str,
        project_id: &str,
        enabled: bool,
    ) -> Result<PathBuf, crate::Error> {
        let mut instance = self.instances().get(slug)?;
        Ok(crate::instances::content::set_enabled(
            &mut instance,
            project_id,
            enabled,
        )?)
    }

    /// Lists the installed content that has a newer compatible version at its source. Blocks.
    #[tracing::instrument(skip(self))]
    pub fn check_updates(&self, slug: &str) -> Result<Vec<UpdateCandidate>, crate::Error> {
        let instance = self.instances().get(slug)?;
        let dl = self.download_ctx();
        let sources = self.sources();
        let ctx = self.content_ctx(&dl, &sources);
        Ok(self.block_on(crate::content::check_updates(&ctx, &instance))?)
    }

    /// Installs every candidate [`Launcher::check_updates`] returned. Blocks.
    ///
    /// One [`AddOutcome`] comes back per candidate, in the order they were given. Manual
    /// downloads are appended to `pending-manual.json`, exactly as [`Launcher::add_content`]
    /// appends them.
    #[tracing::instrument(skip(self, candidates))]
    pub fn apply_updates(
        &self,
        slug: &str,
        candidates: &[UpdateCandidate],
    ) -> Result<Vec<AddOutcome>, crate::Error> {
        let mut instance = self.instances().get(slug)?;
        let dl = self.download_ctx();
        let sources = self.sources();
        let ctx = self.content_ctx(&dl, &sources);
        let mut outcomes = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let outcome =
                self.block_on(crate::content::apply_update(&ctx, &mut instance, candidate))?;
            append_pending(&self.pending_path(slug), &outcome.manual)?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    /// The downloads this instance still needs the user to fetch by hand.
    ///
    /// An instance that never hit one has no `pending-manual.json`, which reads as empty.
    pub fn pending_manual(&self, slug: &str) -> Result<Vec<ManualDownload>, crate::Error> {
        read_pending(&self.pending_path(slug))
    }

    /// Verifies a hand-downloaded file, installs it, and drops it from the pending list.
    ///
    /// Blocks.
    #[tracing::instrument(skip(self, pending))]
    pub fn import_manual_file(
        &self,
        slug: &str,
        pending: &ManualDownload,
        file: &Path,
        kind: ContentKind,
    ) -> Result<ContentEntry, crate::Error> {
        let mut instance = self.instances().get(slug)?;
        let dl = self.download_ctx();
        let sources = self.sources();
        let ctx = self.content_ctx(&dl, &sources);
        let entry = self.block_on(crate::content::import_manual(
            &ctx,
            &mut instance,
            pending,
            file,
            kind,
        ))?;
        remove_pending(&self.pending_path(slug), pending)?;
        Ok(entry)
    }

    /// Imports a modpack archive on disk as a new instance. Blocks.
    pub fn import_modpack_file(
        &self,
        zip: &Path,
        name: Option<String>,
    ) -> Result<ImportOutcome, crate::Error> {
        self.import_pack(zip, name, None)
    }

    /// Downloads a modpack from a source and imports it as a new instance. Blocks.
    ///
    /// `project` is an id or a slug; `version` pins one by id or number, and without one
    /// the newest release wins.
    #[tracing::instrument(skip(self))]
    pub fn import_modpack(
        &self,
        source: SourceId,
        project: &str,
        version: Option<&str>,
        name: Option<String>,
    ) -> Result<ImportOutcome, crate::Error> {
        let (zip, pack_source) = {
            let dl = self.download_ctx();
            let sources = self.sources();
            let ctx = self.content_ctx(&dl, &sources);
            self.block_on(crate::modpacks::fetch_pack(&ctx, source, project, version))?
        };
        self.import_pack(&zip, name, Some(pack_source))
    }

    /// Shared body of the two modpack imports.
    ///
    /// The pack's manifest is read here, not inside [`crate::modpacks::import`], because a
    /// Forge or NeoForge pack needs a JVM for its installer processors and only the manifest
    /// says which Minecraft version that JVM has to match. The plan is then handed to
    /// [`crate::modpacks::import_plan`], so the zip is parsed once.
    ///
    /// A file the pack's author opted out of third-party distribution is appended to
    /// `pending-manual.json` under the new instance, the same way [`Launcher::add_content`]
    /// appends one.
    fn import_pack(
        &self,
        zip: &Path,
        name: Option<String>,
        pack_source: Option<PackSource>,
    ) -> Result<ImportOutcome, crate::Error> {
        let req = ImportRequest {
            zip: zip.to_path_buf(),
            name,
            keep_partial: false,
            pack_source,
            // The launcher only ever fetches a pack's files from the hosts the mrpack
            // specification names. `extra_hosts` exists for tests that serve them locally.
            extra_hosts: Vec::new(),
        };
        let target = req.zip.clone();
        let hosts = req.extra_hosts.clone();
        let (format, plan) = self
            .block_on(async move {
                tokio::task::spawn_blocking(move || crate::modpacks::read_plan(&target, &hosts))
                    .await
            })
            // A `JoinError` here means the parse panicked; there is nothing better to say.
            .map_err(|err| std::io::Error::other(err.to_string()))??;
        let java = match plan.loader {
            Loader::Forge | Loader::NeoForge => {
                // An imported pack has no instance yet, so only `config.toml` can name a JVM.
                Some(self.configured_or_detected_java(None, &plan.minecraft)?)
            }
            _ => None,
        };

        let endpoints = self.loader_endpoints();
        let mojang = self.mojang();
        let instances = self.instances();
        let dl = self.download_ctx();
        let sources = self.sources();
        let ctx = self.content_ctx(&dl, &sources);
        let loader_ctx = LoaderCtx {
            mojang: Some(&mojang),
            ..self.loader_ctx(&dl, java.as_ref(), self.process_runner.as_deref())
        };
        // Copied out, so the config's read lock is not held for the length of the import.
        let game_defaults = self.read_config().game_defaults.clone();
        let outcome = self.block_on(crate::modpacks::import_plan(
            &ctx,
            &instances,
            &loader_ctx,
            &endpoints,
            &game_defaults,
            format,
            plan,
            req,
        ))?;
        append_pending(&self.pending_path(&outcome.instance.slug), &outcome.manual)?;
        Ok(outcome)
    }

    /// Returns the account a launch should use, refreshing a stale Microsoft token first.
    ///
    /// An offline account is returned unchanged. A Microsoft account is refreshed when its
    /// token is about to expire and a client id is configured. Without a client id nothing can
    /// be refreshed: a token with time left is used as it stands, so a launch still works
    /// while the sign-in lasts, and a stale one is [`crate::auth::Error::Disabled`].
    fn fresh_for_launch(
        &self,
        account: Account,
        client_id: Option<&str>,
    ) -> Result<Account, crate::Error> {
        if account.kind != AccountKind::Msa {
            return Ok(account);
        }
        let now = OffsetDateTime::now_utc();
        if client_id.is_none() {
            if token_expires_soon(account.mc_token_expires.as_deref(), now, REFRESH_MARGIN) {
                return Err(crate::auth::Error::Disabled.into());
            }
            return Ok(account);
        }
        let msa = self.msa_client()?;
        let accounts = self.accounts();
        let cancel = self.cancel_token();
        let ctx = LoginCtx {
            msa: &msa,
            secrets: self.secrets(),
            accounts: &accounts,
            sink: &self.events,
            cancel: &cancel,
        };
        Ok(self.block_on(async move { ensure_fresh(&ctx, account, now).await })?)
    }

    /// A content context over `sources`, this root, this event sink, and `dl`.
    ///
    /// [`ContentCtx`] borrows the [`DownloadCtx`] and the source list, both of which this
    /// launcher hands out by value, so the caller keeps them alive and passes them in.
    /// [`Launcher::loader_ctx`] does the same with the download context.
    fn content_ctx<'a>(
        &'a self,
        dl: &'a DownloadCtx<'a>,
        sources: &'a [BoxSource],
    ) -> ContentCtx<'a> {
        ContentCtx {
            sources,
            dl,
            root: &self.root,
            sink: &self.events,
        }
    }

    /// Path of one instance's pending hand-download list.
    fn pending_path(&self, slug: &str) -> PathBuf {
        self.root.instance_dir(slug).join(PENDING_MANUAL_FILE)
    }

    /// The JVM this instance's Forge or NeoForge installer runs its processors under.
    fn installer_java(&self, instance: &Instance, mc: &str) -> Result<JavaInstall, crate::Error> {
        self.configured_or_detected_java(instance.config.jvm.java_path.as_deref(), mc)
    }

    /// The JVM an installer runs under: the configured one, or one found or installed.
    ///
    /// `instance_java` is the instance's own `java_path`, and it wins; `config.toml`'s
    /// `java_path` is next. Either is taken as given, the same way a launch takes it, so an
    /// install never probes or downloads a runtime the user has already pointed at. Only the
    /// path is read from a configured JVM; the version fields are placeholders. With neither
    /// set, a runtime for the Minecraft version is found or installed, which reaches the
    /// network. A modpack import passes `None`, because the instance does not exist yet.
    fn configured_or_detected_java(
        &self,
        instance_java: Option<&Path>,
        mc: &str,
    ) -> Result<JavaInstall, crate::Error> {
        match instance_java
            .map(Path::to_path_buf)
            .or_else(|| self.read_config().jvm.java_path.clone())
        {
            Some(path) => Ok(JavaInstall {
                path,
                major: 0,
                version: "configured".to_string(),
                vendor: "configured".to_string(),
                source: JavaSource::Manual,
            }),
            None => self.java_for_version(mc),
        }
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
            .field("config", &*self.read_config())
            .finish_non_exhaustive()
    }
}

/// The wait between device-code polls: real time on the launcher's own runtime.
///
/// [`crate::auth::session::login_device_code`] takes the wait as a function, so a test drives
/// the flow with no real sleeping; this is the launcher's production one.
fn real_sleep(wait: Duration) -> BoxFuture<'static, ()> {
    Box::pin(tokio::time::sleep(wait))
}

/// Builds the source list: Modrinth always, CurseForge only with an API key.
fn build_sources(http: &HttpClient, config: &Config, endpoints: &Endpoints) -> Vec<BoxSource> {
    let mut sources: Vec<BoxSource> = vec![Arc::new(Modrinth::with_base_url(
        http.clone(),
        endpoints.modrinth.clone(),
    ))];
    match config.curseforge_api_key() {
        Some(key) => sources.push(Arc::new(CurseForge::with_base_url(
            http.clone(),
            key,
            endpoints.curseforge.clone(),
        ))),
        None => tracing::debug!("no CurseForge API key, that source stays off"),
    }
    sources
}

/// One [`ManualDownload`] as `pending-manual.json` stores it.
///
/// [`ManualDownload`] is not serialisable itself, because nothing else persists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingRecord {
    source: SourceId,
    project_id: String,
    version_id: String,
    file_name: String,
    page_url: String,
    #[serde(default)]
    fingerprint: Option<u32>,
    #[serde(default)]
    sha1: Option<String>,
    /// World a data pack installs into. Absent in a file written before this field existed.
    #[serde(default)]
    world: Option<String>,
}

impl From<&ManualDownload> for PendingRecord {
    fn from(pending: &ManualDownload) -> Self {
        PendingRecord {
            source: pending.source,
            project_id: pending.project_id.clone(),
            version_id: pending.version_id.clone(),
            file_name: pending.file_name.clone(),
            page_url: pending.page_url.clone(),
            fingerprint: pending.fingerprint,
            sha1: pending.sha1.clone(),
            world: pending.world.clone(),
        }
    }
}

impl From<PendingRecord> for ManualDownload {
    fn from(record: PendingRecord) -> Self {
        ManualDownload {
            source: record.source,
            project_id: record.project_id,
            version_id: record.version_id,
            file_name: record.file_name,
            page_url: record.page_url,
            fingerprint: record.fingerprint,
            sha1: record.sha1,
            world: record.world,
        }
    }
}

/// What makes two pending downloads the same entry.
fn pending_key(pending: &ManualDownload) -> (SourceId, &str, &str) {
    (
        pending.source,
        pending.project_id.as_str(),
        pending.version_id.as_str(),
    )
}

/// Reads the pending hand-download list at `path`. A missing file reads as empty.
pub(crate) fn read_pending(path: &Path) -> Result<Vec<ManualDownload>, crate::Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let records: Vec<PendingRecord> = serde_json::from_str(&text)
        .map_err(|err| std::io::Error::other(format!("{}: invalid JSON: {err}", path.display())))?;
    Ok(records.into_iter().map(ManualDownload::from).collect())
}

/// Appends `items` to the pending list at `path`, skipping ones already listed.
///
/// Two entries are the same when their source, project id, and version id match. Writing
/// nothing is not an error: an empty `items` leaves the file, and its absence, alone.
pub(crate) fn append_pending(path: &Path, items: &[ManualDownload]) -> Result<(), crate::Error> {
    if items.is_empty() {
        return Ok(());
    }
    let mut listed = read_pending(path)?;
    for item in items {
        if listed
            .iter()
            .any(|kept| pending_key(kept) == pending_key(item))
        {
            continue;
        }
        listed.push(item.clone());
    }
    write_pending(path, &listed)
}

/// Drops `item` from the pending list at `path`. An entry that is not there is not an error.
pub(crate) fn remove_pending(path: &Path, item: &ManualDownload) -> Result<(), crate::Error> {
    let mut listed = read_pending(path)?;
    let before = listed.len();
    listed.retain(|kept| pending_key(kept) != pending_key(item));
    if listed.len() == before {
        return Ok(());
    }
    write_pending(path, &listed)
}

/// Writes the whole pending list at `path`, atomically.
fn write_pending(path: &Path, items: &[ManualDownload]) -> Result<(), crate::Error> {
    let records: Vec<PendingRecord> = items.iter().map(PendingRecord::from).collect();
    let text = serde_json::to_string_pretty(&records)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    crate::paths::write_atomic(path, text.as_bytes())?;
    Ok(())
}

/// Reads a test-only base URL override, falling back to the production endpoint.
///
/// Debug builds only: [`Endpoints::from_env`] is the sole caller, and a release build
/// reads no override.
#[cfg(debug_assertions)]
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

    /// Clears the CurseForge API key from the environment, for the test's lifetime.
    ///
    /// `just` loads a `.env` before it runs the tests, so a key on this machine would
    /// otherwise decide what [`build_sources`] builds.
    fn clean_key() -> std::sync::MutexGuard<'static, ()> {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: guarded by ENV_LOCK; no other thread reads the env while it is held.
        unsafe {
            std::env::remove_var("CURSEFORGE_API_KEY");
        }
        guard
    }

    /// Compiles only for a type that can be shared across threads.
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn launcher_is_send_and_sync() {
        // The GUI hands one `Arc<Launcher>` to every worker thread, so this has to hold.
        assert_send_sync::<Launcher>();
    }

    #[test]
    fn update_config_persists_and_rebuilds_the_sources() {
        let _guard = clean_key();
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        assert_eq!(launcher.sources().len(), 1, "no key, so Modrinth only");

        launcher
            .update_config(|config| {
                config.keys.curseforge_api_key = Some("test-key".to_string());
                config.parallel_downloads = 3;
            })
            .expect("update config");

        let ids: Vec<SourceId> = launcher.sources().iter().map(|s| s.id()).collect();
        assert_eq!(
            ids,
            vec![SourceId::Modrinth, SourceId::CurseForge],
            "the cache was cleared, so the new key built a second source"
        );
        assert!(launcher.source(SourceId::CurseForge).is_ok());

        let (again, _rx) =
            Launcher::open_with_endpoints(dir.path().to_path_buf(), Endpoints::default())
                .expect("second launcher");
        assert_eq!(again.config().parallel_downloads, 3, "the file was written");
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
        assert_eq!(*launcher.config(), crate::config::Config::default());
    }

    #[test]
    fn saved_config_is_read_back_by_a_later_launcher() {
        let _guard = clean_env();
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let (launcher, _rx) =
                Launcher::new(Some(dir.path().to_path_buf())).expect("build launcher");
            launcher
                .update_config(|config| {
                    config.parallel_downloads = 3;
                    config
                        .game_defaults
                        .insert("renderDistance".to_string(), "12".to_string());
                })
                .expect("save config");
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

        let (launcher, _rx) =
            Launcher::open(Root::from_path(outer.path()), false, Endpoints::default())
                .expect("first launcher");
        assert_eq!(launcher.root().path(), target.path());
        launcher
            .update_config(|config| config.jvm.max_mib = 8192)
            .expect("save config");

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
            modrinth: "http://modrinth.invalid".to_string(),
            curseforge: "http://curseforge.invalid".to_string(),
            loaders: LoaderEndpoints {
                fabric: "http://fabric.invalid".to_string(),
                quilt: "http://quilt.invalid".to_string(),
                forge_meta: "http://forge-meta.invalid".to_string(),
                forge_maven: "http://forge-maven.invalid".to_string(),
                neoforge: "http://neoforge.invalid".to_string(),
            },
            msa: MsaEndpoints {
                device_code: "http://msa.invalid/devicecode".to_string(),
                token: "http://msa.invalid/token".to_string(),
                xbl: "http://msa.invalid/xbl".to_string(),
                xsts: "http://msa.invalid/xsts".to_string(),
                mc_login: "http://msa.invalid/mclogin".to_string(),
                profile: "http://msa.invalid/profile".to_string(),
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

    /// A pending hand-download, with `n` in every field that identifies it.
    fn pending(n: u32) -> ManualDownload {
        ManualDownload {
            source: SourceId::CurseForge,
            project_id: format!("project-{n}"),
            version_id: format!("version-{n}"),
            file_name: format!("mod-{n}.jar"),
            page_url: format!("https://example.invalid/{n}"),
            fingerprint: Some(n),
            sha1: None,
            world: None,
        }
    }

    #[test]
    fn a_missing_pending_file_reads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PENDING_MANUAL_FILE);
        assert!(read_pending(&path).expect("read").is_empty());
    }

    #[test]
    fn pending_downloads_round_trip_through_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PENDING_MANUAL_FILE);
        append_pending(&path, &[pending(1), pending(2)]).expect("append");
        assert_eq!(
            read_pending(&path).expect("read"),
            vec![pending(1), pending(2)]
        );

        // The same entry twice is listed once; a new one is added.
        append_pending(&path, &[pending(1), pending(3)]).expect("append again");
        assert_eq!(
            read_pending(&path).expect("read"),
            vec![pending(1), pending(2), pending(3)]
        );

        remove_pending(&path, &pending(2)).expect("remove");
        assert_eq!(
            read_pending(&path).expect("read"),
            vec![pending(1), pending(3)]
        );
        // Removing what is not listed changes nothing.
        remove_pending(&path, &pending(2)).expect("remove again");
        assert_eq!(
            read_pending(&path).expect("read"),
            vec![pending(1), pending(3)]
        );
    }

    #[test]
    fn appending_nothing_creates_no_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PENDING_MANUAL_FILE);
        append_pending(&path, &[]).expect("append nothing");
        assert!(!path.exists());
    }

    #[test]
    fn pending_manual_of_an_instance_starts_empty() {
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
        assert!(
            launcher
                .pending_manual(&instance.slug)
                .expect("pending")
                .is_empty()
        );
    }

    #[test]
    fn instance_summary_reports_the_install_state_and_the_configured_java() {
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

        let summary = launcher.instance_summary(&instance.slug).expect("summary");
        assert_eq!(summary.instance.slug, instance.slug);
        assert_eq!(
            summary.installed_version_id, None,
            "nothing is cached yet, so the instance still needs an install"
        );
        assert!(summary.pending_manual.is_empty());
        assert_eq!(summary.java, None);

        let cached = launcher.root().versions_dir().join("1.20.1.json");
        std::fs::create_dir_all(launcher.root().versions_dir()).expect("mkdir");
        std::fs::write(&cached, b"{}").expect("write version json");
        let java = dir.path().join("java");
        launcher
            .update_config(|config| config.jvm.java_path = Some(java.clone()))
            .expect("update config");

        let summary = launcher.instance_summary(&instance.slug).expect("summary");
        assert_eq!(summary.installed_version_id.as_deref(), Some("1.20.1"));
        assert_eq!(summary.java, Some(java));
    }

    #[test]
    fn sources_always_hold_modrinth() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        assert_eq!(
            launcher.source(SourceId::Modrinth).expect("modrinth").id(),
            SourceId::Modrinth
        );
    }

    #[test]
    fn a_source_that_is_not_configured_is_disabled() {
        let _guard = clean_key();
        let http = HttpClient::new().expect("http");
        let sources = build_sources(&http, &Config::default(), &Endpoints::default());
        assert_eq!(sources.len(), 1, "no key was configured");
        assert_eq!(sources[0].id(), SourceId::Modrinth);
    }

    #[test]
    fn a_configured_key_adds_curseforge() {
        let http = HttpClient::new().expect("http");
        let config = Config {
            keys: crate::config::Keys {
                curseforge_api_key: Some("test-key".to_string()),
                msa_client_id: None,
            },
            ..Config::default()
        };
        let sources = build_sources(&http, &config, &Endpoints::default());
        let ids: Vec<SourceId> = sources.iter().map(|s| s.id()).collect();
        assert_eq!(ids, vec![SourceId::Modrinth, SourceId::CurseForge]);
    }

    #[test]
    fn from_env_reads_the_source_overrides() {
        let _guard = clean_env();
        // SAFETY: guarded by ENV_LOCK; both variables are removed before the assert.
        unsafe {
            std::env::set_var(MODRINTH_BASE_URL_ENV, "http://modrinth-from-env.invalid");
            std::env::set_var(CURSEFORGE_BASE_URL_ENV, "http://cf-from-env.invalid");
        }
        let endpoints = Endpoints::from_env();
        unsafe {
            std::env::remove_var(MODRINTH_BASE_URL_ENV);
            std::env::remove_var(CURSEFORGE_BASE_URL_ENV);
        }
        assert_eq!(endpoints.modrinth, "http://modrinth-from-env.invalid");
        assert_eq!(endpoints.curseforge, "http://cf-from-env.invalid");
    }

    #[test]
    fn from_env_reads_the_microsoft_overrides() {
        let _guard = clean_env();
        // SAFETY: guarded by ENV_LOCK; both variables are removed before the assert.
        unsafe {
            std::env::set_var(MSA_DEVICE_URL_ENV, "http://device-from-env.invalid");
            std::env::set_var(MSA_PROFILE_URL_ENV, "http://profile-from-env.invalid");
        }
        let endpoints = Endpoints::from_env();
        unsafe {
            std::env::remove_var(MSA_DEVICE_URL_ENV);
            std::env::remove_var(MSA_PROFILE_URL_ENV);
        }
        assert_eq!(endpoints.msa.device_code, "http://device-from-env.invalid");
        assert_eq!(endpoints.msa.profile, "http://profile-from-env.invalid");
        // One override replaces one field; the rest stay production.
        assert_eq!(endpoints.msa.token, crate::auth::msa::TOKEN_URL);
    }

    #[test]
    fn an_injected_secret_store_is_the_one_the_launcher_uses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher =
            seamed(&dir).with_secret_store(Box::new(crate::auth::secrets::MemoryStore::new()));
        assert_eq!(
            launcher.secrets().kind(),
            crate::auth::secrets::SecretStoreKind::Memory
        );
        launcher.secrets().put("id", "token").expect("put");
        assert_eq!(
            launcher.secrets().get("id").expect("get"),
            Some("token".to_string()),
            "the store is opened once and kept"
        );
        assert!(
            !dir.path().join("secrets.json").exists(),
            "nothing was written to the app root"
        );
    }

    #[test]
    fn microsoft_login_is_unavailable_without_a_client_id() {
        let _guard = clean_env();
        // SAFETY: guarded by ENV_LOCK; the config decides the answer, not this machine.
        unsafe {
            std::env::remove_var("GCL_MSA_CLIENT_ID");
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        assert!(!launcher.msa_available());
        launcher
            .update_config(|config| config.keys.msa_client_id = Some("client".to_string()))
            .expect("update config");
        assert!(launcher.msa_available());
    }

    #[test]
    fn msa_endpoints_come_from_the_launcher_endpoints() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        assert_eq!(
            launcher.msa_endpoints().token,
            "http://msa.invalid/token".to_string()
        );
    }

    #[test]
    fn a_configured_java_path_is_used_without_probing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        let fake = dir.path().join("java");
        std::fs::write(&fake, b"not really java").expect("write fake java");
        launcher
            .update_config(|config| config.jvm.java_path = Some(fake.clone()))
            .expect("update config");

        // The Mojang host of a `seamed` launcher is unreachable, so this can only return
        // without asking for a version, or a runtime.
        let java = launcher
            .configured_or_detected_java(None, "1.20.1")
            .expect("configured java");
        assert_eq!(java.path, fake);
        assert_eq!(java.source, JavaSource::Manual);

        // An instance's own path wins over the one in `config.toml`.
        let own = dir.path().join("own-java");
        let java = launcher
            .configured_or_detected_java(Some(&own), "1.20.1")
            .expect("instance java");
        assert_eq!(java.path, own);
    }

    #[test]
    fn without_a_configured_java_path_a_runtime_is_looked_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let launcher = seamed(&dir);
        // Nothing is configured, so this falls through to `java_for_version`, which asks the
        // Mojang host. That host does not exist, and the failure is the proof it was asked.
        let err = launcher
            .configured_or_detected_java(None, "1.20.1")
            .expect_err("the metadata host is unreachable");
        assert!(matches!(err, crate::Error::Mojang(_)), "{err:?}");
    }
}
