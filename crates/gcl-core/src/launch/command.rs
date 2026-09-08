//! Builds the java command line for one launch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::Error;
use crate::auth::LaunchIdentity;
use crate::events::EventSink;
use crate::instances::Instance;
use crate::instances::model::GcPreset;
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
    /// Extra JVM flags, inserted after the GC preset flags and before the version's own JVM args.
    pub extra_args: Vec<String>,
    /// Garbage collector preset saved on the instance.
    pub gc: GcPreset,
    /// Major version of the java binary this command runs, as the GC probe read it.
    ///
    /// Some presets expand to different flags per major, so this is the probed major rather
    /// than the version's `javaVersion`: a manually configured `java_path` carries no major of
    /// its own. Only [`GcPreset::Default`] may leave it unset: [`build`] refuses any other
    /// preset with a major below 8, since that means nothing probed the java binary.
    pub gc_major: u32,
}

/// JVM flag names that choose a collector, or choose how one runs.
///
/// Compared by name, not by whole argument, so `-XX:+UseG1GC` and `-XX:-UseG1GC` both count:
/// turning a collector off is as much a choice as turning one on. The list carries the
/// collectors older JVMs still accept (`UseParallelOldGC`, `UseConcMarkSweepGC`) and
/// `UseEpsilonGC`, which a preset would fight just as hard.
const COLLECTOR_FLAG_NAMES: &[&str] = &[
    "UseSerialGC",
    "UseParallelGC",
    "UseParallelOldGC",
    "UseConcMarkSweepGC",
    "UseG1GC",
    "UseZGC",
    "UseShenandoahGC",
    "UseEpsilonGC",
    "ZGenerational",
];

/// The collector flag name one JVM argument switches, if it switches one.
///
/// Reads `-XX:+Name` and `-XX:-Name` only: anything else, `-XX:MaxGCPauseMillis=50` or
/// `-XX:+UseG1GCFoo`, names no collector.
fn collector_flag_name(arg: &str) -> Option<&str> {
    let name = arg
        .strip_prefix("-XX:+")
        .or_else(|| arg.strip_prefix("-XX:-"))?;
    COLLECTOR_FLAG_NAMES.contains(&name).then_some(name)
}

/// The first extra JVM argument that picks a collector itself, when a preset is also set.
///
/// `None` for [`GcPreset::Default`]: with no preset chosen, a hand-written collector flag is
/// exactly what extra arguments are for.
pub fn gc_conflict(gc: GcPreset, extra_args: &[String]) -> Option<&str> {
    if gc == GcPreset::Default {
        return None;
    }
    extra_args
        .iter()
        .find(|a| collector_flag_name(a).is_some())
        .map(String::as_str)
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

/// Replaces every access token with [`REDACTED`], in both the `--accessToken <value>` and the
/// `--accessToken=<value>` form.
///
/// The offline placeholder `"0"` is kept, because it is not a secret and hiding it would make
/// an offline command line harder to read.
fn redact_args(args: &[String]) -> Vec<String> {
    let inline_prefix = format!("{ACCESS_TOKEN_FLAG}=");
    let mut out = args.to_vec();
    let mut i = 0;
    while i < out.len() {
        if let Some(value) = out[i].strip_prefix(&inline_prefix).map(str::to_string) {
            if value != "0" {
                out[i] = format!("{inline_prefix}{REDACTED}");
            }
        } else if out[i] == ACCESS_TOKEN_FLAG && out.get(i + 1).is_some_and(|v| v != "0") {
            out[i + 1] = REDACTED.to_string();
            i += 1;
        }
        i += 1;
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
    dedupe_keeping_order(&mut classpath);

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
    if let Some(flag) = gc_conflict(inputs.jvm.gc, &inputs.jvm.extra_args) {
        return Err(Error::GcConflict {
            preset: inputs.jvm.gc,
            extra_flag: flag.to_string(),
        });
    }
    // Every preset's flags depend on the java major, and no java that can run the game is
    // below 8, so a major under 8 means the caller never probed the binary.
    if inputs.jvm.gc != GcPreset::Default && inputs.jvm.gc_major < 8 {
        return Err(Error::MissingGcMajor {
            preset: inputs.jvm.gc,
        });
    }
    args.extend(inputs.jvm.gc.flags(inputs.jvm.gc_major));
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
    dedupe_path_list_args(&mut args);
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

/// Flags whose next argument is a separator-joined list of jar paths.
const PATH_LIST_FLAGS: [&str; 5] = ["-cp", "-classpath", "--class-path", "-p", "--module-path"];

/// Drops every repeated entry, keeping the first occurrence and the order of the rest.
///
/// An empty entry is left alone: `a::b` names the current directory between two jars, and
/// dropping it would change what java resolves.
fn dedupe_keeping_order(entries: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| e.is_empty() || seen.insert(e.clone()));
}

/// Drops repeated jars from every classpath and module path on the command line.
///
/// The library merge already keeps one copy of a repeated coordinate; this is the guard for a
/// path list a version JSON writes out by hand, such as NeoForge's literal `-p` value. Java's
/// `BootstrapLauncher` throws `IllegalStateException: Duplicate key` on the first repeat.
fn dedupe_path_list_args(args: &mut [String]) {
    for i in 0..args.len() {
        if !PATH_LIST_FLAGS.contains(&args[i].as_str()) {
            continue;
        }
        let Some(value) = args.get(i + 1) else {
            continue;
        };
        let mut entries: Vec<String> = value
            .split(CLASSPATH_SEPARATOR)
            .map(str::to_string)
            .collect();
        let before = entries.len();
        dedupe_keeping_order(&mut entries);
        if entries.len() != before {
            args[i + 1] = entries.join(CLASSPATH_SEPARATOR);
        }
    }
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
    const V1_21_1: &str = include_str!("../../../../tests/fixtures/mojang/1.21.1.json");
    const NEOFORGE_PROFILE: &str =
        include_str!("../../../../tests/fixtures/neoforge/version_21.1.250.json");
    const FORGE_PROFILE: &str =
        include_str!("../../../../tests/fixtures/forge/version_1.20.1-47.4.10.json");

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
        let resolved: VersionJson = serde_json::from_str(version_json).expect("fixture parses");
        fixture_from(resolved)
    }

    fn fixture_from(resolved: VersionJson) -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
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
                gc: GcPreset::Default,
                gc_major: 0,
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

    /// The `-cp` value, split back into entries.
    fn classpath_entries(cmd: &LaunchCommand) -> Vec<String> {
        let cp = index_of(&cmd.args, "-cp");
        cmd.args[cp + 1]
            .split(CLASSPATH_SEPARATOR)
            .map(str::to_string)
            .collect()
    }

    /// Fails when any path list on the command line names the same jar twice.
    ///
    /// `BootstrapLauncher` (Forge and NeoForge) throws `Duplicate key` on one, so this is a
    /// launch failure, not a cosmetic problem.
    fn assert_no_duplicate_paths(cmd: &LaunchCommand) {
        let mut lists: Vec<Vec<String>> = vec![classpath_entries(cmd)];
        for (i, arg) in cmd.args.iter().enumerate() {
            if arg == "-p" || arg == "--module-path" {
                let value = cmd.args.get(i + 1).expect("module path value");
                lists.push(
                    value
                        .split(CLASSPATH_SEPARATOR)
                        .map(str::to_string)
                        .collect(),
                );
            }
        }
        for list in lists {
            let mut seen = std::collections::BTreeSet::new();
            for entry in &list {
                assert!(
                    seen.insert(entry.clone()),
                    "{entry} appears twice in {list:?}"
                );
            }
        }
    }

    /// The vanilla library object with this coordinate, straight out of the fixture.
    fn vanilla_library(name: &str) -> serde_json::Value {
        let v: serde_json::Value = serde_json::from_str(V1_20_1).expect("fixture parses");
        v["libraries"]
            .as_array()
            .expect("libraries")
            .iter()
            .find(|l| l["name"] == name)
            .unwrap_or_else(|| panic!("{name} is not in the fixture"))
            .clone()
    }

    /// A loader profile over 1.20.1 that lists `libraries`.
    fn loader_profile(libraries: serde_json::Value) -> VersionJson {
        serde_json::from_value(serde_json::json!({
            "id": "neoforge-test",
            "inheritsFrom": "1.20.1",
            "type": "release",
            "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries": libraries,
        }))
        .expect("profile parses")
    }

    #[test]
    fn a_loader_repeating_a_vanilla_library_gets_one_classpath_entry() {
        let vanilla: VersionJson = serde_json::from_str(V1_20_1).expect("fixture parses");
        let gson = vanilla_library("com.google.code.gson:gson:2.10");
        let profile = loader_profile(serde_json::json!([
            gson,
            {
                "name": "cpw.mods:bootstraplauncher:2.0.2",
                "url": "https://maven.neoforged.net/releases/",
            },
        ]));
        let resolved = crate::mojang::merge(vanilla, profile, true);
        let f = fixture_from(resolved);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");

        let entries = classpath_entries(&cmd);
        let gson_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.contains("gson-2.10.jar"))
            .collect();
        assert_eq!(gson_entries.len(), 1, "gson twice in {entries:?}");
        assert_no_duplicate_paths(&cmd);
    }

    #[test]
    fn a_neoforge_profile_over_vanilla_has_no_duplicate_classpath_entries() {
        // NeoForge 21.1.250 targets 1.21.1, and both list gson 2.10.1: the real collision.
        let vanilla: VersionJson = serde_json::from_str(V1_21_1).expect("fixture parses");
        let profile: VersionJson =
            serde_json::from_str(NEOFORGE_PROFILE).expect("neoforge fixture parses");
        let resolved = crate::mojang::merge(vanilla, profile, true);
        let f = fixture_from(resolved);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");
        assert_no_duplicate_paths(&cmd);
    }

    #[test]
    fn a_forge_profile_over_vanilla_has_no_duplicate_classpath_entries() {
        let vanilla: VersionJson = serde_json::from_str(V1_20_1).expect("fixture parses");
        let profile: VersionJson =
            serde_json::from_str(FORGE_PROFILE).expect("forge fixture parses");
        let resolved = crate::mojang::merge(vanilla, profile, true);
        let f = fixture_from(resolved);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(&inputs(&f, &java, None), None).expect("build");
        assert_no_duplicate_paths(&cmd);
    }

    #[test]
    fn a_repeated_jar_on_a_literal_module_path_is_dropped() {
        let sep = CLASSPATH_SEPARATOR;
        let mut args = vec![
            "-p".to_string(),
            format!("/libs/a.jar{sep}/libs/b.jar{sep}/libs/a.jar"),
            "--add-modules".to_string(),
            "ALL-MODULE-PATH".to_string(),
        ];
        dedupe_path_list_args(&mut args);
        assert_eq!(args[1], format!("/libs/a.jar{sep}/libs/b.jar"));
        assert_eq!(args[3], "ALL-MODULE-PATH", "other args are untouched");
    }

    #[test]
    fn an_empty_path_list_entry_is_left_where_it_is() {
        // `a::b` on a classpath means the current directory in the middle. Collapsing it
        // would shift what java resolves, so only real repeats are dropped.
        let sep = CLASSPATH_SEPARATOR;
        let mut args = vec![
            "-cp".to_string(),
            format!("/libs/a.jar{sep}{sep}/libs/b.jar{sep}{sep}/libs/a.jar"),
        ];
        dedupe_path_list_args(&mut args);
        assert_eq!(
            args[1],
            format!("/libs/a.jar{sep}{sep}/libs/b.jar{sep}"),
            "both empties stay, the repeated jar goes"
        );
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
    fn redacted_hides_an_equals_form_access_token() {
        let args = vec![
            "--accessToken=ey.super.secret".to_string(),
            "--accessToken=0".to_string(),
            "--username=alice".to_string(),
        ];
        let hidden = redact_args(&args);
        assert_eq!(hidden[0], "--accessToken=<redacted>");
        assert_eq!(hidden[1], "--accessToken=0", "the placeholder is kept");
        assert_eq!(hidden[2], "--username=alice");
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

    /// Inputs whose JVM settings carry a GC preset and a chosen extra-args list.
    fn gc_inputs<'a>(
        f: &'a Fixture,
        java: &'a Path,
        gc: GcPreset,
        gc_major: u32,
        extra_args: Vec<String>,
    ) -> LaunchInputs<'a> {
        let mut inputs = inputs(f, java, None);
        inputs.jvm = JvmSettings {
            min_mib: 512,
            max_mib: 4096,
            extra_args,
            gc,
            gc_major,
        };
        inputs
    }

    #[test]
    fn gc_preset_flags_sit_between_the_heap_flags_and_the_extra_args() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(
            &gc_inputs(
                &f,
                &java,
                GcPreset::G1,
                21,
                vec!["-Dtest.flag=1".to_string()],
            ),
            None,
        )
        .expect("build");

        let xmx = index_of(&cmd.args, "-Xmx4096M");
        let g1 = index_of(&cmd.args, "-XX:+UseG1GC");
        let extra = index_of(&cmd.args, "-Dtest.flag=1");
        assert_eq!(g1, xmx + 1, "{:?}", cmd.args);
        assert_eq!(extra, g1 + 1, "{:?}", cmd.args);
    }

    #[test]
    fn a_default_preset_adds_no_flag() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        let cmd = build(
            &gc_inputs(&f, &java, GcPreset::Default, 21, Vec::new()),
            None,
        )
        .expect("build");
        // No argument anywhere on the line picks a collector, not merely the one after -Xmx.
        assert!(
            !cmd.args.iter().any(|a| collector_flag_name(a).is_some()),
            "{:?}",
            cmd.args
        );
    }

    #[test]
    fn zgc_carries_the_generational_switch_only_below_23() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");

        let at_22 = build(&gc_inputs(&f, &java, GcPreset::Zgc, 22, Vec::new()), None).expect("22");
        assert!(at_22.args.contains(&"-XX:+UseZGC".to_string()));
        assert!(at_22.args.contains(&"-XX:-ZGenerational".to_string()));

        let at_23 = build(&gc_inputs(&f, &java, GcPreset::Zgc, 23, Vec::new()), None).expect("23");
        assert!(at_23.args.contains(&"-XX:+UseZGC".to_string()));
        assert!(
            !at_23.args.iter().any(|a| a.contains("ZGenerational")),
            "{:?}",
            at_23.args
        );
    }

    #[test]
    fn a_preset_and_a_hand_written_collector_flag_is_a_launch_error() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        for flag in conflicting_flags() {
            let flag = &flag;
            let err = build(
                &gc_inputs(&f, &java, GcPreset::G1, 21, vec![(*flag).to_string()]),
                None,
            )
            .expect_err(flag);
            match err {
                Error::GcConflict { preset, extra_flag } => {
                    assert_eq!(preset, GcPreset::G1);
                    assert_eq!(extra_flag, *flag);
                }
                other => panic!("expected GcConflict for {flag}, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_hand_written_collector_flag_with_no_preset_still_launches() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        for flag in conflicting_flags() {
            let flag = &flag;
            let cmd = build(
                &gc_inputs(&f, &java, GcPreset::Default, 21, vec![(*flag).to_string()]),
                None,
            )
            .expect(flag);
            assert!(cmd.args.contains(&(*flag).to_string()), "{:?}", cmd.args);
        }
    }

    #[test]
    fn gc_conflict_reads_the_first_offending_argument_only() {
        let args = [
            "-Dfoo=1".to_string(),
            "-XX:+UseZGC".to_string(),
            "-XX:+UseG1GC".to_string(),
        ];
        assert_eq!(gc_conflict(GcPreset::Zgc, &args), Some("-XX:+UseZGC"));
        assert_eq!(gc_conflict(GcPreset::Default, &args), None);
        assert_eq!(gc_conflict(GcPreset::Zgc, &[]), None);
        assert_eq!(
            gc_conflict(GcPreset::Zgc, &["-XX:+UseG1GCFoo".to_string()]),
            None
        );
    }

    /// Every flag a user could write by hand that picks a collector, in both switch forms.
    fn conflicting_flags() -> Vec<String> {
        COLLECTOR_FLAG_NAMES
            .iter()
            .flat_map(|name| [format!("-XX:+{name}"), format!("-XX:-{name}")])
            .collect()
    }

    #[test]
    fn a_conflict_is_read_from_the_flag_name_in_either_switch_form() {
        for name in COLLECTOR_FLAG_NAMES {
            for arg in [format!("-XX:+{name}"), format!("-XX:-{name}")] {
                assert_eq!(
                    gc_conflict(GcPreset::G1, std::slice::from_ref(&arg)),
                    Some(arg.as_str()),
                    "{arg}"
                );
            }
        }
    }

    #[test]
    fn the_disable_form_of_a_collector_flag_conflicts_too() {
        let args = ["-XX:-UseG1GC".to_string()];
        assert_eq!(gc_conflict(GcPreset::Zgc, &args), Some("-XX:-UseG1GC"));
    }

    #[test]
    fn an_argument_that_is_not_a_collector_switch_is_no_conflict() {
        for arg in [
            "-XX:+UseG1GCFoo",
            "-XX:+UseStringDeduplication",
            "-Xmx4096M",
            "-XX:MaxGCPauseMillis=50",
            "-XX:+UseCompressedOops",
            "UseG1GC",
        ] {
            assert_eq!(
                gc_conflict(GcPreset::Zgc, &[arg.to_string()]),
                None,
                "{arg}"
            );
        }
    }

    #[test]
    fn the_old_collector_names_conflict_as_well() {
        for arg in [
            "-XX:+UseParallelOldGC",
            "-XX:+UseConcMarkSweepGC",
            "-XX:+UseEpsilonGC",
        ] {
            assert_eq!(
                gc_conflict(GcPreset::G1, &[arg.to_string()]),
                Some(arg),
                "{arg}"
            );
        }
    }

    #[test]
    fn a_preset_without_a_probed_java_major_is_an_error() {
        let f = fixture(V1_20_1);
        let java = PathBuf::from("/usr/bin/java");
        for major in [0, 7] {
            let err = build(&gc_inputs(&f, &java, GcPreset::G1, major, Vec::new()), None)
                .expect_err("no probed major");
            match err {
                Error::MissingGcMajor { preset } => assert_eq!(preset, GcPreset::G1),
                other => panic!("wrong error: {other:?}"),
            }
        }
        // The default preset needs no major, so it still builds.
        build(
            &gc_inputs(&f, &java, GcPreset::Default, 0, Vec::new()),
            None,
        )
        .expect("default");
    }
}
