//! Test harness for the GUI flow tests: a real `AppWindow` over a real `Launcher`, driven
//! through the Slint testing backend.
//!
//! The window, the launcher, and the mock metadata host all live in [`TestApp`]. A flow is an
//! async block handed to [`run`], which starts the Slint event loop and quits it when the
//! flow ends. Every wait in a flow yields to that loop, so the bridge's worker threads post
//! their results back exactly as they do in the running app.
//!
//! Only `tests/flow_instances.rs` compiles this module, so nothing here is dead code there;
//! the allow is kept for a second flow binary that uses only part of it.
#![allow(dead_code)]

use std::cell::RefCell;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use gcl_core::Launcher;
use gcl_core::auth::secrets::MemoryStore;
use gcl_core::download::hash::sha1_hex;
use gcl_core::launcher::Endpoints;
use gcl_core::loaders::LoaderEndpoints;
use gcl_core::mojang::MANIFEST_PATH;
use i_slint_backend_testing::{ElementHandle, ElementRoot};
use slint::ComponentHandle;
use wiremock::matchers::{method, path as path_matcher};
use wiremock::{Mock, MockServer, ResponseTemplate};

use gcl_ui::AppWindow;

/// Minecraft version every flow uses. The Fabric fixtures are for this version.
pub const MC: &str = "1.20.1";

/// Fabric build the fixture loader list marks as stable.
pub const FABRIC: &str = "0.19.5";

/// Most Up presses [`TestApp::select_combo`] will spend reaching row 0 before it gives up.
///
/// It presses until the value stops changing, so this is only a guard against a ComboBox that
/// never settles. No real list is anywhere near it.
const COMBO_UP_CAP: usize = 4096;

/// Offline account the harness creates, so a launch never has to open the name prompt.
pub const PLAYER: &str = "Player";

const FABRIC_LOADERS: &str = include_str!("../../../../tests/fixtures/fabric/loader_1.20.1.json");
const FABRIC_PROFILE: &str = include_str!("../../../../tests/fixtures/fabric/profile_1.20.1.json");

/// Client jar bytes the mock vanilla version serves.
const CLIENT_JAR: &[u8] = b"vanilla client jar";

/// log4j2 configuration bytes the mock vanilla version serves.
const LOG_CONFIG: &[u8] = b"<Configuration></Configuration>";

/// The `minecraftArguments` the mock version carries: the player name only.
const VANILLA_ARGUMENTS: &str = "--username ${auth_player_name}";

/// The stand-in for `java`.
///
/// It appends its whole argument list to `java-args.txt`, says it started, then waits. A
/// `SIGTERM` makes it exit 143, the code a shell reports for "terminated"; a `stop` file in
/// the root is the way a flow ends it without a signal. It gives up after 30 seconds so a
/// broken test cannot leave a process behind.
#[cfg(unix)]
const FAKE_JAVA: &str = "#!/bin/sh\n\
     root=$(dirname \"$0\")\n\
     printf '%s\\n' \"$@\" >> \"$root/java-args.txt\"\n\
     trap 'exit 143' TERM\n\
     echo started\n\
     i=0\n\
     while [ ! -f \"$root/stop\" ] && [ $i -lt 3000 ]; do\n\
       sleep 0.01 &\n\
       wait $!\n\
       i=$((i+1))\n\
     done\n\
     exit 0\n";

/// A window, the launcher behind it, and the mock hosts that launcher talks to.
pub struct TestApp {
    /// The app root. Kept so it is removed when the flow ends.
    dir: tempfile::TempDir,
    /// The window under test.
    pub window: AppWindow,
    /// The launcher the window drives, for assertions a flow makes about disk state.
    pub launcher: Arc<Launcher>,
    /// The mock metadata host. Kept alive for the length of the flow.
    _server: MockServer,
    /// The runtime the mock host runs on. Kept alive for the length of the flow.
    _rt: tokio::runtime::Runtime,
}

impl TestApp {
    /// Builds a launcher over a fresh root, points it at a mock Mojang and Fabric, and builds
    /// the same window the binary builds.
    pub fn new() -> TestApp {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime");
        let server = rt.block_on(async {
            let server = MockServer::start().await;
            mock_vanilla(&server, MC).await;
            mock_fabric(&server).await;
            server
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let uri = server.uri();

        // Only Mojang and Fabric are mocked. Every other host is a `.invalid` name, which no
        // resolver can answer, so a flow that reaches one fails fast instead of talking to
        // the real internet. A later task mocks Modrinth; until then a browser flow that
        // searches gets a connection error, which is the wanted answer for now.
        let endpoints = Endpoints {
            mojang: uri.clone(),
            modrinth: "http://modrinth.invalid".to_string(),
            curseforge: "http://curseforge.invalid".to_string(),
            loaders: LoaderEndpoints {
                fabric: uri.clone(),
                quilt: "http://quilt.invalid".to_string(),
                forge_meta: "http://forge-meta.invalid".to_string(),
                forge_maven: "http://forge-maven.invalid".to_string(),
                neoforge: "http://neoforge.invalid".to_string(),
            },
            msa: dead_msa_endpoints(),
        };
        let (launcher, rx) = Launcher::open_with_endpoints(dir.path().to_path_buf(), endpoints)
            .expect("build launcher");
        let launcher = launcher.with_secret_store(Box::new(MemoryStore::new()));

        let java = write_fake_java(dir.path());
        launcher
            .update_config(|config| config.jvm.java_path = Some(java))
            .expect("point the config at the stand-in java");
        launcher
            .accounts()
            .add(gcl_core::auth::offline::offline_account(PLAYER))
            .expect("add the offline account");

        let launcher = Arc::new(launcher);
        let window = gcl_ui::app::build(Arc::clone(&launcher), rx).expect("build the window");
        // A real window size, so every layout the flows click through has a sensible one.
        window
            .window()
            .set_size(slint::PhysicalSize::new(1200, 760));
        window.show().expect("show the window");

        let app = TestApp {
            dir,
            window,
            launcher,
            _server: server,
            _rt: rt,
        };
        pump();
        assert!(
            !app.ids().is_empty(),
            "the generated UI carries no element names, so no flow can address anything. \
             `crates/gcl-ui/build.rs` emits them only when PROFILE is `debug`; set \
             SLINT_EMIT_DEBUG_INFO=1 to force them on."
        );
        app
    }

    /// The app root every instance and the stand-in java live under.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The file the stand-in java appends its argument list to.
    pub fn java_args_file(&self) -> PathBuf {
        self.dir.path().join("java-args.txt")
    }

    /// Every argument the stand-in java has been given so far, one per line.
    pub fn java_args(&self) -> Vec<String> {
        std::fs::read_to_string(self.java_args_file())
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Asks the stand-in java to exit 0 on its own, without a signal.
    pub fn ask_java_to_stop(&self) {
        std::fs::write(self.dir.path().join("stop"), b"").expect("write the stop file");
    }

    /// The one element with this id, which must be present and visible.
    ///
    /// Exactly one match is the rule: a screen control is unique, so two matches mean two
    /// screens are mounted at once or a dialog is still up, and a flow that clicks the first
    /// of them would be testing the wrong window. A repeated row has the same id on every
    /// instance and is reached with [`TestApp::el_nth`] instead.
    ///
    /// A missing element panics with every id the window currently shows, which is what a
    /// flow needs to see when a screen or a dialog is not up.
    pub fn el(&self, id: &str) -> ElementHandle {
        let mut all = self.all(id);
        assert!(
            !all.is_empty(),
            "no element `{id}` is showing. Showing: {:?}",
            self.ids()
        );
        assert_eq!(
            all.len(),
            1,
            "`{id}` matches {} showing elements; use `el_nth` for a repeated row",
            all.len()
        );
        all.remove(0)
    }

    /// The nth element with this id, in tree order, for a control inside a repeater.
    pub fn el_nth(&self, id: &str, index: usize) -> ElementHandle {
        let mut all = self.all(id);
        assert!(
            index < all.len(),
            "`{id}` has only {} rows, wanted {index}",
            all.len()
        );
        all.remove(index)
    }

    /// Every element with this id, in tree order. A repeater gives each row the same id.
    ///
    /// The Slint compiler writes an element name with `-` in place of `_`, so a caller may
    /// use either: `InstancesScreen::create_button` and `InstancesScreen::create-button` are
    /// the same element.
    pub fn all(&self, id: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_element_id(&self.window, &id.replace('_', "-")).collect()
    }

    /// Whether an element with this id is showing.
    pub fn has(&self, id: &str) -> bool {
        !self.all(id).is_empty()
    }

    /// The ids of every element the window is showing, sorted and deduplicated.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .window
            .root_element()
            .query_descendants()
            .find_all()
            .into_iter()
            .filter_map(|element| element.id().map(|id| id.to_string()))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Presses the element with this id through its default accessible action.
    ///
    /// A disabled control fails the flow rather than doing nothing: "the button does nothing"
    /// is the bug these tests are looking for.
    pub fn click(&self, id: &str) {
        let element = self.el(id);
        assert_ne!(
            element.accessible_enabled(),
            Some(false),
            "`{id}` is showing but disabled"
        );
        element.invoke_accessible_default_action();
    }

    /// Presses the nth element with this id, for a control inside a repeater.
    pub fn click_nth(&self, id: &str, index: usize) {
        let element = self.el_nth(id, index);
        assert_ne!(
            element.accessible_enabled(),
            Some(false),
            "`{id}` row {index} is showing but disabled"
        );
        element.invoke_accessible_default_action();
    }

    /// Types `text` into the field with this id.
    pub fn type_into(&self, id: &str, text: &str) {
        self.el(id).set_accessible_value(text);
    }

    /// Picks the entry at `index` of the ComboBox with this id.
    ///
    /// A `ComboBox` offers `accessible-action-expand` and nothing else — no set-value, no
    /// increment — so the popup is opened through that action and then driven with the arrow
    /// keys its own key handler reads. Up is pressed until the value stops changing, which
    /// is row 0 whatever was selected, then Down `index` times, and Return closes the popup
    /// on the wanted row.
    pub fn select_combo(&self, id: &str, index: usize) {
        let combo = self.el(id);
        assert_ne!(
            combo.accessible_enabled(),
            Some(false),
            "`{id}` is showing but disabled"
        );
        combo.invoke_accessible_expand_action();
        pump();
        let mut value = combo.accessible_value();
        for pressed in 0..COMBO_UP_CAP {
            self.press_key(slint::platform::Key::UpArrow);
            let now = combo.accessible_value();
            if now == value {
                break;
            }
            value = now;
            assert!(
                pressed + 1 < COMBO_UP_CAP,
                "`{id}` still changed after {COMBO_UP_CAP} Up presses; it never reaches its \
                 first row"
            );
        }
        for _ in 0..index {
            self.press_key(slint::platform::Key::DownArrow);
        }
        self.press_key(slint::platform::Key::Return);
        pump();
    }

    /// Sends one key press and release to whatever holds the focus.
    pub fn press_key(&self, key: slint::platform::Key) {
        let text: slint::SharedString = key.into();
        let window = self.window.window();
        window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }

    /// Yields to the event loop until an element with this id is showing.
    pub async fn wait_for(&self, id: &str, timeout: Duration) {
        self.wait_until(&format!("`{id}` to show"), |_| self.has(id), timeout)
            .await;
    }

    /// Yields to the event loop until no element with this id is showing.
    pub async fn wait_gone(&self, id: &str, timeout: Duration) {
        self.wait_until(&format!("`{id}` to go"), |_| !self.has(id), timeout)
            .await;
    }

    /// Yields to the event loop until `pred` holds, or fails after `timeout`.
    ///
    /// `what` names the condition, so a timeout says what the UI never did. The wait is in
    /// real time because the launcher's worker threads are real threads.
    pub async fn wait_until(
        &self,
        what: &str,
        pred: impl Fn(&AppWindow) -> bool,
        timeout: Duration,
    ) {
        let started = Instant::now();
        loop {
            pump();
            if pred(&self.window) {
                return;
            }
            assert!(
                started.elapsed() < timeout,
                "timed out after {timeout:?} waiting for {what}. Showing: {:?}",
                self.ids()
            );
            yield_to_loop(Duration::from_millis(20)).await;
        }
    }
}

/// Instantiates whatever the last property change made visible and runs change handlers.
///
/// The testing backend only walks conditionals and repeaters when it is asked to, so a screen
/// or a dialog that a callback just opened is not in the tree until this runs.
pub fn pump() {
    i_slint_backend_testing::mock_elapsed_time(Duration::ZERO);
}

/// Hands the event loop `delay` of real time, then comes back.
pub async fn yield_to_loop(delay: Duration) {
    let fired = Rc::new(std::cell::Cell::new(false));
    let timer = slint::Timer::default();
    let mut armed = false;
    std::future::poll_fn(move |cx| {
        if fired.get() {
            return std::task::Poll::Ready(());
        }
        if !armed {
            armed = true;
            let fired = Rc::clone(&fired);
            let waker = cx.waker().clone();
            timer.start(slint::TimerMode::SingleShot, delay, move || {
                fired.set(true);
                waker.wake_by_ref();
            });
        }
        std::task::Poll::Pending
    })
    .await;
}

/// Runs `flow` on the Slint event loop and quits the loop when it ends.
///
/// A panic inside the flow is caught so the loop can be stopped, then raised again once it
/// has: a panic that escaped into the event loop would leave the test hanging.
pub fn run(flow: impl Future<Output = ()> + 'static) {
    let escaped: Rc<RefCell<Option<Box<dyn std::any::Any + Send>>>> = Rc::new(RefCell::new(None));
    let store = Rc::clone(&escaped);
    slint::spawn_local(async move {
        if let Err(panic) = AssertUnwindSafe(flow).catch_unwind().await {
            *store.borrow_mut() = Some(panic);
        }
        let _ = slint::quit_event_loop();
    })
    .expect("spawn the flow");
    slint::run_event_loop().expect("run the event loop");
    let panic = escaped.borrow_mut().take();
    if let Some(panic) = panic {
        std::panic::resume_unwind(panic);
    }
}

/// Initializes the Slint testing backend. One backend per process, so this runs once.
pub fn init_backend() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(i_slint_backend_testing::init_integration_test_with_system_time);
}

/// Writes the stand-in java into `dir` and returns its path.
#[cfg(unix)]
fn write_fake_java(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-java");
    std::fs::write(&path, FAKE_JAVA).expect("write the stand-in java");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

/// Serves `body` at `at` for every GET.
async fn serve(server: &MockServer, at: &str, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path_matcher(at.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
}

/// Serves a manifest, a version JSON, a client jar, and an empty asset index for `id`.
///
/// Copied from `gcl-core/tests/common`: a test binary cannot use another crate's test module.
/// The version has no libraries and no assets, so installing it fetches only the client jar.
async fn mock_vanilla(server: &MockServer, id: &str) {
    let base = server.uri();
    let index_body = serde_json::json!({ "objects": {} })
        .to_string()
        .into_bytes();
    let version = serde_json::json!({
        "id": id,
        "type": "release",
        "mainClass": "net.minecraft.client.main.Main",
        "minecraftArguments": VANILLA_ARGUMENTS,
        "libraries": [],
        "downloads": {
            "client": {
                "sha1": sha1_hex(CLIENT_JAR),
                "size": CLIENT_JAR.len(),
                "url": format!("{base}/vanilla/client.jar"),
            }
        },
        "assetIndex": {
            "id": "empty",
            "sha1": sha1_hex(&index_body),
            "size": index_body.len(),
            "url": format!("{base}/vanilla/index.json"),
        },
        "assets": "empty",
        "logging": { "client": {
            "argument": "-Dlog4j.configurationFile=${path}",
            "type": "log4j2-xml",
            "file": {
                "id": "client-1.12.xml",
                "sha1": sha1_hex(LOG_CONFIG),
                "size": LOG_CONFIG.len(),
                "url": format!("{base}/vanilla/log4j2.xml"),
            },
        }},
    })
    .to_string();
    let manifest = serde_json::json!({
        "latest": { "release": id, "snapshot": id },
        "versions": [{
            "id": id,
            "type": "release",
            "url": format!("{base}/vanilla/version.json"),
            "time": "2026-01-01T00:00:00+00:00",
            "releaseTime": "2026-01-01T00:00:00+00:00",
            "sha1": sha1_hex(version.as_bytes()),
            "complianceLevel": 1,
        }]
    })
    .to_string();

    serve(server, MANIFEST_PATH, manifest.into_bytes()).await;
    serve(server, "/vanilla/version.json", version.into_bytes()).await;
    serve(server, "/vanilla/client.jar", CLIENT_JAR.to_vec()).await;
    serve(server, "/vanilla/index.json", index_body).await;
    serve(server, "/vanilla/log4j2.xml", LOG_CONFIG.to_vec()).await;
}

/// Serves the Fabric loader list and the loader profile for [`MC`] from the recorded fixtures.
async fn mock_fabric(server: &MockServer) {
    serve(
        server,
        &format!("/v2/versions/loader/{MC}"),
        FABRIC_LOADERS.as_bytes().to_vec(),
    )
    .await;
    serve(
        server,
        &format!("/v2/versions/loader/{MC}/{FABRIC}/profile/json"),
        FABRIC_PROFILE.as_bytes().to_vec(),
    )
    .await;
}

/// Microsoft endpoints no request can reach: no flow signs in.
fn dead_msa_endpoints() -> gcl_core::auth::msa::MsaEndpoints {
    let base = "http://msa.invalid";
    gcl_core::auth::msa::MsaEndpoints {
        device_code: format!("{base}/devicecode"),
        token: format!("{base}/token"),
        xbl: format!("{base}/xbl"),
        xsts: format!("{base}/xsts"),
        mc_login: format!("{base}/mclogin"),
        profile: format!("{base}/profile"),
    }
}
