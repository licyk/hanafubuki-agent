mod support;

use std::fs;

use tempfile::TempDir;

use hanafubuki_agent_memory::*;
use support::{publish_workspace, published_single_source, repository, stage_one, sync_enabled};

fn note_request(
    operation: AdHocMemoryNoteOperation,
    content: impl Into<String>,
) -> AppendAdHocMemoryNote {
    AppendAdHocMemoryNote {
        created_at_millis: 150,
        session_id: "baseline".into(),
        turn_id: 7,
        source_revision: 11,
        operation,
        content: content.into(),
    }
}

#[test]
fn incompatible_database_discards_derived_state_before_canonical_modes_are_resynchronized() {
    let temporary = TempDir::new().unwrap();
    let paths = AgentMemoryPaths::under(temporary.path().join("memory"));
    let repository = AgentMemoryRepository::open(paths.clone()).unwrap();
    repository
        .sync_source_state("disabled", AgentMemoryContributionMode::Disabled, 4, 10)
        .unwrap();
    repository
        .sync_source_state("polluted", AgentMemoryContributionMode::Polluted, 7, 12)
        .unwrap();
    std::fs::write(paths.artifacts.join("stale.md"), "stale derived memory").unwrap();
    drop(repository);
    let connection = rusqlite::Connection::open(&paths.database).unwrap();
    connection
        .execute(
            "UPDATE memory_meta SET schema_version=99 WHERE singleton=1",
            [],
        )
        .unwrap();
    drop(connection);
    let recovery = paths.root.join(".recovery");
    std::fs::create_dir(&recovery).unwrap();
    std::fs::write(
        recovery.join("memory.sqlite3.1.1.invalid"),
        "old memory index",
    )
    .unwrap();

    let rebuilt = AgentMemoryRepository::open(paths.clone()).unwrap();
    assert!(!paths.artifacts.join("stale.md").exists());
    assert!(recovery.join("memory.sqlite3.1.1.invalid").is_file());
    rebuilt
        .sync_source_state("disabled", AgentMemoryContributionMode::Disabled, 4, 10)
        .unwrap();
    rebuilt
        .sync_source_state("polluted", AgentMemoryContributionMode::Polluted, 7, 12)
        .unwrap();
    assert_eq!(
        rebuilt.contribution_mode("disabled").unwrap(),
        (AgentMemoryContributionMode::Disabled, 4)
    );
    assert_eq!(
        rebuilt.contribution_mode("polluted").unwrap(),
        (AgentMemoryContributionMode::Polluted, 7)
    );
    assert!(rebuilt
        .claim_source("disabled", 11, 100, 1_000)
        .unwrap()
        .is_none());
    assert!(rebuilt
        .claim_source("polluted", 13, 100, 1_000)
        .unwrap()
        .is_none());
}

#[test]
fn exact_lease_and_watermark_prevent_duplicate_phase_one_work() {
    let (_temporary, repository) = repository();
    sync_enabled(&repository, "session-1", 3);
    let lease = repository
        .claim_source("session-1", 3, 100, 1_000)
        .unwrap()
        .unwrap();
    assert!(repository
        .claim_source("session-1", 3, 101, 1_000)
        .unwrap()
        .is_none());
    let mut forged = lease.clone();
    forged.token.push('x');
    assert_eq!(
        repository.complete_source(&forged, None, 102),
        Err(AgentMemoryError::LeaseLost)
    );
    repository.complete_source(&lease, None, 102).unwrap();
}

#[test]
fn source_renewal_is_exact_and_extends_the_same_lease() {
    let (_temporary, repository) = repository();
    sync_enabled(&repository, "session-renew", 7);
    let lease = repository
        .claim_source("session-renew", 7, 100, 100)
        .unwrap()
        .unwrap();
    let renewed = repository.renew_source(&lease, 150, 200).unwrap();
    assert_eq!(renewed.token, lease.token);
    assert_eq!(renewed.reset_epoch, lease.reset_epoch);
    assert_eq!(renewed.source_revision, lease.source_revision);
    assert_eq!(renewed.expires_at_millis, 350);
    repository.complete_source(&renewed, None, 250).unwrap();

    sync_enabled(&repository, "session-forged", 9);
    let lease = repository
        .claim_source("session-forged", 9, 300, 100)
        .unwrap()
        .unwrap();
    let mut forged = lease.clone();
    forged.source_revision = forged.source_revision.saturating_add(1);
    assert_eq!(
        repository.renew_source(&forged, 350, 200),
        Err(AgentMemoryError::LeaseLost)
    );
    assert_eq!(
        repository.renew_source(&lease, 400, 200),
        Err(AgentMemoryError::LeaseLost)
    );
}

#[test]
fn consolidation_renewal_is_exact_and_reset_invalidates_it() {
    let (_temporary, repository) = repository();
    sync_enabled(&repository, "dirty", 1);
    let lease = repository.claim_consolidation(100, 100).unwrap().unwrap();
    let renewed = repository.renew_consolidation(&lease, 150, 200).unwrap();
    assert_eq!(renewed.token, lease.token);
    assert_eq!(renewed.reset_epoch, lease.reset_epoch);
    assert_eq!(renewed.source_revision, lease.source_revision);
    assert_eq!(renewed.expires_at_millis, 350);

    let mut forged = renewed.clone();
    forged.source_revision = forged.source_revision.saturating_add(1);
    assert_eq!(
        repository.renew_consolidation(&forged, 200, 200),
        Err(AgentMemoryError::LeaseLost)
    );
    let generation = publish_workspace(
        &repository,
        &renewed,
        &[],
        "# Memory\n\nRenewed lease published.\n",
        "v1\n\nRenewed lease published.\n",
        250,
    );
    assert_eq!(generation, 1);

    sync_enabled(&repository, "dirty-after-publish", 2);
    let next = repository.claim_consolidation(300, 100).unwrap().unwrap();
    repository.reset(1).unwrap();
    assert_eq!(
        repository.renew_consolidation(&next, 350, 200),
        Err(AgentMemoryError::LeaseLost)
    );
}

#[test]
fn failed_or_lost_consolidation_discards_only_the_exact_workspace() {
    let (_temporary, repository) = repository();
    let workspace = repository.prepare_consolidation_workspace(1, &[]).unwrap();
    assert!(workspace.path.is_dir());
    repository
        .discard_consolidation_workspace(&workspace)
        .unwrap();
    assert!(!workspace.path.exists());
    repository
        .discard_consolidation_workspace(&workspace)
        .unwrap();

    let mut forged = workspace;
    forged.path = repository.paths().staging.join("generation-forged");
    assert!(matches!(
        repository.discard_consolidation_workspace(&forged),
        Err(AgentMemoryError::InvalidInput(_))
    ));
}

#[test]
fn failed_jobs_release_the_exact_lease_and_cannot_publish_later() {
    let (_temporary, repository) = repository();
    sync_enabled(&repository, "cancelled-source", 4);
    let source = repository
        .claim_source("cancelled-source", 4, 100, 100)
        .unwrap()
        .unwrap();
    repository.fail_source(&source, 150, "cancelled").unwrap();
    assert_eq!(
        repository.complete_source(&source, None, 160),
        Err(AgentMemoryError::LeaseLost)
    );

    let consolidation = repository.claim_consolidation(200, 100).unwrap().unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(consolidation.source_revision + 1, &[])
        .unwrap();
    repository
        .fail_consolidation(&consolidation, 250, "cancelled")
        .unwrap();
    assert_eq!(
        repository.publish_consolidation_workspace(&consolidation, &workspace, 260),
        Err(AgentMemoryError::LeaseLost)
    );
    repository
        .discard_consolidation_workspace(&workspace)
        .unwrap();
}

#[test]
fn old_session_to_read_usage_and_pollution_forgetting_is_end_to_end() {
    let (_temporary, repository) = repository();
    sync_enabled(&repository, "old-session", 8);
    let source = repository
        .claim_source("old-session", 8, 100, 1_000)
        .unwrap()
        .unwrap();
    repository
        .complete_source(
            &source,
            Some(stage_one(
                "user prefers narrow validation",
                "narrow-validation",
            )),
            101,
        )
        .unwrap();

    let global = repository.claim_consolidation(102, 1_000).unwrap().unwrap();
    let inputs = repository.consolidation_inputs(2).unwrap();
    assert_eq!(inputs.len(), 1);
    let generation = publish_workspace(
        &repository,
        &global,
        &inputs,
        "# Memory\n\nUser prefers narrow validation. Source: old-session.\n",
        "v1\n\n- Validation preferences: MEMORY.md\n",
        103,
    );
    assert_eq!(generation, 1);
    assert!(repository
        .load_context()
        .unwrap()
        .unwrap()
        .summary
        .contains("Validation preferences"));
    assert!(repository
        .list_resources()
        .unwrap()
        .iter()
        .any(|resource| resource.id.starts_with("rollout_summaries/")));
    assert!(repository
        .validate_citation_sources(1, "MEMORY.md", &["old-session".into()])
        .unwrap());
    assert!(!repository
        .validate_citation_sources(1, "MEMORY.md", &["invented-session".into()])
        .unwrap());

    repository
        .record_citation_usage(&["old-session".into()], 104)
        .unwrap();
    repository
        .sync_source_state("old-session", AgentMemoryContributionMode::Polluted, 2, 9)
        .unwrap();
    assert!(repository.consolidation_inputs(2).unwrap().is_empty());
    // Revoking the source schedules a forgetting pass; it does not blind the
    // model in the meantime. What was published stays readable until the pass
    // that removes it actually publishes.
    assert!(repository
        .load_context()
        .unwrap()
        .unwrap()
        .summary
        .contains("Validation preferences"));

    let forgetting = repository.claim_consolidation(105, 1_000).unwrap().unwrap();
    publish_workspace(
        &repository,
        &forgetting,
        &[],
        "# Memory\n\nNo durable memories remain.\n",
        "v1\n\nNo durable memories remain.\n",
        106,
    );
    assert!(repository
        .search("narrow validation", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn identical_phase_two_input_uses_valid_durable_baseline_and_completes_as_noop() {
    let (temporary, repository) = published_single_source();
    repository
        .sync_source_state("baseline", AgentMemoryContributionMode::Enabled, 2, 1)
        .unwrap();
    let lease = repository
        .claim_consolidation(200, 10_000)
        .unwrap()
        .unwrap();
    let inputs = repository.consolidation_inputs(32).unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(lease.source_revision + 1, &inputs)
        .unwrap();
    assert_eq!(workspace.baseline_generation, Some(1));
    assert!(!workspace.has_semantic_changes);
    let diff = fs::read_to_string(workspace.path.join("semantic_diff.md")).unwrap();
    assert!(diff.contains("semanticChanges: 0"));
    assert_eq!(
        repository
            .complete_consolidation_noop(&lease, &workspace, 201)
            .unwrap(),
        1
    );
    repository
        .discard_consolidation_workspace(&workspace)
        .unwrap();
    assert_eq!(repository.generation().unwrap(), 1);
    drop(repository);

    let reopened =
        AgentMemoryRepository::open(AgentMemoryPaths::under(temporary.path().join("memory")))
            .unwrap();
    reopened
        .sync_source_state("baseline", AgentMemoryContributionMode::Enabled, 3, 1)
        .unwrap();
    let lease = reopened.claim_consolidation(300, 10_000).unwrap().unwrap();
    let inputs = reopened.consolidation_inputs(32).unwrap();
    let workspace = reopened
        .prepare_consolidation_workspace(lease.source_revision + 1, &inputs)
        .unwrap();
    assert_eq!(workspace.baseline_generation, Some(1));
    assert!(!workspace.has_semantic_changes);
}

#[test]
fn semantic_diff_reports_add_update_and_all_removal_reasons() {
    let (_temporary, repository) = published_single_source();
    sync_enabled(&repository, "added", 1);
    let added = repository
        .claim_source("added", 1, 200, 10_000)
        .unwrap()
        .unwrap();
    repository
        .complete_source(&added, Some(stage_one("added memory", "added")), 201)
        .unwrap();
    repository.upsert_source_revision("baseline", 2).unwrap();
    let updated = repository
        .claim_source("baseline", 2, 202, 10_000)
        .unwrap()
        .unwrap();
    repository
        .complete_source(
            &updated,
            Some(stage_one("updated baseline memory", "baseline")),
            203,
        )
        .unwrap();
    let lease = repository
        .claim_consolidation(204, 10_000)
        .unwrap()
        .unwrap();
    let inputs = repository.consolidation_inputs(32).unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(lease.source_revision + 1, &inputs)
        .unwrap();
    let diff = fs::read_to_string(workspace.path.join("semantic_diff.md")).unwrap();
    assert!(diff.contains("## added: added"));
    assert!(diff.contains("## updated: baseline"));

    for (mode, expected) in [
        (AgentMemoryContributionMode::Disabled, "reason: disabled"),
        (AgentMemoryContributionMode::Polluted, "reason: polluted"),
    ] {
        let (_temporary, repository) = published_single_source();
        repository
            .sync_source_state("baseline", mode, 2, 1)
            .unwrap();
        let lease = repository
            .claim_consolidation(200, 10_000)
            .unwrap()
            .unwrap();
        let workspace = repository
            .prepare_consolidation_workspace(
                lease.source_revision + 1,
                &repository.consolidation_inputs(32).unwrap(),
            )
            .unwrap();
        let diff = fs::read_to_string(workspace.path.join("semantic_diff.md")).unwrap();
        assert!(diff.contains(expected));
    }

    let (_temporary, repository) = published_single_source();
    repository.revoke_source("baseline").unwrap();
    let lease = repository
        .claim_consolidation(200, 10_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(
            lease.source_revision + 1,
            &repository.consolidation_inputs(32).unwrap(),
        )
        .unwrap();
    assert!(fs::read_to_string(workspace.path.join("semantic_diff.md"))
        .unwrap()
        .contains("reason: revoked"));

    let (_temporary, repository) = published_single_source();
    let connection = rusqlite::Connection::open(&repository.paths().database).unwrap();
    connection
        .execute(
            "UPDATE memory_stage1 SET generated_at=0, last_used_at=NULL WHERE session_id='baseline'",
            [],
        )
        .unwrap();
    drop(connection);
    repository
        .expire_unused_stage_one(3 * 24 * 60 * 60 * 1_000, 1)
        .unwrap();
    let lease = repository
        .claim_consolidation(300_000_000, 10_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(
            lease.source_revision + 1,
            &repository.consolidation_inputs(32).unwrap(),
        )
        .unwrap();
    assert!(fs::read_to_string(workspace.path.join("semantic_diff.md"))
        .unwrap()
        .contains("reason: expired"));

    let (_temporary, repository) = published_single_source();
    let connection = rusqlite::Connection::open(&repository.paths().database).unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    connection
        .execute("DELETE FROM memory_sources WHERE session_id='baseline'", [])
        .unwrap();
    connection
        .execute("UPDATE memory_meta SET dirty=1 WHERE singleton=1", [])
        .unwrap();
    drop(connection);
    let lease = repository
        .claim_consolidation(200, 10_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(
            lease.source_revision + 1,
            &repository.consolidation_inputs(32).unwrap(),
        )
        .unwrap();
    assert!(fs::read_to_string(workspace.path.join("semantic_diff.md"))
        .unwrap()
        .contains("reason: deleted"));
}

#[test]
fn no_op_revalidates_artifacts_before_advancing_the_watermark() {
    let (_temporary, repository) = published_single_source();
    repository
        .sync_source_state("baseline", AgentMemoryContributionMode::Enabled, 2, 1)
        .unwrap();
    let lease = repository
        .claim_consolidation(200, 10_000)
        .unwrap()
        .unwrap();
    let inputs = repository.consolidation_inputs(32).unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(lease.source_revision + 1, &inputs)
        .unwrap();
    assert!(!workspace.has_semantic_changes);
    fs::write(
        repository.paths().artifacts.join("MEMORY.md"),
        "# Memory\n\ncorrupted after staging\n",
    )
    .unwrap();
    assert!(matches!(
        repository.complete_consolidation_noop(&lease, &workspace, 201),
        Err(AgentMemoryError::Corrupt(_))
    ));
    assert_eq!(repository.generation().unwrap(), 1);
    repository
        .fail_consolidation(&lease, 202, "artifact validation failed")
        .unwrap();
    repository
        .discard_consolidation_workspace(&workspace)
        .unwrap();
}

#[test]
fn semantic_diff_is_item_and_total_bounded_with_utf8_safe_head_tail() {
    let (_temporary, repository) = repository();
    let large = "记忆🙂".repeat(700);
    let inputs = (0..600)
        .map(|index| {
            (
                format!("session-{index:04}"),
                1,
                StageOneResult {
                    raw_memory: large.clone(),
                    rollout_summary: format!("summary {index} {large}"),
                    rollout_slug: format!("task-{index:04}"),
                },
            )
        })
        .collect::<Vec<_>>();
    let workspace = repository
        .prepare_consolidation_workspace(1, &inputs)
        .unwrap();
    let bytes = fs::read(workspace.path.join("semantic_diff.md")).unwrap();
    assert!(bytes.len() <= 128 * 1024);
    let diff = String::from_utf8(bytes).unwrap();
    assert!(diff.contains("UTF-8 head/tail omission"));
    assert!(diff.contains("semantic diff omitted additional entries"));
}

#[test]
fn invalid_clean_generation_never_enables_noop() {
    let corruptions: [fn(&AgentMemoryRepository); 3] = [
        |repository| {
            let path = repository.paths().artifacts.join("manifest.json");
            let manifest = fs::read_to_string(&path).unwrap();
            fs::write(
                path,
                manifest.replace("\"schemaVersion\": 2", "\"schemaVersion\": 99"),
            )
            .unwrap();
        },
        |repository| {
            fs::write(
                repository.paths().artifacts.join("MEMORY.md"),
                vec![b'x'; MAX_MEMORY_RESOURCE_BYTES + 1],
            )
            .unwrap();
        },
        |repository| {
            let path = repository.paths().artifacts.join("manifest.json");
            let manifest = fs::read_to_string(&path).unwrap();
            fs::write(
                path,
                manifest.replace("\"sessionId\": \"baseline\"", "\"sessionId\": \"forged\""),
            )
            .unwrap();
        },
    ];
    for corrupt in corruptions {
        let (_temporary, repository) = published_single_source();
        corrupt(&repository);
        repository
            .sync_source_state("baseline", AgentMemoryContributionMode::Enabled, 2, 1)
            .unwrap();
        let lease = repository
            .claim_consolidation(200, 10_000)
            .unwrap()
            .unwrap();
        let workspace = repository
            .prepare_consolidation_workspace(
                lease.source_revision + 1,
                &repository.consolidation_inputs(32).unwrap(),
            )
            .unwrap();
        assert_eq!(workspace.baseline_generation, None);
        assert!(workspace.has_semantic_changes);
    }
}

#[test]
fn ad_hoc_notes_are_typed_append_only_and_reject_invalid_or_duplicate_input() {
    let (_temporary, repository) = repository();
    assert!(matches!(
        repository.append_ad_hoc_note(note_request(
            AdHocMemoryNoteOperation::Remember,
            "remember this"
        )),
        Err(AgentMemoryError::InvalidInput(_))
    ));
    sync_enabled(&repository, "baseline", 1);
    let note = repository
        .append_ad_hoc_note(note_request(
            AdHocMemoryNoteOperation::Remember,
            "Use narrow validation. token=secret-value",
        ))
        .unwrap();
    assert_eq!(note.version, 1);
    assert_eq!(note.session_id, "baseline");
    assert_eq!(note.turn_id, 7);
    assert_eq!(note.source_revision, 11);
    assert_eq!(note.operation, AdHocMemoryNoteOperation::Remember);
    assert!(!note.content.contains("secret-value"));
    assert_eq!(note.note_id.len(), 64);
    let lines = fs::read_to_string(&repository.paths().ad_hoc_notes).unwrap();
    assert_eq!(lines.lines().count(), 1);
    assert!(serde_json::from_str::<AdHocMemoryNote>(lines.trim()).is_ok());

    let mut duplicate = note_request(
        AdHocMemoryNoteOperation::Remember,
        "Use narrow validation. token=secret-value",
    );
    duplicate.created_at_millis = 999;
    assert!(matches!(
        repository.append_ad_hoc_note(duplicate),
        Err(AgentMemoryError::InvalidInput(_))
    ));
    assert!(matches!(
        repository.append_ad_hoc_note(note_request(AdHocMemoryNoteOperation::Forget, "   ")),
        Err(AgentMemoryError::InvalidInput(_))
    ));
    assert!(matches!(
        repository.append_ad_hoc_note(note_request(
            AdHocMemoryNoteOperation::Update,
            "x".repeat(MAX_AD_HOC_MEMORY_NOTE_BYTES + 1)
        )),
        Err(AgentMemoryError::InvalidInput(_))
    ));
}

#[test]
fn ad_hoc_note_flows_through_phase_two_and_policy_removes_it_without_rewriting_evidence() {
    let (_temporary, repository) = published_single_source();
    let remember = repository
        .append_ad_hoc_note(note_request(
            AdHocMemoryNoteOperation::Remember,
            "Prefer Bun commands.",
        ))
        .unwrap();
    let update = repository
        .append_ad_hoc_note(note_request(
            AdHocMemoryNoteOperation::Update,
            "Update the package manager preference to Bun 1.3.",
        ))
        .unwrap();
    let lease = repository
        .claim_consolidation(200, 10_000)
        .unwrap()
        .unwrap();
    let inputs = repository.consolidation_inputs(32).unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(lease.source_revision + 1, &inputs)
        .unwrap();
    assert_eq!(workspace.notes.len(), 2);
    let staged_notes = fs::read_to_string(workspace.path.join("ad_hoc_notes.md")).unwrap();
    assert!(staged_notes.contains(&remember.note_id));
    assert!(staged_notes.contains(&update.note_id));
    assert!(staged_notes.contains("operation: remember"));
    assert!(staged_notes.contains("operation: update"));
    let diff = fs::read_to_string(workspace.path.join("semantic_diff.md")).unwrap();
    assert!(diff.contains("## added note:"));
    fs::write(
        workspace.path.join("MEMORY.md"),
        "# Memory\n\nPrefer Bun 1.3. Source: baseline.\n",
    )
    .unwrap();
    fs::write(
        workspace.path.join("memory_summary.md"),
        "v1\n\n- Package manager preference: MEMORY.md\n",
    )
    .unwrap();
    assert_eq!(
        repository
            .publish_consolidation_workspace(&lease, &workspace, 201)
            .unwrap(),
        2
    );
    assert!(!repository
        .list_resources()
        .unwrap()
        .iter()
        .any(|resource| resource.id == "ad_hoc_notes.md"));
    assert!(repository
        .validate_citation_sources(2, "MEMORY.md", &["baseline".into()])
        .unwrap());

    repository
        .sync_source_state("baseline", AgentMemoryContributionMode::Disabled, 2, 1)
        .unwrap();
    let lease = repository
        .claim_consolidation(300, 10_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(
            lease.source_revision + 1,
            &repository.consolidation_inputs(32).unwrap(),
        )
        .unwrap();
    assert!(workspace.notes.is_empty());
    let diff = fs::read_to_string(workspace.path.join("semantic_diff.md")).unwrap();
    assert!(diff.contains("removed note"));
    assert!(diff.contains("reason: disabled"));
    fs::write(
        workspace.path.join("MEMORY.md"),
        "# Memory\n\nNo durable memories remain.\n",
    )
    .unwrap();
    fs::write(
        workspace.path.join("memory_summary.md"),
        "v1\n\nNo durable memories remain.\n",
    )
    .unwrap();
    repository
        .publish_consolidation_workspace(&lease, &workspace, 301)
        .unwrap();
    assert!(repository.search("Bun 1.3", 10).unwrap().is_empty());

    repository.reset(3).unwrap();
    repository
        .sync_source_state("baseline", AgentMemoryContributionMode::Enabled, 3, 2)
        .unwrap();
    let lease = repository
        .claim_consolidation(400, 10_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(
            lease.source_revision + 1,
            &repository.consolidation_inputs(32).unwrap(),
        )
        .unwrap();
    assert!(workspace.notes.is_empty());
    assert!(!fs::read_to_string(workspace.path.join("ad_hoc_notes.md"))
        .unwrap()
        .contains("Prefer Bun"));
    assert_eq!(
        fs::read_to_string(&repository.paths().ad_hoc_notes)
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn polluted_source_removes_published_ad_hoc_note_provenance() {
    let (_temporary, repository) = published_single_source();
    repository
        .append_ad_hoc_note(note_request(
            AdHocMemoryNoteOperation::Remember,
            "This fact must be forgotten after pollution.",
        ))
        .unwrap();
    let lease = repository
        .claim_consolidation(200, 10_000)
        .unwrap()
        .unwrap();
    let inputs = repository.consolidation_inputs(32).unwrap();
    publish_workspace(
        &repository,
        &lease,
        &inputs,
        "# Memory\n\nTemporary fact. Source: baseline.\n",
        "v1\n\nTemporary fact.\n",
        201,
    );
    repository
        .sync_source_state("baseline", AgentMemoryContributionMode::Polluted, 2, 1)
        .unwrap();
    let lease = repository
        .claim_consolidation(300, 10_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(
            lease.source_revision + 1,
            &repository.consolidation_inputs(32).unwrap(),
        )
        .unwrap();
    assert!(workspace.notes.is_empty());
    let diff = fs::read_to_string(workspace.path.join("semantic_diff.md")).unwrap();
    assert!(diff.contains("removed note"));
    assert!(diff.contains("reason: polluted"));
}

#[cfg(unix)]
#[test]
fn symlinked_clean_generation_never_enables_noop() {
    use std::os::unix::fs::symlink;

    let (_temporary, repository) = published_single_source();
    let memory = repository.paths().artifacts.join("MEMORY.md");
    fs::remove_file(&memory).unwrap();
    symlink("memory_summary.md", &memory).unwrap();
    repository
        .sync_source_state("baseline", AgentMemoryContributionMode::Enabled, 2, 1)
        .unwrap();
    let lease = repository
        .claim_consolidation(200, 10_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(
            lease.source_revision + 1,
            &repository.consolidation_inputs(32).unwrap(),
        )
        .unwrap();
    assert_eq!(workspace.baseline_generation, None);
    assert!(workspace.has_semantic_changes);
}

#[test]
fn reset_invalidates_leases_without_deleting_projected_modes() {
    let (_temporary, repository) = repository();
    repository
        .sync_source_state("session-1", AgentMemoryContributionMode::Disabled, 3, 1)
        .unwrap();
    repository
        .sync_source_state("dirty", AgentMemoryContributionMode::Enabled, 1, 1)
        .unwrap();
    let global = repository.claim_consolidation(100, 1_000).unwrap().unwrap();
    let workspace = repository.prepare_consolidation_workspace(1, &[]).unwrap();
    repository.reset(0).unwrap();
    assert_eq!(
        repository.publish_consolidation_workspace(&global, &workspace, 101),
        Err(AgentMemoryError::LeaseLost)
    );
    assert_eq!(
        repository.contribution_mode("session-1").unwrap().0,
        AgentMemoryContributionMode::Disabled
    );
}

#[test]
fn resource_ids_cannot_escape_memory_artifacts() {
    let (_temporary, repository) = repository();
    for id in [
        "../memory.sqlite3",
        "/etc/passwd",
        ".hidden.md",
        "memory.sqlite3",
    ] {
        assert!(matches!(
            repository.read_resource(id, 1, 10),
            Err(AgentMemoryError::InvalidInput(_)) | Err(AgentMemoryError::NotFound)
        ));
    }
}

#[test]
fn redaction_and_trailing_citations_remain_bounded() {
    let text = "token=abc123\nAuthorization: Bearer secret\nexact-value-123";
    let redacted = redact_memory_text(text, &["exact-value-123".into()]);
    assert!(!redacted.contains("abc123"));
    assert!(!redacted.contains("Bearer secret"));
    assert!(!redacted.contains("exact-value-123"));

    let answer = "answer\n<hanafubuki-memory-citations>[{\"resourceId\":\"rollout_summaries/1-task.md\",\"startLine\":1,\"endLine\":2,\"note\":\"used\",\"sourceSessionIds\":[\"1\"]}]</hanafubuki-memory-citations>";
    let parsed = parse_memory_citations(answer).unwrap();
    assert_eq!(parsed.visible_text, "answer");
    assert_eq!(parsed.citations.len(), 1);
}

/// The read path is the artifacts folder as it stands.
///
/// Everything asserted here used to fail: the generation counter belongs to the
/// write pipeline, and using it to admit reads meant a user's memories were
/// invisible until a consolidation pass had run, and invisible again the moment
/// a new source arrived to be consolidated.
#[test]
fn published_memory_is_readable_without_a_consolidation_pass() {
    let (_temporary, repository) = repository();
    let artifacts = &repository.paths().artifacts;
    fs::write(
        artifacts.join("memory_summary.md"),
        "v1\n\n- Validation preferences: MEMORY.md\n",
    )
    .unwrap();
    fs::write(
        artifacts.join("MEMORY.md"),
        "# Memory\n\nThe user prefers narrow validation runs.\n",
    )
    .unwrap();

    assert_eq!(repository.generation().unwrap(), 0);
    let context = repository.load_context().unwrap().unwrap();
    assert!(context.summary.contains("Validation preferences"));
    assert_eq!(context.generation, 0);

    let ids = repository
        .list_resources()
        .unwrap()
        .into_iter()
        .map(|resource| resource.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, ["MEMORY.md", "memory_summary.md"]);
    assert_eq!(
        repository
            .search("narrow validation", 10)
            .unwrap()
            .first()
            .map(|hit| hit.resource_id.clone()),
        Some("MEMORY.md".to_string())
    );
    assert!(repository
        .read_resource("MEMORY.md", 1, 10)
        .unwrap()
        .content
        .contains("narrow validation"));

    // A source waiting for the next pass leaves published memory readable.
    sync_enabled(&repository, "pending-source", 3);
    assert!(repository
        .load_context()
        .unwrap()
        .unwrap()
        .summary
        .contains("Validation preferences"));
}

/// A read bound is a context budget. Exceeding it costs the overflow, never the
/// whole memory: the summary is truncated, and an over-long resource is still
/// listed and still readable.
#[test]
fn oversized_memory_is_bounded_rather_than_withheld() {
    let (_temporary, repository) = repository();
    let artifacts = &repository.paths().artifacts;
    let summary = format!("v1 preferences\n{}", "s".repeat(MAX_MEMORY_SUMMARY_BYTES));
    fs::write(artifacts.join("memory_summary.md"), &summary).unwrap();
    fs::write(
        artifacts.join("MEMORY.md"),
        "x".repeat(MAX_MEMORY_RESOURCE_BYTES + 4_096),
    )
    .unwrap();

    let context = repository.load_context().unwrap().unwrap();
    assert!(context.summary.starts_with("v1 preferences"));
    assert_eq!(context.summary.len(), MAX_MEMORY_SUMMARY_BYTES);
    assert!(repository
        .list_resources()
        .unwrap()
        .iter()
        .any(|resource| resource.id == "MEMORY.md"));
    assert!(repository.read_resource("MEMORY.md", 1, 10).is_ok());
}

#[test]
fn an_empty_or_missing_summary_contributes_nothing() {
    let (_temporary, repository) = repository();
    assert!(repository.load_context().unwrap().is_none());
    fs::write(
        repository.paths().artifacts.join("memory_summary.md"),
        "   \n\n",
    )
    .unwrap();
    assert!(repository.load_context().unwrap().is_none());
}
