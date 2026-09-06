//! Tests for installer jar reading, the data map, and argument substitution.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::loaders::test_support::{write_jar_with_main, write_zip};
use crate::mojang::RuleContext;

const FORGE_PROFILE: &str =
    include_str!("../../../../tests/fixtures/forge/install_profile_1.20.1-47.4.10.json");
const NEOFORGE_PROFILE: &str =
    include_str!("../../../../tests/fixtures/neoforge/install_profile_21.1.250.json");

/// Counts that describe a parsed profile without pinning its whole content.
fn shape(p: &InstallProfile) -> serde_json::Value {
    serde_json::json!({
        "spec": p.spec,
        "profile": p.profile,
        "version": p.version,
        "minecraft": p.minecraft,
        "json": p.json,
        "data_keys": p.data.len(),
        "processors": p.processors.len(),
        "libraries": p.libraries.len(),
        "client_processors": p.processors.iter().filter(|x| runs_on_client(&x.sides)).count(),
        "legacy": p.is_legacy(),
    })
}

/// A root plus a temp dir that keeps it alive.
struct Layout {
    _dir: TempDir,
    root: Root,
}

impl Layout {
    fn new() -> Layout {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = Root::from_path(dir.path());
        root.ensure_layout().expect("layout");
        Layout { _dir: dir, root }
    }
}

/// Builds a synthetic installer jar with a data file and a `maven/` artifact.
fn synthetic_installer(root: &Root) -> InstallerJar {
    let path = root.installers_dir().join("forge-test-installer.jar");
    write_zip(
        &path,
        &[
            ("data/client.lzma", b"binpatch-bytes"),
            ("maven/net/x/y/1/y-1.jar", b"universal-jar"),
            ("version.json", b"{}"),
        ],
    );
    InstallerJar::open(path).expect("open installer")
}

#[test]
fn parses_the_forge_install_profile() {
    let profile: InstallProfile = serde_json::from_str(FORGE_PROFILE).expect("parse forge");
    assert_eq!(profile.json.as_deref(), Some("/version.json"));
    insta::assert_json_snapshot!("forge_install_profile", shape(&profile));
}

#[test]
fn parses_the_neoforge_install_profile() {
    let profile: InstallProfile = serde_json::from_str(NEOFORGE_PROFILE).expect("parse neoforge");
    assert_eq!(profile.json.as_deref(), Some("/version.json"));
    insta::assert_json_snapshot!("neoforge_install_profile", shape(&profile));
}

#[test]
fn a_profile_with_version_info_and_no_spec_is_legacy() {
    let profile: InstallProfile =
        serde_json::from_str(r#"{"versionInfo":{"id":"1.12.2-forge"}}"#).expect("parse legacy");
    assert!(profile.is_legacy());
    let modern: InstallProfile = serde_json::from_str(r#"{"spec":1}"#).expect("parse modern");
    assert!(!modern.is_legacy());
}

#[test]
fn sides_decides_whether_a_processor_runs_on_the_client() {
    assert!(runs_on_client(&[]));
    assert!(runs_on_client(&["client".to_string()]));
    assert!(runs_on_client(&[
        "client".to_string(),
        "server".to_string()
    ]));
    assert!(!runs_on_client(&["server".to_string()]));
}

#[test]
fn build_data_map_resolves_coordinates_literals_and_jar_files() {
    let layout = Layout::new();
    let temp = layout.root.cache_dir().join("installer-temp");
    let jar = synthetic_installer(&layout.root);
    let profile: InstallProfile = serde_json::from_str(
        r#"{
            "spec": 1,
            "minecraft": "1.20.1",
            "data": {
                "UNIVERSAL": { "client": "[net.x:y:1]", "server": "[net.x:y:1]" },
                "MC_SLIM_SHA": { "client": "'abc123'", "server": "'def456'" },
                "BINPATCH": { "client": "/data/client.lzma", "server": "/data/server.lzma" }
            }
        }"#,
    )
    .expect("parse synthetic profile");

    let client_jar = layout.root.versions_dir().join("1.20.1/1.20.1.jar");
    let data = build_data_map(&profile, &jar, &layout.root, "1.20.1", &client_jar, &temp)
        .expect("build data map");

    let libs = layout.root.libraries_dir();
    assert_eq!(
        data.get("UNIVERSAL"),
        Some(
            libs.join("net/x/y/1/y-1.jar")
                .display()
                .to_string()
                .as_str()
        )
    );
    assert_eq!(data.get("MC_SLIM_SHA"), Some("abc123"));
    let binpatch = PathBuf::from(data.get("BINPATCH").expect("binpatch"));
    assert_eq!(binpatch, temp.join("data/client.lzma"));
    assert_eq!(
        std::fs::read(&binpatch).expect("read extracted"),
        b"binpatch-bytes"
    );

    assert_eq!(data.get("SIDE"), Some("client"));
    assert_eq!(data.get("MINECRAFT_VERSION"), Some("1.20.1"));
    assert_eq!(
        data.get("MINECRAFT_JAR"),
        Some(client_jar.display().to_string().as_str())
    );
    assert_eq!(
        data.get("INSTALLER"),
        Some(jar.path.display().to_string().as_str())
    );
    assert_eq!(
        data.get("ROOT"),
        Some(layout.root.path().display().to_string().as_str())
    );
    assert_eq!(
        data.get("LIBRARY_DIR"),
        Some(libs.display().to_string().as_str())
    );
}

#[test]
fn substitute_replaces_keys_coordinates_and_rejects_unknown_keys() {
    let layout = Layout::new();
    let libs = layout.root.libraries_dir();
    let data = DataMap(BTreeMap::from([
        ("MINECRAFT_JAR".to_string(), "/cache/1.20.1.jar".to_string()),
        ("ROOT".to_string(), "/root".to_string()),
    ]));

    assert_eq!(
        substitute("{MINECRAFT_JAR}", &data, &layout.root).expect("key"),
        "/cache/1.20.1.jar"
    );
    assert_eq!(
        substitute("{ROOT}/run.sh", &data, &layout.root).expect("embedded key"),
        "/root/run.sh"
    );
    assert_eq!(
        substitute("--task", &data, &layout.root).expect("plain"),
        "--task"
    );
    assert_eq!(
        substitute("[net.x:y:1]", &data, &layout.root).expect("coordinate"),
        libs.join("net/x/y/1/y-1.jar").display().to_string()
    );
    assert_eq!(
        substitute("[net.x:y:1:extra@txt]", &data, &layout.root).expect("classified"),
        libs.join("net/x/y/1/y-1-extra.txt").display().to_string()
    );

    let err = substitute("{NOPE}", &data, &layout.root).expect_err("unknown key");
    assert!(
        matches!(err, Error::UnknownDataKey(ref k) if k == "NOPE"),
        "{err:?}"
    );
}

#[test]
fn extract_prefix_lands_maven_artifacts_under_the_libraries_dir() {
    let layout = Layout::new();
    let jar = synthetic_installer(&layout.root);
    let libs = layout.root.libraries_dir();

    let written = jar.extract_prefix("maven/", &libs).expect("extract maven");

    assert_eq!(written, vec![libs.join("net/x/y/1/y-1.jar")]);
    assert_eq!(
        std::fs::read(libs.join("net/x/y/1/y-1.jar")).expect("read"),
        b"universal-jar"
    );
}

#[test]
fn extract_prefix_rejects_a_traversing_entry() {
    let layout = Layout::new();
    let path = layout.root.installers_dir().join("evil-installer.jar");
    write_zip(&path, &[("maven/../evil.jar", b"pwned")]);
    let jar = InstallerJar::open(path).expect("open");

    let err = jar
        .extract_prefix("maven/", &layout.root.libraries_dir())
        .expect_err("traversal must be refused");

    assert!(matches!(err, Error::UnsafePath(_)), "{err:?}");
    assert!(!layout.root.cache_dir().join("evil.jar").exists());
}

#[test]
fn read_entry_and_extract_file_use_the_names_the_profile_gives() {
    let layout = Layout::new();
    let jar = synthetic_installer(&layout.root);
    let temp = layout.root.cache_dir().join("t");

    assert_eq!(jar.read_entry("version.json").expect("read"), b"{}");
    let dest = jar
        .extract_file("/data/client.lzma", &temp)
        .expect("extract");
    assert_eq!(dest, temp.join("data/client.lzma"));
    assert!(jar.read_entry("nope.txt").is_err());
    assert!(matches!(
        jar.extract_file("/../evil", &temp),
        Err(Error::UnsafePath(_))
    ));
}

#[test]
fn manifest_main_class_reads_the_processor_jar_manifest() {
    let layout = Layout::new();
    let with_main = layout.root.libraries_dir().join("tool.jar");
    write_jar_with_main(&with_main, "net.test.Tool");
    let without = layout.root.libraries_dir().join("plain.jar");
    write_zip(&without, &[("a.txt", b"x")]);

    let jar = InstallerJar::open(with_main).expect("open");
    assert_eq!(
        jar.manifest_main_class().expect("manifest").as_deref(),
        Some("net.test.Tool")
    );
    let plain = InstallerJar::open(without).expect("open");
    assert_eq!(plain.manifest_main_class().expect("manifest"), None);
}

#[test]
fn manifest_main_class_joins_a_wrapped_line() {
    let layout = Layout::new();
    let path = layout.root.libraries_dir().join("wrapped.jar");
    write_zip(
        &path,
        &[(
            "META-INF/MANIFEST.MF",
            b"Manifest-Version: 1.0\r\nMain-Class: net.test.VeryLongNamed\r\n Processor\r\nBuilt-By: x\r\n",
        )],
    );
    let jar = InstallerJar::open(path).expect("open");
    assert_eq!(
        jar.manifest_main_class().expect("manifest").as_deref(),
        Some("net.test.VeryLongNamedProcessor")
    );
}

#[test]
fn library_specs_skip_rule_excluded_entries_and_target_the_libraries_dir() {
    let layout = Layout::new();
    let profile: InstallProfile = serde_json::from_str(FORGE_PROFILE).expect("parse forge");
    let specs =
        library_specs(&profile.libraries, &layout.root, &RuleContext::current()).expect("specs");

    assert_eq!(specs.len(), profile.libraries.len());
    let libs = layout.root.libraries_dir();
    assert!(specs.iter().all(|s| s.dest.starts_with(&libs)));
    let lzma = specs
        .iter()
        .find(|s| s.label.contains("lzma-java"))
        .expect("lzma library");
    assert_eq!(
        lzma.dest,
        libs.join("com/github/jponge/lzma-java/1.3/lzma-java-1.3.jar")
    );
    assert_eq!(
        lzma.sha1.as_deref(),
        Some("a25db9d4d385ccda4825ae1b47a7a61d86e595af")
    );
}
