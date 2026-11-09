use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use hanafubuki_agent_kernel::AgentHistoryStore as _;
use hanafubuki_agent_memory::{AgentMemoryContributionMode, AgentMemoryPaths};
use tempfile::TempDir;

fn lease() -> MemoryLease {
    MemoryLease {
        scope: "test".to_string(),
        token: "lease-token".to_string(),
        source_revision: 7,
        reset_epoch: 3,
        expires_at_millis: 100,
    }
}

fn repository() -> (TempDir, AgentMemoryRepository) {
    let temporary = TempDir::new().unwrap();
    let repository =
        AgentMemoryRepository::open(AgentMemoryPaths::under(temporary.path().join("memory")))
            .unwrap();
    (temporary, repository)
}

#[tokio::test]
async fn operation_heartbeats_until_completion() {
    let renewals = AtomicUsize::new(0);
    let operation_cancellation = CancellationToken::default();
    let result = run_lease_bound_operation(
        lease(),
        CancellationToken::default(),
        operation_cancellation.clone(),
        Duration::from_secs(1),
        Duration::from_millis(10),
        |lease, now| {
            renewals.fetch_add(1, Ordering::SeqCst);
            let mut renewed = lease.clone();
            renewed.expires_at_millis = now.saturating_add(100);
            Ok(renewed)
        },
        async {
            tokio::time::sleep(Duration::from_millis(45)).await;
            Ok::<_, String>("complete")
        },
    )
    .await
    .unwrap();

    assert_eq!(result.0, "complete");
    assert!(renewals.load(Ordering::SeqCst) >= 2);
    assert!(!operation_cancellation.is_cancelled());
}

#[tokio::test]
async fn lease_loss_cancels_the_operation() {
    let operation_cancellation = CancellationToken::default();
    let result = run_lease_bound_operation(
        lease(),
        CancellationToken::default(),
        operation_cancellation.clone(),
        Duration::from_secs(1),
        Duration::from_millis(5),
        |_lease, _now| Err(AgentMemoryError::LeaseLost),
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, String>(())
        },
    )
    .await;

    assert_eq!(result.unwrap_err().kind, LeaseBoundFailureKind::LeaseLost);
    assert!(operation_cancellation.is_cancelled());
}

#[tokio::test]
async fn renewal_failure_cancels_the_operation() {
    let operation_cancellation = CancellationToken::default();
    let result = run_lease_bound_operation(
        lease(),
        CancellationToken::default(),
        operation_cancellation.clone(),
        Duration::from_secs(1),
        Duration::from_millis(5),
        |_lease, _now| Err(AgentMemoryError::Io("database unavailable".to_string())),
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, String>(())
        },
    )
    .await;

    let failure = result.unwrap_err();
    assert_eq!(failure.kind, LeaseBoundFailureKind::Failed);
    assert!(failure.message.contains("lease renewal failed"));
    assert!(operation_cancellation.is_cancelled());
}

#[tokio::test]
async fn total_deadline_cancels_independently_of_heartbeat() {
    let operation_cancellation = CancellationToken::default();
    let result = run_lease_bound_operation(
        lease(),
        CancellationToken::default(),
        operation_cancellation.clone(),
        Duration::from_millis(20),
        Duration::from_secs(1),
        |_lease, _now| -> Result<MemoryLease, AgentMemoryError> {
            panic!("heartbeat must not run before the total deadline")
        },
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, String>(())
        },
    )
    .await;

    assert_eq!(
        result.unwrap_err().kind,
        LeaseBoundFailureKind::DeadlineExceeded
    );
    assert!(operation_cancellation.is_cancelled());
}

#[tokio::test]
async fn supervisor_cancellation_cancels_the_exact_operation() {
    let supervisor_cancellation = CancellationToken::default();
    let operation_cancellation = CancellationToken::default();
    let cancellation_trigger = supervisor_cancellation.clone();
    let trigger = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancellation_trigger.cancel();
    });
    let result = run_lease_bound_operation(
        lease(),
        supervisor_cancellation,
        operation_cancellation.clone(),
        Duration::from_secs(1),
        Duration::from_secs(1),
        |_lease, _now| -> Result<MemoryLease, AgentMemoryError> {
            panic!("heartbeat must not run before cancellation")
        },
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, String>(())
        },
    )
    .await;
    trigger.await.unwrap();

    assert_eq!(result.unwrap_err().kind, LeaseBoundFailureKind::Cancelled);
    assert!(operation_cancellation.is_cancelled());
}

#[test]
fn operation_failure_releases_source_and_consolidation_and_discards_staging() {
    let (_temporary, repository) = repository();
    let now = current_millis().unwrap();
    repository
        .sync_source_state("source", AgentMemoryContributionMode::Enabled, 1, 4)
        .unwrap();
    let source = repository
        .claim_source("source", 4, now, 60_000)
        .unwrap()
        .unwrap();
    fail_source_after_operation(
        &repository,
        LeaseBoundFailure::new(
            LeaseBoundFailureKind::Cancelled,
            "supervisor shutdown",
            source.clone(),
        ),
        &[],
    )
    .unwrap();
    assert_eq!(
        repository.complete_source(&source, None, now.saturating_add(1)),
        Err(AgentMemoryError::LeaseLost)
    );

    let consolidation = repository
        .claim_consolidation(now.saturating_add(2), 60_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(consolidation.source_revision + 1, &[])
        .unwrap();
    fail_consolidation_after_operation(
        &repository,
        LeaseBoundFailure::new(
            LeaseBoundFailureKind::DeadlineExceeded,
            "total deadline exceeded",
            consolidation.clone(),
        ),
        &workspace,
        &[],
    )
    .unwrap();
    assert!(!workspace.path.exists());
    assert_eq!(
        repository.publish_consolidation_workspace(
            &consolidation,
            &workspace,
            now.saturating_add(3),
        ),
        Err(AgentMemoryError::LeaseLost)
    );
}

#[test]
fn lease_loss_after_reset_still_discards_the_exact_staging_workspace() {
    let (_temporary, repository) = repository();
    let now = current_millis().unwrap();
    repository
        .sync_source_state("dirty", AgentMemoryContributionMode::Enabled, 1, 1)
        .unwrap();
    let consolidation = repository
        .claim_consolidation(now, 60_000)
        .unwrap()
        .unwrap();
    let workspace = repository
        .prepare_consolidation_workspace(consolidation.source_revision + 1, &[])
        .unwrap();
    repository.reset(0).unwrap();
    fail_consolidation_after_operation(
        &repository,
        LeaseBoundFailure::new(
            LeaseBoundFailureKind::LeaseLost,
            "lease lost",
            consolidation,
        ),
        &workspace,
        &[],
    )
    .unwrap();
    assert!(!workspace.path.exists());
    assert_eq!(repository.generation().unwrap(), 0);
}

/// A terminal, idle session at `revision` last updated at `updated_at`.
fn idle_session(
    store: &FileAgentHistoryStore,
    revision: u64,
    updated_at: u128,
) -> AgentMemoryHistoryCandidate {
    let session_id = store.allocate_session_id().unwrap();
    let turn = hanafubuki_agent_kernel::PersistedAgentTurn {
        id: hanafubuki_agent_kernel::AgentTurnId::new(1),
        generation: hanafubuki_agent_kernel::AgentGeneration::new(0),
        phase: hanafubuki_agent_kernel::AgentTurnPhase::Completed,
        sampling_step: 1,
        reasoning: Default::default(),
        final_answer_message_id: None,
        last_error: None,
        terminal_reason: None,
        created_at: updated_at - 10,
        updated_at,
        recovery: None,
        budget_ledger: Default::default(),
        no_progress: Default::default(),
        pending_steering: Vec::new(),
        next_steering_sequence: 0,
        last_consumed_steering_sequence: None,
        model_session_generation: hanafubuki_agent_kernel::AgentModelSessionGeneration::new(0),
        permission_mode: Default::default(),
    };
    let runtime = hanafubuki_agent_kernel::PersistedAgentRuntimeState {
        id: session_id,
        forked_from_session_id: None,
        parent_session_id: None,
        origin: None,
        cwd: None,
        branch: None,
        title: None,
        generation: hanafubuki_agent_kernel::AgentGeneration::new(0),
        revision: hanafubuki_agent_kernel::AgentRevision::new(revision),
        config: hanafubuki_agent_kernel::AgentSessionConfig {
            provider_id: "provider-1".to_string(),
            provider_protocol: hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            model: "fixture".to_string(),
            instructions: "test".to_string(),
            mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
            reasoning_effort: None,
            context_window_tokens: None,
            input_capabilities: Default::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: hanafubuki_agent_kernel::ModelCompactionCapability::LocalSummary,
            sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
        },
        memory_contribution: Default::default(),
        turn_budget_policy: hanafubuki_agent_kernel::AgentTurnBudgetPolicy::default(),
        lifecycle: hanafubuki_agent_kernel::AgentSessionLifecycle::Available,
        usage: None,
        context_window: hanafubuki_agent_kernel::AgentContextWindowSnapshot::initial(),
        compaction: None,
        turn: Some(turn),
        activities: Vec::new(),
        task_plan: None,
        pending_user_input: None,
        next_message_id: 1,
        next_history_item_id: 1,
        model_context_boundary: hanafubuki_agent_kernel::AgentModelContextBoundary::Full,
        history_base: None,
        created_at: updated_at - 10,
        updated_at,
    };
    store
        .create_session(hanafubuki_agent_kernel::AgentConversationCommit {
            runtime,
            history_mutations: Vec::new(),
        })
        .unwrap();
    AgentMemoryHistoryCandidate {
        session_id,
        source_revision: hanafubuki_agent_kernel::AgentRevision::new(revision),
        updated_at,
    }
}

#[test]
fn an_old_session_is_claimed_behind_pages_of_newer_extracted_ones() {
    let temporary = TempDir::new().unwrap();
    let workers = BackgroundWorkerGroup::default();
    let history = FileAgentHistoryStore::open(
        hanafubuki_agent_history::AgentHistoryPaths::under(temporary.path().join("agent")),
        workers.clone(),
    )
    .unwrap();
    let (_memory, repository) = repository();
    let oldest = idle_session(&history, 1, 1_800_000_000_000);
    let newer = (1..=5)
        .map(|index| idle_session(&history, 1, 1_800_000_000_000 + index * 1_000))
        .collect::<Vec<_>>();
    for candidate in newer.iter().chain([&oldest]) {
        repository
            .sync_source_state(
                &candidate.session_id.to_string(),
                AgentMemoryContributionMode::Enabled,
                1,
                1,
            )
            .unwrap();
    }
    for candidate in &newer {
        let lease = repository
            .claim_source(&candidate.session_id.to_string(), 1, 10, 1_000_000)
            .unwrap()
            .unwrap();
        repository.complete_source(&lease, None, 11).unwrap();
    }

    let mut sources = SourceClaims::new(
        &history,
        &repository,
        AgentMemoryHistoryQuery {
            current_session_id: None,
            now_millis: 1_800_000_100_000,
            minimum_idle_millis: 1_000,
            maximum_age_millis: 1_000_000,
            limit: 2,
            older_than: None,
        },
    );

    let (claimed, _lease) = sources.claim_next().unwrap().unwrap();
    assert_eq!(claimed, oldest);
    assert!(sources.claim_next().unwrap().is_none());
    history
        .shutdown(std::time::Instant::now() + Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(std::time::Instant::now() + Duration::from_secs(2))
        .is_empty());
}
