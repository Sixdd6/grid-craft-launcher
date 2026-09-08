//! Test harness for the GUI flow tests: a real `AppWindow` over a real `Launcher`, driven
//! through the Slint testing backend.
//!
//! The window, the launcher, and the mock metadata host all live in [`TestApp`]. A flow is an
//! async block handed to [`run`], which starts the Slint event loop and quits it when the
//! flow ends. Every wait in a flow yields to that loop, so the bridge's worker threads post
//! their results back exactly as they do in the running app.
//!
//! Every `tests/flow_*.rs` binary compiles this module and uses part of it, so the allow is
//! what keeps the parts one binary does not need from reading as dead code.
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

/// How far one [`TestApp::scroll`] step moves a scroll view, in logical pixels.
const SCROLL_STEP: f32 = 80.0;

/// How many scroll steps [`TestApp::scroll_to`] takes before it gives up.
const SCROLL_STEPS: usize = 60;

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

/// Which mock hosts a [`TestApp`] mounts on top of Mojang and Fabric.
///
/// Everything left off is a `.invalid` name, which no resolver can answer, so a flow that
/// reaches a host it did not ask for fails fast instead of talking to the real internet.
#[derive(Clone, Copy, Default)]
pub struct Mocks {
    /// Mount Modrinth: search, modpack search, one project, its versions, and the files
    /// those versions point at.
    pub modrinth: bool,
    /// Mount the six Microsoft sign-in endpoints and configure a client id, so the accounts
    /// screen offers Microsoft sign-in.
    pub msa: bool,
    /// Make the mock token endpoint answer `authorization_pending` for good, so a sign-in
    /// waits until something cancels it. Only read when `msa` is set.
    pub msa_pending: bool,
    /// Mount the details endpoints the project screen needs on top of `modrinth`: a second,
    /// older version of the mod, each version with a file of its own.
    ///
    /// Off, the version list holds only the newest version, which is what the content flows
    /// expect from an add that names no version.
    pub project: bool,
    /// Add an offline account before the window is built.
    ///
    /// Every flow that launches the game needs one; the accounts flow starts from an empty
    /// store, because adding the first account is what it tests.
    pub player: bool,
}

impl Mocks {
    /// What the instances flows want: Mojang, Fabric, and a player to launch as.
    pub fn plain() -> Mocks {
        Mocks {
            player: true,
            ..Mocks::default()
        }
    }

    /// What the content and modpack flows want: the above, with Modrinth mounted.
    pub fn modrinth() -> Mocks {
        Mocks {
            modrinth: true,
            player: true,
            ..Mocks::default()
        }
    }

    /// What the project details flow wants: the above, with the second version mounted.
    pub fn project() -> Mocks {
        Mocks {
            modrinth: true,
            project: true,
            player: true,
            ..Mocks::default()
        }
    }

    /// What the Microsoft half of the accounts flow wants: no account, sign-in on.
    pub fn msa() -> Mocks {
        Mocks {
            msa: true,
            ..Mocks::default()
        }
    }

    /// The same, with a sign-in that never finishes on its own, so Cancel is testable.
    pub fn msa_pending() -> Mocks {
        Mocks {
            msa: true,
            msa_pending: true,
            ..Mocks::default()
        }
    }
}

impl TestApp {
    /// Builds a launcher over a fresh root, points it at a mock Mojang and Fabric, and builds
    /// the same window the binary builds.
    pub fn new() -> TestApp {
        TestApp::with(Mocks::plain())
    }

    /// [`TestApp::new`], with the extra mock hosts `mocks` names.
    pub fn with(mocks: Mocks) -> TestApp {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime");
        let server = rt.block_on(async {
            let server = MockServer::start().await;
            mock_vanilla(&server, MC).await;
            mock_fabric(&server).await;
            if mocks.modrinth {
                mock_modrinth(&server, mocks.project).await;
            }
            if mocks.msa {
                mock_msa(&server, mocks.msa_pending).await;
            }
            server
        });
        let dir = tempfile::tempdir().expect("tempdir");
        let uri = server.uri();

        let endpoints = Endpoints {
            mojang: uri.clone(),
            modrinth: if mocks.modrinth {
                uri.clone()
            } else {
                "http://modrinth.invalid".to_string()
            },
            curseforge: "http://curseforge.invalid".to_string(),
            loaders: LoaderEndpoints {
                fabric: uri.clone(),
                quilt: "http://quilt.invalid".to_string(),
                forge_meta: "http://forge-meta.invalid".to_string(),
                forge_maven: "http://forge-maven.invalid".to_string(),
                neoforge: "http://neoforge.invalid".to_string(),
            },
            msa: if mocks.msa {
                msa_endpoints(&uri)
            } else {
                dead_msa_endpoints()
            },
        };
        let (launcher, rx) = Launcher::open_with_endpoints(dir.path().to_path_buf(), endpoints)
            .expect("build launcher");
        let launcher = launcher
            .with_secret_store(Box::new(MemoryStore::new()))
            // A modpack may only fetch its files from the hosts the mrpack specification
            // names, and the mock server is not one of them.
            .with_pack_hosts(vec!["127.0.0.1".to_string(), "localhost".to_string()])
            // A project icon may only be fetched from a source's own CDN, and the mock
            // server is not one of those either. Without this the search rows' icons would
            // be fetched from the real `cdn.modrinth.com` the fixture names.
            .with_icon_hosts(vec!["127.0.0.1".to_string(), "localhost".to_string()])
            // A description image may come from any host, but only over https, and the mock
            // server speaks plain HTTP. This lifts the scheme rule for it alone; there is no
            // host allowlist to extend.
            .with_image_hosts(vec!["127.0.0.1".to_string(), "localhost".to_string()]);

        let java = write_fake_java(dir.path());
        launcher
            .update_config(|config| {
                config.jvm.java_path = Some(java);
                if mocks.msa {
                    config.keys.msa_client_id = Some(MSA_CLIENT_ID.to_string());
                }
            })
            .expect("write the test config");
        if mocks.player {
            launcher
                .accounts()
                .add(gcl_core::auth::offline::offline_account(PLAYER))
                .expect("add the offline account");
        }

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

    /// Puts this window back on screen after a [`TestApp::hide`].
    pub fn show(&self) {
        self.window.show().expect("show the window");
        pump();
    }

    /// Takes this window off screen, so a flow can carry on over a second [`TestApp`].
    ///
    /// One process holds one Slint backend but may hold several windows. Every lookup is
    /// scoped to one window, so the old one only has to stop being drawn.
    pub fn hide(&self) {
        self.window.hide().expect("hide the window");
    }

    /// The base URL of the mock host, for a flow that builds a file pointing back at it.
    pub fn server_uri(&self) -> String {
        self._server.uri()
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

    /// Whether the window is showing an element with this accessible label.
    ///
    /// A `Text` carries its own text as its label, so this reads a line the user sees whose
    /// element has no id of its own to look up — a dialog's title, for one.
    pub fn has_label(&self, label: &str) -> bool {
        ElementHandle::find_by_accessible_label(&self.window, label)
            .next()
            .is_some()
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

    /// Clicks the middle of the element with this id, with real pointer events.
    ///
    /// The accessible action skips hit-testing, so it presses a button that no pointer could
    /// reach: one under a modal overlay, or one whose `TouchArea` still holds a grab from an
    /// earlier click. Those are the defects these flows exist to catch, so a click here is a
    /// move, a press, and a release at the element's centre, exactly as a mouse sends them.
    /// [`TestApp::activate`] is the accessible action, for the few controls hit-testing
    /// cannot reach.
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
        self.click_element(&element);
    }

    /// Clicks the nth element with this id, for a control inside a repeater.
    pub fn click_nth(&self, id: &str, index: usize) {
        let element = self.el_nth(id, index);
        assert_ne!(
            element.accessible_enabled(),
            Some(false),
            "`{id}` row {index} is showing but disabled"
        );
        self.click_element(&element);
    }

    /// Presses the element with this id through its default accessible action.
    ///
    /// For a control a pointer cannot land on: a row inside a `ComboBox` popup, which is not
    /// laid out in the window's own coordinate space.
    pub fn activate(&self, id: &str) {
        let element = self.el(id);
        assert_ne!(
            element.accessible_enabled(),
            Some(false),
            "`{id}` is showing but disabled"
        );
        element.invoke_accessible_default_action();
    }

    /// The same, for the nth element with this id.
    pub fn activate_nth(&self, id: &str, index: usize) {
        let element = self.el_nth(id, index);
        assert_ne!(
            element.accessible_enabled(),
            Some(false),
            "`{id}` row {index} is showing but disabled"
        );
        element.invoke_accessible_default_action();
    }

    /// Sends a move, a press, and a release at the middle of `element`.
    ///
    /// The middle has to be inside every scroll viewport the element sits in, or the pointer
    /// lands on the clipped-away part and nothing is pressed. That is a harness fault, not a
    /// UI one, so it fails here with the two rectangles rather than as a silent no-op later:
    /// the caller wanted a [`TestApp::scroll_to`] first.
    fn click_element(&self, element: &ElementHandle) {
        // The whole rectangle has to be inside the viewport, not only its middle: a row
        // whose middle is still on screen but whose lower half is clipped is drawn cut in
        // half, and a click on it is not the click a user could make.
        assert!(
            self.fully_in_view(element),
            "`{}` at {:?} sized {:?} is not fully inside a scroll viewport ({:?}); \
             scroll it into view first",
            element.id().unwrap_or_default(),
            element.absolute_position(),
            element.size(),
            self.viewports()
        );
        let position = center_of(element);
        let window = self.window.window();
        let button = slint::platform::PointerEventButton::Left;
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
        pump();
        window.dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
        pump();
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
        pump();
        window.dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
        pump();
    }

    /// The scroll viewports the window is showing, as position and size.
    ///
    /// A `ScrollView` clips its content, so an element the element tree still reports may be
    /// scrolled past the edge and unreachable by a pointer. [`TestApp::scroll_to`] and
    /// [`TestApp::click_element`] both measure against these.
    fn viewports(&self) -> Vec<(slint::LogicalPosition, slint::LogicalSize)> {
        self.all("ScrollView::flickable")
            .iter()
            .map(|view| (view.absolute_position(), view.size()))
            .collect()
    }

    /// Whether every corner of `element` is inside each scroll viewport it overlaps.
    fn fully_in_view(&self, element: &ElementHandle) -> bool {
        let at = element.absolute_position();
        let size = element.size();
        self.viewports().iter().all(|(view_at, view_size)| {
            let overlaps = view_at.x < at.x + size.width
                && view_at.x + view_size.width > at.x
                && view_at.y < at.y + size.height
                && view_at.y + view_size.height > at.y;
            let contained = at.x >= view_at.x
                && at.y >= view_at.y
                && at.x + size.width <= view_at.x + view_size.width
                && at.y + size.height <= view_at.y + view_size.height;
            !overlaps || contained
        })
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

    /// Drags the slider with this id to `fraction` of its width, from 0.0 to 1.0.
    ///
    /// A press, a move, and a release, the way a mouse does it: the widget saves on the
    /// release, so nothing else in the harness can stand in for a real drag. The value that
    /// lands is where the pointer is, so a caller reads it back rather than predicting it.
    pub fn drag_slider(&self, id: &str, fraction: f32) {
        let element = self.el(id);
        assert_ne!(
            element.accessible_enabled(),
            Some(false),
            "`{id}` is showing but disabled"
        );
        let at = element.absolute_position();
        let size = element.size();
        let position = slint::LogicalPosition::new(
            at.x + size.width * fraction.clamp(0.0, 1.0),
            at.y + size.height / 2.0,
        );
        let window = self.window.window();
        let button = slint::platform::PointerEventButton::Left;
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
        window.dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
        window.dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
        pump();
    }

    /// Scrolls the window by `delta` logical pixels at its middle, where every screen's
    /// scroll view sits.
    ///
    /// A negative delta scrolls down. The move comes first so the flickable under the
    /// pointer is the one that gets the wheel.
    pub fn scroll(&self, delta: f32) {
        let size = self.window.window().size();
        let position =
            slint::LogicalPosition::new(size.width as f32 / 2.0, size.height as f32 / 2.0);
        let window = self.window.window();
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
        window.dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: delta,
        });
        pump();
    }

    /// Scrolls down until an element with this id is completely inside its scroll viewport.
    ///
    /// Being in the element tree is not enough: a `ScrollView` clips what it draws, so a row
    /// that is only half past the bottom edge is still found, still reports a position, and
    /// still swallows a pointer click aimed at its middle. The loop therefore runs until
    /// [`TestApp::fully_in_view`] holds, not until [`TestApp::has`] does.
    pub fn scroll_to(&self, id: &str) {
        // `0..=SCROLL_STEPS` so the state after the last scroll is checked too: the range
        // runs one more time than it scrolls, and the last pass only reads.
        for step in 0..=SCROLL_STEPS {
            let found = self.all(id);
            let element = found.first();
            if let Some(element) = element
                && self.fully_in_view(element)
            {
                return;
            }
            if step == SCROLL_STEPS {
                break;
            }
            // Toward the element: back up when it is above the viewport, on down when it is
            // below. A fixed direction walks away from a row that is already past the top
            // edge and never reaches it.
            let direction = match element {
                Some(element) => self.scroll_direction(element),
                None => -1.0,
            };
            self.scroll(direction * SCROLL_STEP);
        }
        panic!(
            "`{id}` never came fully into view. Showing: {:?}",
            self.ids()
        );
    }

    /// Which way to scroll to bring `element` into view: `1.0` for up, `-1.0` for down.
    ///
    /// The scroll view an element belongs to is the one it sits inside horizontally, so a
    /// second scroll view beside it does not decide the direction. An element that is inside
    /// every viewport it belongs to, and still not fully in view, is scrolled down, which is
    /// the direction a flow wants for a list it has not walked yet.
    fn scroll_direction(&self, element: &ElementHandle) -> f32 {
        let at = element.absolute_position();
        let size = element.size();
        let centre_x = at.x + size.width / 2.0;
        for (view_at, view_size) in self.viewports() {
            if centre_x < view_at.x || centre_x > view_at.x + view_size.width {
                continue;
            }
            if at.y < view_at.y {
                return 1.0;
            }
            if at.y + size.height > view_at.y + view_size.height {
                return -1.0;
            }
        }
        -1.0
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

/// The middle of an element, in window coordinates.
fn center_of(element: &ElementHandle) -> slint::LogicalPosition {
    let at = element.absolute_position();
    let size = element.size();
    slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0)
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
    ONCE.call_once(|| {
        // Both variables win over the config file, and `just` loads a `.env`, so a real id
        // or key on this machine must not decide whether Microsoft sign-in and CurseForge
        // are on.
        // SAFETY: nextest runs every test binary in its own process, and every flow calls
        // this first, before any `TestApp` and so before any thread that could read the
        // environment.
        unsafe {
            std::env::remove_var("GCL_MSA_CLIENT_ID");
            std::env::remove_var("CURSEFORGE_API_KEY");
        }
        i_slint_backend_testing::init_integration_test_with_system_time();
    });
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

/// Serves Modrinth: a content search, a modpack search, one project, and its versions.
///
/// The two searches share the `/search` path, so they are told apart by the `facets` value:
/// only a modpack search asks for `project_type:modpack`, which reaches the query string
/// percent-encoded as [`MODPACK_FACET`]. The matchers are opposites, so the order they are
/// mounted in cannot decide which one answers.
async fn mock_modrinth(server: &MockServer, project: bool) {
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path_matcher("/search"))
        .and(|req: &wiremock::Request| !query_of(req).contains(MODPACK_FACET))
        .respond_with(ResponseTemplate::new(200).set_body_string(mod_search(&base)))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_matcher("/search"))
        .and(|req: &wiremock::Request| query_of(req).contains(MODPACK_FACET))
        .respond_with(ResponseTemplate::new(200).set_body_string(MODRINTH_PACKS))
        .mount(server)
        .await;

    serve(
        server,
        &format!("/project/{MOD_PROJECT}"),
        mod_project(&base).into_bytes(),
    )
    .await;
    serve(
        server,
        &format!("/project/{MOD_PROJECT}/version"),
        mod_versions(&base, project).into_bytes(),
    )
    .await;
    serve(server, MOD_FILE_PATH, MOD_JAR.to_vec()).await;
    serve(server, OLD_MOD_FILE_PATH, OLD_MOD_JAR.to_vec()).await;
    serve(server, ICON_PATH, ICON_PNG.to_vec()).await;
    serve(server, DESC_IMAGE_PATH, ICON_PNG.to_vec()).await;

    if project {
        // A version's changelog comes from the single-version endpoint, which the version
        // list's `include_changelog=false` does not filter. The newest version answers with
        // notes; the older one answers 500, so the flow can drive the failure path too.
        serve(
            server,
            &format!("/version/{MOD_VERSION_ID}"),
            mod_version_with_changelog(&base).into_bytes(),
        )
        .await;
        Mock::given(method("GET"))
            .and(path_matcher(format!("/version/{OLD_MOD_VERSION_ID}")))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(server)
            .await;
    }

    let pack = mrpack_bytes(&base, PACK_NAME);
    serve(
        server,
        &format!("/project/{PACK_PROJECT}/version"),
        pack_versions(&base, &pack).into_bytes(),
    )
    .await;
    serve(server, PACK_FILE_PATH, pack).await;
    serve(server, PACK_MOD_PATH_URL, PACK_MOD_JAR.to_vec()).await;
}

/// The `project_type:modpack` facet as it appears in the raw query string.
///
/// The client percent-encodes the whole JSON `facets` value, so the colon arrives as `%3A`.
/// Matching on this rather than on the word `modpack` keeps a pack whose name carries that
/// word out of the decision.
const MODPACK_FACET: &str = "project_type%3Amodpack";

/// The query string of a request, without the leading `?`.
fn query_of(req: &wiremock::Request) -> String {
    req.url.query().unwrap_or_default().to_string()
}

/// Project id of the mod every content flow installs. `project_sodium.json` names it.
pub const MOD_PROJECT: &str = "AANobbMI";

/// Title of that project, which is the row's accessible label.
pub const MOD_TITLE: &str = "Sodium";

/// Bytes the mock serves as that mod's jar.
const MOD_JAR: &[u8] = b"synthetic sodium jar";

/// Bytes the mock serves as the older version's jar.
const OLD_MOD_JAR: &[u8] = b"synthetic older sodium jar";

/// Path the mock serves those bytes at.
const OLD_MOD_FILE_PATH: &str = "/files/sodium-old.jar";

/// File name the older version's file carries, which is what lands in `mods/`.
pub const OLD_MOD_FILE_NAME: &str = "sodium-fabric-0.5.12-beta.2+mc1.20.1.jar";

/// Version number of the newest version of that mod, as the versions tab shows it.
pub const MOD_VERSION: &str = "mc1.20.1-0.5.13-fabric";

/// Version number of the older one.
pub const OLD_MOD_VERSION: &str = "mc1.20.1-0.5.12-beta.2-fabric";

/// Version id of the newest version, which is the one whose changelog the mock serves.
///
/// [`mod_versions`] checks the recorded list still names it, so a re-recorded fixture fails
/// here rather than leaving the changelog mock silently unmatched.
pub const MOD_VERSION_ID: &str = "OihdIimA";

/// Version id of the older one, whose changelog request the mock answers 500.
pub const OLD_MOD_VERSION_ID: &str = "ryOMVRuG";

/// The changelog the mock serves for [`MOD_VERSION_ID`], as Modrinth markdown.
const MOD_CHANGELOG: &str = "\
## What's new

- Faster chunk loading
";

/// The heading the notes modal must show for that changelog.
pub const MOD_CHANGELOG_HEADING: &str = "What's new";

/// The bullet under it.
pub const MOD_CHANGELOG_BULLET: &str = "Faster chunk loading";

/// Path the mock serves the project icon at.
const ICON_PATH: &str = "/icons/sodium.png";

/// A 2x2 RGBA PNG, the smallest thing the icon decoder can answer with.
const ICON_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x17, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x0c, 0x19, 0x18, 0xfe, 0xff, 0xff, 0xff, 0x9f, 0xe1, 0x3f, 0x00, 0x47, 0xca, 0x08, 0xf8,
    0xfd, 0x5d, 0xa3, 0xca, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Path the mock serves the one image the description names at.
///
/// A description image is fetched from any host, unlike an icon, so this points at the mock
/// server rather than a CDN: a body left as recorded would fetch a real banner over the real
/// internet.
const DESC_IMAGE_PATH: &str = "/images/banner.png";

/// The description the mock serves for that project.
///
/// It carries every shape the description tab has to answer for: a heading, a paragraph, an
/// image, a quote, a rule, a pipe table, and a bullet list with one nested level.
fn mod_body(base: &str) -> String {
    format!(
        "\
# {MOD_BODY_HEADING}

A rendering engine that improves frame rates.

![{MOD_BODY_IMAGE_ALT}]({base}{DESC_IMAGE_PATH})

> {MOD_BODY_QUOTE}

---

| {MOD_BODY_TABLE_HEADER} | Default |
|---|---|
| Chunk updates | 1 |

## Features

- {MOD_BODY_BULLET}
  - {MOD_BODY_NESTED_BULLET}
- Much better frame pacing
"
    )
}

/// The heading text the description tab must show first.
pub const MOD_BODY_HEADING: &str = "Sodium";

/// The first bullet the description tab must show.
pub const MOD_BODY_BULLET: &str = "Significantly improved frame rates";

/// The bullet one level in under it, which must reach the screen as `depth: 1`.
pub const MOD_BODY_NESTED_BULLET: &str = "Faster chunk loading";

/// The quote the description carries, which must reach a `quote` block of its own.
pub const MOD_BODY_QUOTE: &str = "Sodium does not change the game's visuals.";

/// The first cell of the description table's header row.
pub const MOD_BODY_TABLE_HEADER: &str = "Setting";

/// The alt text of the description's one image, which is what a failed load would show.
pub const MOD_BODY_IMAGE_ALT: &str = "banner";

/// Path the mock serves those bytes at.
const MOD_FILE_PATH: &str = "/files/sodium.jar";

/// File name the version fixture gives that jar, which is what lands in `mods/`.
pub const MOD_FILE_NAME: &str = "sodium-fabric-0.5.13+mc1.20.1.jar";

/// That file name without its extension, which is what the content list shows.
pub const MOD_FILE_STEM: &str = "sodium-fabric-0.5.13+mc1.20.1";

/// Project id of the modpack the browser installs. `search_packs.json` names it first.
pub const PACK_PROJECT: &str = "1KVo5zza";

/// Title of that modpack, which is its row's accessible label.
pub const PACK_TITLE: &str = "Fabulously Optimized";

/// Name inside the synthetic `.mrpack` the mock serves.
const PACK_NAME: &str = "Fabulously Optimized";

/// Path the mock serves that `.mrpack` at.
const PACK_FILE_PATH: &str = "/files/pack.mrpack";

/// Bytes of the one mod a synthetic pack installs.
const PACK_MOD_JAR: &[u8] = b"synthetic pack mod jar";

/// Path the mock serves those bytes at.
const PACK_MOD_PATH_URL: &str = "/files/pack-mod.jar";

/// Where that mod lands inside the instance a pack import creates.
pub const PACK_MOD_PATH: &str = "mods/pack-mod.jar";

const MODRINTH_SEARCH: &str =
    include_str!("../../../../tests/fixtures/modrinth/search_sodium.json");
const MODRINTH_PACKS: &str = include_str!("../../../../tests/fixtures/modrinth/search_packs.json");
const MODRINTH_PROJECT: &str =
    include_str!("../../../../tests/fixtures/modrinth/project_sodium.json");
const MODRINTH_VERSIONS: &str =
    include_str!("../../../../tests/fixtures/modrinth/versions_sodium_1.20.1_fabric.json");

/// The recorded version list, pointed at the mock host.
///
/// The recorded file names Modrinth's CDN and the real jar's hash, neither of which a test
/// can reach, so each version's one primary file is rewritten to bytes this mock serves.
/// Everything else — the version id, the Minecraft versions, the loaders — is the fixture's
/// own. With `both` unset only the newest version is served, which is what an add that names
/// no version installs; the project flow needs the older one as well, to install over.
fn mod_versions(base: &str, both: bool) -> String {
    let mut versions: serde_json::Value =
        serde_json::from_str(MODRINTH_VERSIONS).expect("read the recorded version list");
    let list = versions
        .as_array_mut()
        .expect("the recorded version list is an array");
    list.truncate(if both { 2 } else { 1 });
    assert_eq!(
        list.len(),
        if both { 2 } else { 1 },
        "the recorded version list is shorter than the flows need"
    );
    list[0]["files"] = one_file(base, MOD_FILE_PATH, MOD_FILE_NAME, MOD_JAR);
    assert_eq!(
        list[0]["id"], MOD_VERSION_ID,
        "the changelog mock is mounted under this id"
    );
    if both {
        list[1]["files"] = one_file(base, OLD_MOD_FILE_PATH, OLD_MOD_FILE_NAME, OLD_MOD_JAR);
        assert_eq!(
            list[1]["id"], OLD_MOD_VERSION_ID,
            "the failing changelog mock is mounted under this id"
        );
    }
    versions.to_string()
}

/// The newest recorded version on its own, as the single-version endpoint answers it, with a
/// changelog the version list itself never carries.
fn mod_version_with_changelog(base: &str) -> String {
    let mut versions: serde_json::Value =
        serde_json::from_str(MODRINTH_VERSIONS).expect("read the recorded version list");
    let list = versions
        .as_array_mut()
        .expect("the recorded version list is an array");
    let mut version = list[0].take();
    version["files"] = one_file(base, MOD_FILE_PATH, MOD_FILE_NAME, MOD_JAR);
    version["changelog"] = serde_json::json!(MOD_CHANGELOG);
    version.to_string()
}

/// One primary file, served by the mock host at `path`, with the sha1 of the bytes it serves.
fn one_file(base: &str, path: &str, name: &str, bytes: &[u8]) -> serde_json::Value {
    serde_json::json!([{
        "hashes": { "sha1": sha1_hex(bytes) },
        "url": format!("{base}{path}"),
        "filename": name,
        "primary": true,
        "size": bytes.len(),
        "file_type": serde_json::Value::Null,
    }])
}

/// The recorded search page, with every hit's icon pointed at the mock host.
///
/// The recording names `cdn.modrinth.com`, which is on the icon allowlist, so a browser flow
/// left as recorded would fetch real icons over the real internet.
fn mod_search(base: &str) -> String {
    let mut found: serde_json::Value =
        serde_json::from_str(MODRINTH_SEARCH).expect("read the recorded search page");
    let hits = found["hits"]
        .as_array_mut()
        .expect("the recorded search page has hits");
    for hit in hits.iter_mut() {
        hit["icon_url"] = serde_json::json!(format!("{base}{ICON_PATH}"));
    }
    // The second hit carries a title no row can fit on one line and a description written
    // over three lines, so a browser flow can prove the row wraps the whole title and folds
    // the line breaks away. The first hit is left as recorded: every other flow opens it.
    hits[1]["title"] = serde_json::json!(LONG_TITLE);
    hits[1]["description"] = serde_json::json!(MULTILINE_DESCRIPTION);
    found.to_string()
}

/// A 90-character title, longer than any row is wide.
pub const LONG_TITLE: &str =
    "Sodium Extra with a title long enough to wrap over more than one line in any browser row!!";

/// A description broken over three lines, with both line endings in it.
pub const MULTILINE_DESCRIPTION: &str = "A Sodium addon.\nIt adds options\r\nand more options.";

/// The recorded project, with its icon pointed at the mock host and a description written
/// for the flow: a heading, a paragraph, an image the mock serves, a quote, a rule, a table,
/// and a bullet list one level deep.
fn mod_project(base: &str) -> String {
    let mut project: serde_json::Value =
        serde_json::from_str(MODRINTH_PROJECT).expect("read the recorded project");
    project["icon_url"] = serde_json::json!(format!("{base}{ICON_PATH}"));
    project["body"] = serde_json::json!(mod_body(base));
    project.to_string()
}

/// One modpack version whose primary file is the synthetic `.mrpack` the mock serves.
fn pack_versions(base: &str, pack: &[u8]) -> String {
    serde_json::json!([{
        "id": "packv1",
        "project_id": PACK_PROJECT,
        "name": "1.0.0",
        "version_number": "1.0.0",
        "version_type": "release",
        "date_published": "2026-01-01T00:00:00Z",
        "game_versions": [MC],
        "loaders": ["fabric"],
        "dependencies": [],
        "files": [{
            "hashes": { "sha1": sha1_hex(pack) },
            "url": format!("{base}{PACK_FILE_PATH}"),
            "filename": "pack.mrpack",
            "primary": true,
            "size": pack.len(),
            "file_type": serde_json::Value::Null,
        }],
    }])
    .to_string()
}

/// Builds a `.mrpack` for [`MC`] and Fabric with one downloaded file and one override.
///
/// The file points back at the mock host, which is why the launcher under test is built with
/// [`gcl_core::Launcher::with_pack_hosts`].
pub fn mrpack_bytes(base: &str, name: &str) -> Vec<u8> {
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "name": name,
        "versionId": "1.0.0",
        "dependencies": { "minecraft": MC, "fabric-loader": FABRIC },
        "files": [{
            "path": PACK_MOD_PATH,
            "hashes": { "sha1": sha1_hex(PACK_MOD_JAR) },
            "env": { "client": "required", "server": "required" },
            "downloads": [format!("{base}{PACK_MOD_PATH_URL}")],
            "fileSize": PACK_MOD_JAR.len(),
        }],
    })
    .to_string();
    zip_bytes(&[
        ("modrinth.index.json", index.into_bytes()),
        ("overrides/config/pack.txt", b"from the pack\n".to_vec()),
    ])
}

/// Writes a zip with these entries, in order.
fn zip_bytes(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(*name, options).expect("start the entry");
            zip.write_all(bytes).expect("write the entry");
        }
        zip.finish().expect("finish the zip");
    }
    buf.into_inner()
}

/// Client id the Microsoft mock signs in with. Not a real one: nothing leaves the process.
pub const MSA_CLIENT_ID: &str = "00000000-0000-0000-0000-000000000001";

/// The code the mock device-code endpoint hands out.
pub const MSA_USER_CODE: &str = "ABCD-EFGH";

/// The page the mock tells the user to open.
pub const MSA_URI: &str = "https://microsoft.com/link";

/// Player name the mock profile carries, which is the account row's name.
pub const MSA_NAME: &str = "Notch";

/// Serves the six Microsoft sign-in endpoints with fixed answers.
///
/// `interval` is zero, so the token poll answers on the first try and no flow waits on a
/// real clock. Copied from `gcl-core/tests/common`: a test binary cannot use another crate's
/// test module.
async fn mock_msa(server: &MockServer, pending: bool) {
    post(
        server,
        "/devicecode",
        &format!(
            r#"{{"user_code":"{MSA_USER_CODE}","device_code":"dev-secret",
                 "verification_uri":"{MSA_URI}","expires_in":900,"interval":0,
                 "message":"Sign in."}}"#
        ),
    )
    .await;
    if pending {
        // The OAuth "keep waiting" answer, which only a 400 carries. The sign-in polls until
        // the user cancels it, which is what the Cancel sub-flow needs.
        Mock::given(method("POST"))
            .and(path_matcher("/token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":"authorization_pending"}"#),
            )
            .mount(server)
            .await;
    } else {
        post(
            server,
            "/token",
            r#"{"access_token":"msa-access","refresh_token":"refresh-1"}"#,
        )
        .await;
    }
    post(
        server,
        "/xbl",
        r#"{"Token":"xbl-token","DisplayClaims":{"xui":[{"uhs":"user-hash"}]}}"#,
    )
    .await;
    post(
        server,
        "/xsts",
        r#"{"Token":"xsts-token","DisplayClaims":{"xui":[{"uhs":"user-hash","xid":"2535"}]}}"#,
    )
    .await;
    post(
        server,
        "/mclogin",
        r#"{"access_token":"mc-token","expires_in":86400}"#,
    )
    .await;
    serve(
        server,
        "/profile",
        format!(r#"{{"id":"069a79f444e94726a5befca90e38aaf5","name":"{MSA_NAME}"}}"#).into_bytes(),
    )
    .await;
}

/// Serves `body` at `at` for every POST.
async fn post(server: &MockServer, at: &str, body: &str) {
    Mock::given(method("POST"))
        .and(path_matcher(at.to_string()))
        .respond_with(ResponseTemplate::new(200).set_body_string(body.to_string()))
        .mount(server)
        .await;
}

/// The six Microsoft login endpoints, all on the mock host.
fn msa_endpoints(base: &str) -> gcl_core::auth::msa::MsaEndpoints {
    gcl_core::auth::msa::MsaEndpoints {
        device_code: format!("{base}/devicecode"),
        token: format!("{base}/token"),
        xbl: format!("{base}/xbl"),
        xsts: format!("{base}/xsts"),
        mc_login: format!("{base}/mclogin"),
        profile: format!("{base}/profile"),
    }
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
