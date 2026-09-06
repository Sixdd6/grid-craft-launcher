//! Builds the java command line for one launch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::Error;
use crate::auth::LaunchIdentity;
use crate::events::EventSink;
use crate::instances::Instance;
use crate::mojang::args::{ArgContext, default_legacy_jvm_args, expand_arguments, expand_legacy};
use crate::mojang::assets::{AssetIndex, materialize_legacy};
use crate::mojang::install::InstallPlan;
use crate::mojang::rules::RuleContext;
use crate::mojang::version::Argument;
use crate::paths::Root;

/// Heap size and extra flags for the JVM, already resolved from the instance override or the
/// config defaults by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JvmSettings {
    /// Initial heap in MiB, passed as `-Xms`.
    pub min_mib: u32,
    /// Maximum heap in MiB, passed as `-Xmx`.
    pub max_mib: u32,
    /// Extra JVM flags, inserted after the heap flags and before the version's own JVM args.
    pub extra_args: Vec<String>,
}

/// Everything [`build`] needs to produce a command line.
pub struct LaunchInputs<'a> {
    /// The install plan for the version being launched; carries the resolved version JSON.
    pub plan: &'a InstallPlan,
    /// The instance the game runs in; its `.minecraft` is the working directory.
    pub instance: &'a Instance,
    /// Placeholders for the signed-in player.
    pub identity: &'a LaunchIdentity,
    /// Path to the java binary.
    pub java: &'a Path,
    /// App root, source of the assets and libraries directories.
    pub root: &'a Root,
    /// Heap and extra JVM flags.
    pub jvm: JvmSettings,
    /// Rule context for this machine. `build` adds the launch feature flags to a clone of it.
    pub rules: &'a RuleContext,
    /// Value of `${launcher_name}`.
    pub launcher_name: &'a str,
    /// Value of `${launcher_version}`.
    pub launcher_version: &'a str,
    /// Window size, when the user pinned one. Turns on the `has_custom_resolution` feature.
    pub resolution: Option<(u32, u32)>,
}

/// The launch argument whose value is a secret, hidden by [`LaunchCommand::redacted`].
const ACCESS_TOKEN_FLAG: &str = "--accessToken";

/// Stand-in printed in place of a real access token.
const REDACTED: &str = "<redacted>";

/// A fully expanded command line, ready to spawn.
///
/// Its [`Debug`] prints the [`LaunchCommand::redacted`] form, so a traced or logged command
/// never carries a Microsoft access token.
#[derive(Clone, Serialize, PartialEq)]
pub struct LaunchCommand {
    /// The java binary to run.
    pub program: PathBuf,
    /// Every argument, in order: heap flags, JVM args, main class, then game args.
    ///
    /// Serialized in the [`LaunchCommand::redacted`] form, so a `--json` dry run cannot print
    /// an access token. Read the field directly to spawn the real command.
    #[serde(serialize_with = "serialize_redacted_args")]
    pub args: Vec<String>,
    /// Working directory, the instance's `.minecraft`.
    pub cwd: PathBuf,
    /// Extra environment variables. Empty today; the UI fills it in later.
    pub env: Vec<(String, String)>,
}

/// Replaces the value after every `--accessToken` with [`REDACTED`].
///
/// The offline placeholder `"0"` is kept, because it is not a secret and hiding it would make
/// an offline command line harder to read.
fn redact_args(args: &[String]) -> Vec<String> {
    let mut out = args.to_vec();
    for i in 0..out.len().saturating_sub(1) {
        if out[i] == ACCESS_TOKEN_FLAG && out[i + 1] != "0" {
            out[i + 1] = REDACTED.to_string();
        }
    }
    out
}

/// Serializes the argument list through [`redact_args`].
fn serialize_redacted_args<S>(args: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    redact_args(args).serialize(serializer)
}

impl LaunchCommand {
    /// A copy of this command with the value after `--accessToken` replaced.
    ///
    /// The offline placeholder `"0"` is kept, because it is not a secret and hiding it would
    /// make an offline command line harder to read.
    pub fn redacted(&self) -> LaunchCommand {
        LaunchCommand {
            program: self.program.clone(),
            args: redact_args(&self.args),
            cwd: self.cwd.clone(),
            env: self.env.clone(),
        }
    }
}

impl std::fmt::Debug for LaunchCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hidden = self.redacted();
        f.debug_struct("LaunchCommand")
            .field("program", &hidden.program)
            .field("args", &hidden.args)
            .field("cwd", &hidden.cwd)
            .field("env", &hidden.env)
            .finish()
    }
}

/// The separator java expects between classpath entries on this platform.
pub const CLASSPATH_SEPARATOR: &str = if cfg!(windows) { ";" } else { ":" };

/// Builds the command line for one launch.
///
/// Blocking: for a legacy version this reads the cached asset index and lays out the virtual
/// asset tree. Both are small file operations, so callers on a runtime wrap it in `block_on`
/// rather than paying for a separate async path.
pub fn build(inputs: &LaunchInputs<'_>, sink: Option<&EventSink>) -> Result<LaunchCommand, Error> {
    let plan = inputs.plan;
    let main_class = plan.main_class.clone().ok_or(Error::MissingMainClass)?;
    let game_dir = inputs.instance.game_dir();
    let rules = launch_rules(inputs);
    let assets = legacy_assets(inputs, &game_dir)?;

    let mut classpath: Vec<String> = plan
        .classpath
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    classpath.push(plan.client_jar.to_string_lossy().into_owned());

    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    let mut set = |k: &str, v: String| {
        vars.insert(k.to_string(), v);
    };
    set("auth_player_name", inputs.identity.name.clone());
    set("auth_uuid", inputs.identity.uuid_undashed.clone());
    set("auth_access_token", inputs.identity.access_token.clone());
    set("user_type", inputs.identity.user_type.clone());
    set("clientid", inputs.identity.client_id.clone());
    set("auth_xuid", inputs.identity.xuid.clone());
    // Pre-1.8 argument templates ask for it and never use the contents.
    set("user_properties", "{}".to_string());
    set("version_name", plan.resolved.id.clone());
    set(
        "version_type",
        plan.resolved
            .kind
            .clone()
            .unwrap_or_else(|| "release".to_string()),
    );
    set("game_directory", path_string(&game_dir));
    set("assets_root", path_string(&assets.root));
    set("assets_index_name", plan.asset_index_id.clone());
    set("natives_directory", path_string(&plan.natives_dir));
    set(
        "library_directory",
        path_string(&inputs.root.libraries_dir()),
    );
    set("classpath_separator", CLASSPATH_SEPARATOR.to_string());
    set("classpath", classpath.join(CLASSPATH_SEPARATOR));
    set("launcher_name", inputs.launcher_name.to_string());
    set("launcher_version", inputs.launcher_version.to_string());
    if let Some(game_assets) = &assets.game_assets {
        set("game_assets", path_string(game_assets));
    }
    if let Some((width, height)) = inputs.resolution {
        set("resolution_width", width.to_string());
        set("resolution_height", height.to_string());
    }
    let ctx = ArgContext { vars };

    let mut args = vec![
        format!("-Xms{}M", inputs.jvm.min_mib),
        format!("-Xmx{}M", inputs.jvm.max_mib),
    ];
    args.extend(inputs.jvm.extra_args.iter().cloned());
    match &plan.resolved.arguments {
        Some(a) => args.extend(expand_arguments(&a.jvm, &rules, &ctx, sink)),
        None => {
            let legacy: Vec<Argument> = default_legacy_jvm_args()
                .into_iter()
                .map(Argument::Plain)
                .collect();
            args.extend(expand_arguments(&legacy, &rules, &ctx, sink));
        }
    }
    if let Some(config) = &plan.log_config
        && let Some(client) = plan
            .resolved
            .logging
            .as_ref()
            .and_then(|l| l.client.as_ref())
    {
        args.push(client.argument.replace("${path}", &path_string(config)));
    }
    args.push(main_class);
    match (&plan.resolved.arguments, &plan.resolved.minecraft_arguments) {
        (Some(a), _) => args.extend(expand_arguments(&a.game, &rules, &ctx, sink)),
        (None, Some(legacy)) => args.extend(expand_legacy(legacy, &ctx)),
        (None, None) => {}
    }

    Ok(LaunchCommand {
        program: inputs.java.to_path_buf(),
        args,
        cwd: game_dir,
        env: Vec::new(),
    })
}

/// The rule context a launch evaluates argument rules against: this machine plus the launch
/// feature flags. The caller's context is left untouched.
fn launch_rules(inputs: &LaunchInputs<'_>) -> RuleContext {
    let mut rules = inputs.rules.clone();
    rules.features.insert("is_demo_user".to_string(), false);
    rules.features.insert(
        "has_custom_resolution".to_string(),
        inputs.resolution.is_some(),
    );
    for name in [
        "has_quick_plays_support",
        "is_quick_play_singleplayer",
        "is_quick_play_multiplayer",
        "is_quick_play_realms",
    ] {
        rules.features.insert(name.to_string(), false);
    }
    rules
}

/// Where the game reads assets from, after any legacy layout has been written.
struct AssetPaths {
    /// Value of `${assets_root}`.
    root: PathBuf,
    /// Value of `${game_assets}`, set only for a legacy version.
    game_assets: Option<PathBuf>,
}

/// Lays out the legacy asset tree when this version needs one, and picks `${assets_root}`.
///
/// A version is legacy when its `assets` id is `legacy` or `pre-1.6`, or when its cached asset
/// index sets `virtual` or `map_to_resources`.
fn legacy_assets(inputs: &LaunchInputs<'_>, game_dir: &Path) -> Result<AssetPaths, Error> {
    let assets_dir = inputs.root.assets_dir();
    let id = &inputs.plan.asset_index_id;
    let index_path = assets_dir.join("indexes").join(format!("{id}.json"));
    let index: Option<AssetIndex> = match std::fs::read(&index_path) {
        Ok(bytes) => Some(
            serde_json::from_slice(&bytes).map_err(|source| Error::Json {
                path: index_path.clone(),
                source,
            })?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(Error::Io {
                path: index_path,
                source,
            });
        }
    };
    let by_id = matches!(
        inputs.plan.assets_id.as_deref(),
        Some("legacy") | Some("pre-1.6")
    );
    let by_index = index
        .as_ref()
        .is_some_and(|i| i.virtual_ || i.map_to_resources);
    if !by_id && !by_index {
        return Ok(AssetPaths {
            root: assets_dir,
            game_assets: None,
        });
    }
    if let Some(index) = &index {
        materialize_legacy(index, inputs.root, id, game_dir)?;
    }
    let virtual_dir = assets_dir.join("virtual").join(id);
    Ok(AssetPaths {
        root: virtual_dir.clone(),
        game_assets: Some(virtual_dir),
    })
}

/// A path as a command-line string. Non-UTF-8 bytes are replaced, as they are on the java side.
fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instances::{Instances, model::Loader};
    use crate::mojang::install::plan_install;
    use crate::mojang::version::VersionJson;

    const V1_20_1: &str = include_str!("../../../../tests/fixtures/mojang/1.20.1.json");
    const V1_8_9: &str = include_str!("../../../../tests/fixtures/mojang/1.8.9.json");

    struct Fixture {
        _dir: tempfile::TempDir,
        root: Root,
        instance: Instance,
        plan: InstallPlan,
        identity: LaunchIdentity,
        rules: RuleContext,
    }

    fn linux_rules() -> RuleContext {
        RuleContext {
            os_name: "linux",
            os_version: "6.1.0".to_string(),
            arch: "x86_64",
            features: BTreeMap::new(),
        }
    }

    fn fixture(version_json: &str) -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        let resolved: VersionJson = serde_json::from_str(version_json).expect("fixture parses");
        let rules = linux_rules();
        let mut plan = plan_install(&resolved, &root, &rules, None).expect("plan");
        // A real install downloads the log4j config and records it; do the same here so the
        // command line under test is the one a launch after an install would produce.
        plan.log_config = crate::mojang::install::log_config_spec(&resolved, &root).map(|(_, p)| p);
        let instance = Instances::new(root.clone())
            .create(
                "Test Pack",
                &resolved.id,
                Loader::None,
                None,
                &BTreeMap::new(),
            )
            .expect("instance");
        Fixture {
            _dir: dir,
            root,
            instance,
            plan,
            identity: LaunchIdentity {
                name: "alice".to_string(),
                uuid_undashed: "b50ad385829d3141a2167e7d7539ba7f".to_string(),
                access_token: "0".to_string(),
                user_type: "legacy".to_string(),
                xuid: String::new(),
                client_id: String::new(),
            },
            rules,
        }
    }

    fn inputs<'a>(
        f: &'a Fixture,
        java: &'a Path,
        resolution: Option<(u32, u32)>,
    ) -> LaunchInputs<'a> {
        LaunchInputs {
            plan: &f.plan,
            instance: &f.instance,
            identity: &f.identity,
            java,
            root: &f.root,
            jvm: JvmSettings {
                min_mib: 512,
                max_mib: 4096,
                extra_args: vec!["-XX:+UseG1GC".to_string()],
            },
            rules: &f.rules,
            launcher_name: "grid-craft-launcher",
            launcher_version: "0.1.0",
            resolution,
        }
    }

    /// Index of `needle`, or a failed assertion naming the args.
    fn index_of(args: &[String], needle: &str) -> usize {
        args.iter()
            .position(|a| a == needle)
            .unwrap_or_else(|| panic!("{needle} missing from {args:?}"))
    }

    fn root_filter(f: &Fixture) -> String {
        regex::escape(&f.root.path().to_string_lossy())
    }

    #[test]
    fn modern_version_expands_every_placeholder() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");

        assert_eq!(cmd.program, java);
        assert_eq!(cmd.cwd, f.instance.game_dir());
        assert!(cmd.args.contains(&"-Xms512M".to_string()));
        assert!(cmd.args.contains(&"-Xmx4096M".to_string()));
        assert!(cmd.args.contains(&"-XX:+UseG1GC".to_string()));
        assert!(
            !cmd.args.iter().any(|a| a.contains("${")),
            "unexpanded placeholder in {:?}",
            cmd.args
        );

        let user = index_of(&cmd.args, "--username");
        assert_eq!(cmd.args[user + 1], "alice");
        let uuid = index_of(&cmd.args, "--uuid");
        assert_eq!(cmd.args[uuid + 1], "b50ad385829d3141a2167e7d7539ba7f");
        let token = index_of(&cmd.args, "--accessToken");
        assert_eq!(cmd.args[token + 1], "0");
        let user_type = index_of(&cmd.args, "--userType");
        assert_eq!(cmd.args[user_type + 1], "legacy");

        let natives = format!("-Djava.library.path={}", f.plan.natives_dir.display());
        assert!(cmd.args.contains(&natives), "{:?}", cmd.args);

        let cp = index_of(&cmd.args, "-cp");
        assert!(
            cmd.args[cp + 1].ends_with(&f.plan.client_jar.to_string_lossy().into_owned()),
            "client jar is not last on the classpath: {}",
            cmd.args[cp + 1]
        );
        let main = index_of(&cmd.args, "net.minecraft.client.main.Main");
        assert!(main > cp + 1, "main class must follow the jvm args");
        assert!(user > main, "game args must follow the main class");
    }

    #[test]
    fn modern_command_line_snapshot() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");
        insta::with_settings!({filters => vec![(root_filter(&f).as_str(), "<root>")]}, {
            insta::assert_json_snapshot!("launch_args_1_20_1", cmd.args);
        });
    }

    #[test]
    fn resolution_turns_on_the_custom_resolution_feature() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, Some((1280, 720))), None).expect("build");
        let width = index_of(&cmd.args, "--width");
        assert_eq!(cmd.args[width + 1], "1280");
        let height = index_of(&cmd.args, "--height");
        assert_eq!(cmd.args[height + 1], "720");
        assert!(!cmd.args.contains(&"--demo".to_string()));
    }

    #[test]
    fn legacy_version_uses_minecraft_arguments_and_default_jvm_args() {
        let f = fixture(V1_8_9);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");

        let natives = format!("-Djava.library.path={}", f.plan.natives_dir.display());
        assert!(cmd.args.contains(&natives), "{:?}", cmd.args);
        let cp = index_of(&cmd.args, "-cp");
        let main = index_of(&cmd.args, "net.minecraft.client.main.Main");
        // -cp, the classpath, the logging argument, then the main class.
        assert_eq!(main, cp + 3, "main class follows -cp <classpath>");
        let asset_index = index_of(&cmd.args, "--assetIndex");
        assert_eq!(cmd.args[asset_index + 1], "1.8");
        assert!(!cmd.args.iter().any(|a| a == "--tweakClass"));
        assert!(
            !cmd.args.iter().any(|a| a.contains("${")),
            "unexpanded placeholder in {:?}",
            cmd.args
        );
    }

    #[test]
    fn legacy_command_line_snapshot() {
        let f = fixture(V1_8_9);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");
        insta::with_settings!({filters => vec![(root_filter(&f).as_str(), "<root>")]}, {
            insta::assert_json_snapshot!("launch_args_1_8_9", cmd.args);
        });
    }

    #[test]
    fn a_virtual_asset_index_points_assets_root_at_the_virtual_tree() {
        let f = fixture(V1_8_9);
        let indexes = f.root.assets_dir().join("indexes");
        std::fs::create_dir_all(&indexes).expect("indexes dir");
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let object = crate::mojang::assets::object_path(&f.root, hash);
        std::fs::create_dir_all(object.parent().expect("parent")).expect("object dir");
        std::fs::write(&object, b"hi").expect("object");
        std::fs::write(
            indexes.join("1.8.json"),
            serde_json::json!({
                "virtual": true,
                "objects": { "lang/en_GB.lang": { "hash": hash, "size": 2 } },
            })
            .to_string(),
        )
        .expect("index");

        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");
        let virtual_dir = f.root.assets_dir().join("virtual/1.8");
        let assets_dir = index_of(&cmd.args, "--assetsDir");
        assert_eq!(
            cmd.args[assets_dir + 1],
            virtual_dir.to_string_lossy().into_owned()
        );
        assert!(virtual_dir.join("lang/en_GB.lang").is_file());
    }

    #[test]
    fn the_logging_argument_sits_between_the_jvm_args_and_the_main_class() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");
        let config = f
            .plan
            .log_config
            .as_ref()
            .expect("fixture has a log config");
        let expected = format!("-Dlog4j.configurationFile={}", config.display());
        let arg = index_of(&cmd.args, &expected);
        let main = index_of(&cmd.args, "net.minecraft.client.main.Main");
        let cp = index_of(&cmd.args, "-cp");
        assert!(arg > cp, "logging arg must follow the expanded jvm args");
        assert!(arg < main, "logging arg must precede the main class");
    }

    #[test]
    fn no_log_config_means_no_logging_argument() {
        let mut f = fixture(V1_20_1);
        f.plan.log_config = None;
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");
        assert!(
            !cmd.args.iter().any(|a| a.starts_with("-Dlog4j")),
            "{:?}",
            cmd.args
        );
    }

    fn command_with_token(token: &str) -> LaunchCommand {
        LaunchCommand {
            program: PathBuf::from("/usr/bin/java"),
            args: vec![
                "--username".to_string(),
                "alice".to_string(),
                "--accessToken".to_string(),
                token.to_string(),
            ],
            cwd: PathBuf::from("/tmp"),
            env: Vec::new(),
        }
    }

    #[test]
    fn redacted_hides_a_real_access_token() {
        let cmd = command_with_token("ey.super.secret");
        let hidden = cmd.redacted();
        assert_eq!(hidden.args[3], "<redacted>");
        assert_eq!(hidden.args[1], "alice");
        assert_eq!(cmd.args[3], "ey.super.secret", "the original is untouched");
    }

    #[test]
    fn redacted_keeps_the_offline_placeholder_token() {
        let hidden = command_with_token("0").redacted();
        assert_eq!(hidden.args[3], "0");
    }

    #[test]
    fn serializing_never_emits_the_access_token() {
        let json = serde_json::to_string(&command_with_token("ey.super.secret"))
            .expect("command serializes");
        assert!(!json.contains("ey.super.secret"), "{json}");
        assert!(json.contains("<redacted>"), "{json}");
    }

    #[test]
    fn serializing_keeps_the_offline_placeholder_token() {
        let json = serde_json::to_string(&command_with_token("0")).expect("command serializes");
        assert!(json.contains(r#""0""#), "{json}");
        assert!(!json.contains("<redacted>"), "{json}");
    }

    #[test]
    fn debug_never_prints_the_access_token() {
        let printed = format!("{:?}", command_with_token("ey.super.secret"));
        assert!(!printed.contains("ey.super.secret"), "{printed}");
        assert!(printed.contains("<redacted>"), "{printed}");
    }

    #[test]
    fn a_version_without_a_main_class_is_an_error() {
        let mut f = fixture(V1_20_1);
        f.plan.main_class = None;
        let java = PathBuf::from("/usr/bin/java");
        let err = build(&inputs(&f, &java, None), None).expect_err("no main class");
        assert!(matches!(err, Error::MissingMainClass));
    }
}
