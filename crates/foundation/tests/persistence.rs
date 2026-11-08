use hanafubuki_agent_foundation::persistence::{
    discard_sqlite, quarantine_safe_json, quarantine_sqlite,
};

#[test]
fn safe_json_quarantine_removes_live_and_backup_candidates() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.json");
    let backup = root.path().join("state.json.bak");
    std::fs::write(&path, "old").unwrap();
    std::fs::write(&backup, "older").unwrap();

    let quarantined = quarantine_safe_json(&path).unwrap();

    assert_eq!(quarantined.len(), 2);
    assert!(!path.exists());
    assert!(!backup.exists());
    assert!(quarantined.iter().all(|candidate| candidate.is_file()));
}

#[test]
fn sqlite_quarantine_includes_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("index.sqlite3");
    for candidate in [
        path.clone(),
        root.path().join("index.sqlite3-wal"),
        root.path().join("index.sqlite3-shm"),
    ] {
        std::fs::write(candidate, "old").unwrap();
    }

    let quarantined = quarantine_sqlite(&path).unwrap();

    assert_eq!(quarantined.len(), 3);
    assert!(!path.exists());
}

#[test]
fn sqlite_discard_removes_only_live_database_and_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("index.sqlite3");
    for candidate in [
        path.clone(),
        root.path().join("index.sqlite3-wal"),
        root.path().join("index.sqlite3-shm"),
    ] {
        std::fs::write(candidate, "old").unwrap();
    }
    let recovery = root.path().join(".recovery");
    std::fs::create_dir(&recovery).unwrap();
    let invalid = recovery.join("index.sqlite3.1.1.invalid");
    std::fs::write(&invalid, "older").unwrap();

    discard_sqlite(&path).unwrap();

    assert!(!path.exists());
    assert!(!root.path().join("index.sqlite3-wal").exists());
    assert!(!root.path().join("index.sqlite3-shm").exists());
    assert!(invalid.is_file());
}
