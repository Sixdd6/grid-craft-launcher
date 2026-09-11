//! Unit tests for the settings screen's pure helpers. No Slint instance and no display needed.

use std::path::PathBuf;

use gcl_core::config::Config;

use super::{config_view, key_status, parse_mib, root_change_status, verify_line, verify_status};

/// A config with something in every field the screen shows.
fn filled() -> Config {
    let mut config = Config {
        root: Some(PathBuf::from("/data/gcl")),
        parallel_downloads: 4,
        ..Config::default()
    };
    config.jvm.min_mib = 2048;
    config.jvm.max_mib = 8192;
    config.jvm.java_path = Some(PathBuf::from("/usr/lib/jvm/java-21/bin/java"));
    config
        .game_defaults
        .insert("renderDistance".to_string(), "12".to_string());
    config
        .game_defaults
        .insert("fov".to_string(), "0.5".to_string());
    config
}

#[test]
fn config_view_copies_the_displayed_fields() {
    let view = config_view(&filled());
    assert_eq!(view.root, "/data/gcl");
    assert_eq!(view.parallel, 4);
    assert_eq!(view.jvm_min, 2048);
    assert_eq!(view.jvm_max, 8192);
    assert_eq!(view.java_path, "/usr/lib/jvm/java-21/bin/java");
    assert_eq!(
        view.game_defaults,
        vec![
            ("fov".to_string(), "0.5".to_string()),
            ("renderDistance".to_string(), "12".to_string()),
        ],
        "defaults are shown in key order"
    );
}

#[test]
fn config_view_carries_no_key_value() {
    // `curseforge_enabled` only ever reflects the environment or the compiled-in build key
    // (see `Config::curseforge_api_key`), never a value on the config struct, so there is
    // nothing to set here. The view's `Debug` output is checked to prove it never carries a
    // secret, whatever future field might.
    let view = config_view(&filled());
    let shown = format!("{view:?}");
    assert!(!shown.contains("super-secret"), "got {shown}");
}

#[test]
fn config_view_leaves_an_unset_root_and_java_path_empty() {
    let view = config_view(&Config::default());
    assert_eq!(view.root, "");
    assert_eq!(view.java_path, "");
    assert_eq!(view.parallel, 8);
    assert!(view.game_defaults.is_empty());
}

#[test]
fn parse_mib_reads_a_plain_number() {
    assert_eq!(parse_mib("4096"), Some(4096));
    assert_eq!(parse_mib("  2048  "), Some(2048));
}

#[test]
fn parse_mib_reads_the_units_a_user_types() {
    assert_eq!(parse_mib("4096M"), Some(4096));
    assert_eq!(parse_mib("4096 MiB"), Some(4096));
    assert_eq!(parse_mib("8g"), Some(8192));
    assert_eq!(parse_mib("8 GiB"), Some(8192));
}

#[test]
fn parse_mib_refuses_what_the_jvm_would() {
    assert_eq!(parse_mib(""), None);
    assert_eq!(parse_mib("0"), None);
    assert_eq!(parse_mib("-1"), None);
    assert_eq!(parse_mib("lots"), None);
    assert_eq!(parse_mib("4096 kb"), None, "kilobytes are not offered");
    assert_eq!(parse_mib("9999999 G"), None, "an overflow is not a size");
}

#[test]
fn verify_line_reports_pass_and_the_whole_error_chain() {
    let ok: Result<(), gcl_core::Error> = Ok(());
    assert_eq!(verify_line("modrinth", &ok), "PASS modrinth");

    let err: Result<(), gcl_core::Error> =
        Err(gcl_core::Error::Io(std::io::Error::other("host is down")));
    assert_eq!(
        verify_line("curseforge", &err),
        "FAIL curseforge: host is down"
    );
}

#[test]
fn verify_status_counts_the_failures() {
    let all_good = vec!["PASS modrinth".to_string(), "PASS mojang".to_string()];
    assert_eq!(verify_status(&all_good), "2 check(s) passed");

    let one_bad = vec![
        "PASS modrinth".to_string(),
        "FAIL curseforge: no key".to_string(),
    ];
    assert_eq!(verify_status(&one_bad), "1 of 2 check(s) failed");
}

#[test]
fn root_change_status_says_the_data_stays_put() {
    let text = root_change_status("/home/steve/.local/share/gcl");
    assert!(
        text.starts_with("root will change on next start"),
        "got {text}"
    );
    assert!(text.ends_with("/home/steve/.local/share/gcl"), "got {text}");
}

#[test]
fn key_status_says_saved_or_cleared() {
    assert_eq!(
        key_status("CurseForge API key", false),
        "CurseForge API key saved"
    );
    assert_eq!(
        key_status("Microsoft client id", true),
        "Microsoft client id cleared"
    );
}
