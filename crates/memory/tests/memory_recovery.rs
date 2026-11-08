//! What a failed job, a crash, a pass in progress or an older database
//! leaves of the memory pipeline's work.

mod support;

use std::fs;

use rusqlite::Connection;
use tempfile::TempDir;

use hanafubuki_agent_memory::*;
use support::{publish_workspace, published_single_source, repository, stage_one, sync_enabled};

fn extract(repository: &AgentMemoryRepository, session: &str, revision: u64, now: u64) {
    sync_enabled(repository, session, revision);
    let lease = repository
        .claim_source(session, revision, now, 10_000)
        .unwrap()
        .unwrap();
    repository
        .complete_source(&lease, Some(stage_one(session, session)), now + 1)
        .unwrap();
}

fn manifest_generation(paths: &AgentMemoryPaths) -> u64 {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(paths.artifacts.join("manifest.json")).unwrap()).unwrap();
    manifest["generation"].as_u64().unwrap()
}

#[test]
fn a_failed_consolidation_backs_off_like_a_failed_extraction() {
    let (_temporary, repository) = repository();
    extract(&repository, "source", 1, 100);

    let first = repository.claim_consolidation(200, 1_000).unwrap().unwrap();
    repository
        .fail_consolidation(&first, 300, "failed")
        .unwrap();
    assert!(repository
        .claim_consolidation(301, 1_000)
        .unwrap()
        .is_none());
    let second = repository
        .claim_consolidation(300 + 30_000, 1_000)
        .unwrap()
        .unwrap();
    repository
        .fail_consolidation(&second, 31_000, "failed again")
        .unwrap();
    // The wait doubles with each failure.
    assert!(repository
        .claim_consolidation(31_000 + 59_999, 1_000)
        .unwrap()
        .is_none());
    let third = repository
        .claim_consolidation(31_000 + 60_000, 1_000)
        .unwrap()
        .unwrap();

    // A success clears the backoff.
    let inputs = repository.consolidation_inputs(8).unwrap();
    publish_workspace(&repository, &third, &inputs, "# Memory\n", "v1\n", 91_001);
    extract(&repository, "later", 1, 91_002);
    let fourth = repository
        .claim_consolidation(91_010, 1_000)
        .unwrap()
        .unwrap();
    repository
        .fail_consolidation(&fourth, 91_020, "failed")
        .unwrap();
    assert!(repository
        .claim_consolidation(91_020 + 29_999, 1_000)
        .unwrap()
        .is_none());
    assert!(repository
        .claim_consolidation(91_020 + 30_000, 1_000)
        .unwrap()
        .is_some());
}

/// A second generation committed while its install was refused: the
/// previous generation's backup path is taken by a file.
fn committed_but_not_installed() -> (TempDir, AgentMemoryRepository) {
    let (temporary, repository) = published_single_source();
    extract(&repository, "second", 1, 200);
    let lease = repository
        .claim_consolidation(210, 10_000)
        .unwrap()
        .unwrap();
    let inputs = repository.consolidation_inputs(8).unwrap();
    let blocker = repository.paths().reset_trash.join("artifacts-before-2");
    fs::write(&blocker, "not a directory").unwrap();

    let generation = publish_workspace(
        &repository,
        &lease,
        &inputs,
        "# Memory\n\nSecond generation.\n",
        "v1\n\nSecond generation.\n",
        220,
    );

    assert_eq!(generation, 2);
    assert_eq!(repository.generation().unwrap(), 2);
    assert_eq!(manifest_generation(repository.paths()), 1);
    assert!(repository
        .load_context()
        .unwrap()
        .unwrap()
        .summary
        .contains("Baseline memory"));
    assert!(repository.paths().staging.join("generation-2").is_dir());
    fs::remove_file(blocker).unwrap();
    (temporary, repository)
}

#[test]
fn a_committed_generation_is_installed_at_the_next_open() {
    let (_temporary, repository) = committed_but_not_installed();
    let paths = repository.paths().clone();
    drop(repository);

    let reopened = AgentMemoryRepository::open(paths.clone()).unwrap();

    assert_eq!(manifest_generation(&paths), 2);
    assert!(reopened
        .load_context()
        .unwrap()
        .unwrap()
        .summary
        .contains("Second generation"));
    assert_eq!(fs::read_dir(&paths.staging).unwrap().count(), 0);
}

#[test]
fn a_committed_generation_is_installed_before_the_next_consolidation_is_prepared() {
    let (_temporary, repository) = committed_but_not_installed();
    extract(&repository, "third", 1, 300);

    let lease = repository
        .claim_consolidation(310, 10_000)
        .unwrap()
        .unwrap();

    assert_eq!(manifest_generation(repository.paths()), 2);
    let workspace = repository
        .prepare_consolidation_workspace(lease.source_revision + 1, &[])
        .unwrap();
    assert_eq!(workspace.baseline_generation, Some(2));
    assert!(fs::read_to_string(workspace.path.join("MEMORY.md"))
        .unwrap()
        .contains("Second generation"));
}

#[test]
fn citation_usage_is_kept_while_sources_wait_for_consolidation() {
    let (_temporary, repository) = published_single_source();
    // A newer extraction leaves the published generation behind.
    extract(&repository, "newer", 1, 200);

    repository
        .record_citation_usage(&["baseline".into()], 210)
        .unwrap();

    let inputs = repository.consolidation_inputs(1).unwrap();
    assert_eq!(inputs[0].0, "baseline", "the cited source ranks first");
}

#[test]
fn a_version_four_database_keeps_its_memory() {
    let (_temporary, repository) = published_single_source();
    let paths = repository.paths().clone();
    drop(repository);
    let connection = Connection::open(&paths.database).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE memory_meta DROP COLUMN global_retry_count;
             ALTER TABLE memory_meta DROP COLUMN global_next_retry_at;
             UPDATE memory_meta SET schema_version=4 WHERE singleton=1;",
        )
        .unwrap();
    drop(connection);

    let reopened = AgentMemoryRepository::open(paths.clone()).unwrap();

    assert_eq!(reopened.generation().unwrap(), 1);
    assert_eq!(manifest_generation(&paths), 1);
    assert_eq!(
        reopened.contribution_mode("baseline").unwrap(),
        (AgentMemoryContributionMode::Enabled, 1)
    );
    assert_eq!(reopened.consolidation_inputs(8).unwrap().len(), 1);
    extract(&reopened, "after-migration", 1, 400);
    let lease = reopened.claim_consolidation(410, 1_000).unwrap().unwrap();
    reopened.fail_consolidation(&lease, 420, "failed").unwrap();
    assert!(reopened.claim_consolidation(421, 1_000).unwrap().is_none());
}

#[test]
fn an_unknown_contribution_mode_reads_as_disabled() {
    let (_temporary, repository) = published_single_source();
    Connection::open(&repository.paths().database)
        .unwrap()
        .execute(
            "UPDATE memory_sources SET contribution_mode='from_a_newer_build' WHERE session_id='baseline'",
            [],
        )
        .unwrap();

    assert_eq!(
        repository.contribution_mode("baseline").unwrap(),
        (AgentMemoryContributionMode::Disabled, 1)
    );
}
