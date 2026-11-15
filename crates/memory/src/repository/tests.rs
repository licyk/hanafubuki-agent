//! What opening an existing memory database discards: a damaged one or one
//! of another schema (its contents are derived and rebuilt), never one that
//! is only busy.

use super::*;
use crate::AgentMemoryContributionMode;

fn opened(name: &str) -> (tempfile::TempDir, AgentMemoryPaths) {
    let temporary = tempfile::TempDir::new().unwrap();
    let paths = AgentMemoryPaths::under(temporary.path().join(name));
    let repository = AgentMemoryRepository::open(paths.clone()).unwrap();
    repository
        .sync_source_state("kept", AgentMemoryContributionMode::Disabled, 4, 10)
        .unwrap();
    std::fs::write(paths.artifacts.join("derived.md"), "derived memory").unwrap();
    (temporary, paths)
}

#[test]
fn another_repository_keeps_live_staging_and_shares_updates() {
    let (_temporary, paths) = opened("shared");
    let first = AgentMemoryRepository::open(paths.clone()).unwrap();
    let staging = paths.staging.join("generation-1");
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("MEMORY.md"), "work in progress").unwrap();
    let second = AgentMemoryRepository::open(paths.clone()).unwrap();
    assert_eq!(
        std::fs::read_to_string(staging.join("MEMORY.md")).unwrap(),
        "work in progress"
    );
    second
        .sync_source_state("other", AgentMemoryContributionMode::Enabled, 1, 10)
        .unwrap();
    assert_eq!(
        first.contribution_mode("other").unwrap(),
        (AgentMemoryContributionMode::Enabled, 1)
    );
    assert_ne!(first.new_token("same", 10), second.new_token("same", 10));
    drop(second);
    drop(first);
    AgentMemoryRepository::open(paths).unwrap();
    assert!(!staging.exists());
}

#[test]
fn a_busy_database_fails_the_open_and_is_kept() {
    let (_temporary, paths) = opened("busy");
    let holder = Connection::open(&paths.database).unwrap();
    // Another connection holds the write lock the open needs.
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();

    let error = AgentMemoryRepository::open(paths.clone())
        .err()
        .expect("a locked database opened");

    assert!(matches!(error, AgentMemoryError::Io(_)), "{error}");
    holder.execute_batch("ROLLBACK").unwrap();
    drop(holder);
    assert!(paths.artifacts.join("derived.md").exists());
    let reopened = AgentMemoryRepository::open(paths).unwrap();
    assert_eq!(
        reopened.contribution_mode("kept").unwrap(),
        (AgentMemoryContributionMode::Disabled, 4)
    );
}

#[test]
fn a_database_of_another_schema_version_is_discarded() {
    let (_temporary, paths) = opened("version");
    Connection::open(&paths.database)
        .unwrap()
        .execute(
            "UPDATE memory_meta SET schema_version=99 WHERE singleton=1",
            [],
        )
        .unwrap();

    let reopened = AgentMemoryRepository::open(paths.clone()).unwrap();

    assert!(!paths.artifacts.join("derived.md").exists());
    assert!(reopened.contribution_mode("kept").is_err());
}

#[test]
fn a_file_that_is_not_a_database_is_discarded() {
    let (_temporary, paths) = opened("not-a-database");
    for sidecar in ["-wal", "-shm"] {
        let mut path = paths.database.as_os_str().to_owned();
        path.push(sidecar);
        let _ = std::fs::remove_file(std::path::PathBuf::from(path));
    }
    std::fs::write(&paths.database, vec![0x5a_u8; 8192]).unwrap();

    let reopened = AgentMemoryRepository::open(paths.clone()).unwrap();

    assert!(!paths.artifacts.join("derived.md").exists());
    assert!(reopened.contribution_mode("kept").is_err());
}
