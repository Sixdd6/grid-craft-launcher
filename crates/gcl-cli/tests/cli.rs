use assert_cmd::Command;

#[test]
fn version_flag_prints_version() {
    Command::cargo_bin("gcl")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn debug_verify_source_is_not_implemented_yet() {
    Command::cargo_bin("gcl")
        .unwrap()
        .args(["debug", "verify-source", "modrinth"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("not implemented"));
}
