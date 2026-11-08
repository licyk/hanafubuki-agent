use super::*;

#[test]
fn compaction_boundary_changes_model_context_without_removing_display_history() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 3),
            history_mutations: vec![
                AgentHistoryMutation::AppendItem {
                    item: item(1, 0, "old display item"),
                },
                AgentHistoryMutation::AppendItem {
                    item: item(2, 0, "retained tail"),
                },
            ],
        })
        .unwrap();
    let mut compacted = runtime(session_id, 1, 6);
    let source_window = compacted.context_window;
    let target_window = source_window.next().unwrap();
    let snapshot = AgentCompactionSnapshot {
        id: AgentCompactionId::new(1),
        generation: AgentCompactionGeneration::new(1),
        trigger: AgentCompactionTrigger::Manual,
        phase: AgentCompactionPhase::StandaloneTurn,
        reason: AgentCompactionReason::UserRequested,
        implementation: AgentCompactionImplementation::LocalSummary,
        status: AgentCompactionStatus::Completed,
        source_revision: AgentRevision::new(0),
        source_window_id: source_window.window_id,
        target_window,
        turn_generation: None,
        summary: Some("summary".to_string()),
        usage: None,
        last_error: None,
        created_at: 1,
        updated_at: 2,
    };
    compacted.context_window = target_window;
    compacted.compaction = Some(snapshot.clone());
    compacted.model_context_boundary = AgentModelContextBoundary::Compacted {
        checkpoint: AgentCompactionCheckpoint {
            checkpoint_item_id: AgentHistoryItemId::new(3),
            snapshot,
            replacement_history: vec![
                item(4, 0, "retained user"),
                item(5, 0, "summary replacement"),
            ],
            window_chain: vec![source_window, target_window],
            future_items_after_item_id: 5,
            initial_context_digest: [7; 32],
        },
    };
    let encoded_runtime = serde_json::to_string(&compacted).unwrap();
    serde_json::from_str::<hanafubuki_agent_kernel::PersistedAgentRuntimeState>(&encoded_runtime)
        .unwrap();
    store
        .append_commit(AgentConversationCommit {
            runtime: compacted.clone(),
            history_mutations: vec![
                AgentHistoryMutation::AppendItem {
                    item: item(3, 0, "summary"),
                },
                AgentHistoryMutation::SetModelContextBoundary {
                    boundary: compacted.model_context_boundary,
                },
            ],
        })
        .unwrap();
    let display = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    assert_eq!(display.items.len(), 3);
    assert_eq!(
        store
            .load_latest_model_context(session_id)
            .unwrap()
            .iter()
            .map(|item| item.item_id.value())
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn retention_is_disabled_by_default_and_never_selects_a_referenced_parent() {
    let (_temporary, _paths, workers, store) = open_store();
    let parent = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(parent, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: turn_start(1, 1),
            }],
        })
        .unwrap();
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(parent, 1, 3),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: turn_start(2, 2),
            }],
        })
        .unwrap();
    let child = store.allocate_session_id().unwrap();
    let mut child_runtime = runtime(child, 0, 1);
    child_runtime.forked_from_session_id = Some(parent);
    store
        .create_fork(
            parent,
            AgentTurnId::new(2),
            AgentConversationCommit {
                runtime: child_runtime,
                history_mutations: Vec::new(),
            },
        )
        .unwrap();
    let report = store
        .retention_report(&AgentHistoryRetentionPolicy {
            maximum_total_bytes: Some(0),
            ..AgentHistoryRetentionPolicy::default()
        })
        .unwrap();
    assert!(!report.automatic_deletion_enabled);
    assert!(!report
        .candidates
        .iter()
        .any(|candidate| candidate.session_id == parent));
    assert!(report
        .candidates
        .iter()
        .any(|candidate| candidate.session_id == child));
    store.shutdown_session(parent).unwrap();
    store.shutdown_session(child).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
