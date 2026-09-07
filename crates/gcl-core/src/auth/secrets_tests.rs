//! Tests for the secret stores.
//!
//! Nothing here touches the real OS keyring: every test that calls [`open_default`] sets
//! `GCL_NO_KEYRING=1` first, and [`KeyringStore`] itself is only compiled, never run.

use super::*;

/// Sets `GCL_NO_KEYRING=1` for this test.
///
/// Safe here because nextest runs each test in its own process, so no other thread is
/// reading the environment at the same time.
fn forbid_the_keyring() {
    unsafe { std::env::set_var(NO_KEYRING_ENV, "1") };
}

/// The file's permission bits.
#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

/// Sets the file's permission bits.
#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("set_permissions");
}

#[test]
fn memory_store_round_trips() {
    let store = MemoryStore::new();
    assert_eq!(store.kind(), SecretStoreKind::Memory);
    assert_eq!(store.get("a").expect("get"), None);
    store.put("a", "token-a").expect("put");
    store.put("b", "token-b").expect("put");
    assert_eq!(store.get("a").expect("get").as_deref(), Some("token-a"));
    store.put("a", "token-a2").expect("replace");
    assert_eq!(store.get("a").expect("get").as_deref(), Some("token-a2"));
    store.delete("a").expect("delete");
    assert_eq!(store.get("a").expect("get"), None);
    assert_eq!(store.get("b").expect("get").as_deref(), Some("token-b"));
    store.delete("a").expect("delete is idempotent");
}

#[test]
fn file_store_round_trips_in_a_temp_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(&Root::from_path(dir.path()));
    assert_eq!(store.kind(), SecretStoreKind::File);
    assert_eq!(store.path(), dir.path().join("secrets.json"));

    assert_eq!(store.get("unknown").expect("get"), None);
    store.put("id-1", "token-1").expect("put");
    store.put("id-2", "token-2").expect("put");
    assert_eq!(store.get("id-1").expect("get").as_deref(), Some("token-1"));

    store.put("id-1", "token-1b").expect("replace");
    assert_eq!(store.get("id-1").expect("get").as_deref(), Some("token-1b"));

    store.delete("id-1").expect("delete");
    assert_eq!(store.get("id-1").expect("get"), None);
    assert_eq!(store.get("id-2").expect("get").as_deref(), Some("token-2"));
    store.delete("id-1").expect("delete is idempotent");
    store.delete("absent").expect("delete is idempotent");
}

#[test]
fn file_store_writes_the_documented_json_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(&Root::from_path(dir.path()));
    store.put("id-1", "token-1").expect("put");

    let text = std::fs::read_to_string(store.path()).expect("file exists");
    let map: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&text).expect("json map");
    assert_eq!(map.get("id-1").map(String::as_str), Some("token-1"));
}

#[test]
fn file_store_reads_a_store_written_by_an_earlier_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    FileStore::new(&root).put("id-1", "token-1").expect("put");
    let reopened = FileStore::new(&root);
    assert_eq!(
        reopened.get("id-1").expect("get").as_deref(),
        Some("token-1")
    );
}

#[cfg(unix)]
#[test]
fn file_store_creates_the_file_at_0600_and_leaves_no_temp_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(&Root::from_path(dir.path()));
    store.put("id-1", "token-1").expect("put");
    assert_eq!(mode_of(store.path()), 0o600, "after the first write");

    store.put("id-2", "token-2").expect("second write");
    assert_eq!(mode_of(store.path()), 0o600, "after the second write");

    // The temp file the atomic write goes through is created at 0600 too, and is renamed
    // away, so nothing but secrets.json is left in the root.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "secrets.json")
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[cfg(unix)]
#[test]
fn file_store_replaces_a_world_readable_file_from_an_older_launcher() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(&Root::from_path(dir.path()));
    std::fs::write(store.path(), "{}").expect("write");
    set_mode(store.path(), 0o644);
    assert_eq!(
        mode_of(store.path()),
        0o644,
        "the fixture is world-readable"
    );

    store.put("id-1", "token-1").expect("put");
    assert_eq!(mode_of(store.path()), 0o600, "put must narrow the mode");
    assert_eq!(store.get("id-1").expect("get").as_deref(), Some("token-1"));
}

#[test]
fn file_store_reports_a_broken_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(&Root::from_path(dir.path()));
    std::fs::write(store.path(), "not json").expect("write");
    let err = store.get("id-1").expect_err("a broken file fails");
    assert!(matches!(err, Error::Json { .. }), "got {err:?}");
}

#[test]
fn open_default_falls_back_to_the_file_store_and_warns_once() {
    forbid_the_keyring();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let store = open_default(&root, &tx);
    assert_eq!(store.kind(), SecretStoreKind::File);

    let mut warnings = Vec::new();
    while let Ok(event) = rx.try_recv() {
        match event {
            Event::Warning(message) => warnings.push(message),
            other => panic!("unexpected event {other:?}"),
        }
    }
    assert_eq!(warnings.len(), 1, "got {warnings:?}");
    assert!(
        warnings[0].starts_with(&format!(
            "the OS keyring is turned off by {NO_KEYRING_ENV};"
        )),
        "{warnings:?}"
    );
    assert!(
        warnings[0].contains(&dir.path().join("secrets.json").display().to_string()),
        "{warnings:?}"
    );
}

#[test]
fn the_store_open_default_returns_round_trips() {
    forbid_the_keyring();
    let dir = tempfile::tempdir().expect("tempdir");
    let store = open_default(&Root::from_path(dir.path()), &crate::events::null_sink());
    store.put("id-1", "token-1").expect("put");
    assert_eq!(store.get("id-1").expect("get").as_deref(), Some("token-1"));
    store.delete("id-1").expect("delete");
    assert_eq!(store.get("id-1").expect("get"), None);
}
