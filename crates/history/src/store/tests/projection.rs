use super::*;

#[test]
fn persisted_title_is_the_searchable_projection_authority() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    let mut titled = runtime(session_id, 0, 1);
    titled.title = Some(hanafubuki_agent_kernel::AgentSessionTitle::user("迁移实例日志").unwrap());
    store
        .create_session(AgentConversationCommit {
            runtime: titled,
            history_mutations: Vec::new(),
        })
        .unwrap();

    let page = store
        .list_sessions_page(AgentSessionListRequest {
            search: Some("实例".to_string()),
            ..AgentSessionListRequest::default()
        })
        .unwrap();
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(
        page.sessions[0].title.as_ref().unwrap().value,
        "迁移实例日志"
    );
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn jsonl_is_canonical_and_upserts_keep_a_stable_item_identity() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "first"),
            }],
        })
        .unwrap();
    let mut settled = item(1, 1, "updated");
    settled.turn_working_millis = Some(1_234);
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(session_id, 1, 2),
            history_mutations: vec![AgentHistoryMutation::UpsertItemSnapshot {
                item: settled.clone(),
            }],
        })
        .unwrap();

    let page = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    assert_eq!(page.items, vec![settled]);

    store.shutdown_session(session_id).unwrap();
    let mut files = Vec::new();
    collect_jsonl_files(&paths.active_sessions, &mut files).unwrap();
    let encoded = fs::read_to_string(&files[0]).unwrap();
    assert_eq!(encoded.lines().count(), 2);
    assert!(encoded.ends_with('\n'));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn deltas_advance_transcript_head_without_rewriting_the_runtime_checkpoint() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "checkpoint"),
            }],
        })
        .unwrap();
    store
        .append_delta(AgentConversationDelta {
            session_id,
            session_revision: AgentRevision::new(1),
            session_updated_at: 1_800_000_000_001,
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(2, 0, "delta"),
            }],
        })
        .unwrap();

    assert_eq!(
        store
            .load_runtime_state(session_id)
            .unwrap()
            .revision
            .value(),
        0
    );
    let page = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    assert_eq!(page.through_revision.value(), 1);
    let mut delta_item = item(2, 0, "delta");
    delta_item.logical_order = 65_536;
    assert_eq!(page.items, vec![item(1, 0, "checkpoint"), delta_item]);
    assert_eq!(
        store
            .list_sessions_page(AgentSessionListRequest::default())
            .unwrap()
            .sessions[0]
            .revision
            .value(),
        1
    );

    store.shutdown_session(session_id).unwrap();
    let mut files = Vec::new();
    collect_jsonl_files(&paths.active_sessions, &mut files).unwrap();
    let records = fs::read_to_string(&files[0]).unwrap();
    let delta: serde_json::Value = serde_json::from_str(records.lines().nth(1).unwrap()).unwrap();
    assert!(delta["payload"].get("runtimeCheckpoint").is_none());

    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);
    fs::remove_file(&paths.sqlite_database).unwrap();
    let rebuilt_workers = BackgroundWorkerGroup::default();
    let rebuilt = FileAgentHistoryStore::open(paths, rebuilt_workers.clone()).unwrap();
    assert_eq!(
        rebuilt
            .load_runtime_state(session_id)
            .unwrap()
            .revision
            .value(),
        0
    );
    assert_eq!(
        rebuilt
            .list_items_page(AgentTranscriptPageRequest {
                session_id,
                page_size: 10,
                cursor: None,
                direction: AgentHistorySortDirection::Ascending,
                through_revision: None,
            })
            .unwrap()
            .through_revision
            .value(),
        1
    );
    assert!(rebuilt_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn an_incomplete_tail_is_truncated_before_the_next_append() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "complete"),
            }],
        })
        .unwrap();
    store.shutdown_session(session_id).unwrap();
    let mut files = Vec::new();
    collect_jsonl_files(&paths.active_sessions, &mut files).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&files[0])
        .unwrap()
        .write_all(b"{\"partial\":")
        .unwrap();

    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(session_id, 1, 3),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(2, 0, "after repair"),
            }],
        })
        .unwrap();
    store.shutdown_session(session_id).unwrap();
    let encoded = fs::read_to_string(&files[0]).unwrap();
    assert_eq!(encoded.lines().count(), 2);
    assert!(!encoded.contains("partial"));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn deleting_sqlite_rebuilds_the_projection_from_jsonl() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "rebuild me"),
            }],
        })
        .unwrap();
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);
    fs::remove_file(&paths.sqlite_database).unwrap();

    let rebuilt_workers = BackgroundWorkerGroup::default();
    let rebuilt = FileAgentHistoryStore::open(paths, rebuilt_workers.clone()).unwrap();
    let sessions = rebuilt
        .list_sessions_page(AgentSessionListRequest {
            page_size: 10,
            cursor: None,
            lifecycle: AgentSessionLifecycle::Available,
            direction: AgentHistorySortDirection::Descending,
            search: None,
        })
        .unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].session_id, session_id);
    assert_eq!(
        rebuilt.load_latest_model_context(session_id).unwrap(),
        vec![item(1, 0, "rebuild me")]
    );
    assert!(rebuilt_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn canonical_tool_output_truncation_survives_compaction_fork_and_sqlite_rebuild() {
    let (_temporary, paths, workers, store) = open_store();
    let parent = store.allocate_session_id().unwrap();
    let canonical = truncated_tool_output(parent, 1, 0);
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(parent, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: canonical.clone(),
            }],
        })
        .unwrap();

    let mut compacted = runtime(parent, 1, 2);
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
        summary: Some("large tool output retained".to_string()),
        usage: None,
        last_error: None,
        created_at: 1,
        updated_at: 2,
    };
    compacted.context_window = target_window;
    compacted.compaction = Some(snapshot.clone());
    compacted.model_context_boundary = AgentModelContextBoundary::Compacted {
        checkpoint: AgentCompactionCheckpoint {
            checkpoint_item_id: AgentHistoryItemId::new(1),
            snapshot,
            replacement_history: vec![canonical.clone()],
            window_chain: vec![source_window, target_window],
            future_items_after_item_id: 1,
            initial_context_digest: [9; 32],
        },
    };
    store
        .append_commit(AgentConversationCommit {
            runtime: compacted,
            history_mutations: vec![AgentHistoryMutation::SetModelContextBoundary {
                boundary: AgentModelContextBoundary::Compacted {
                    checkpoint: AgentCompactionCheckpoint {
                        checkpoint_item_id: AgentHistoryItemId::new(1),
                        snapshot: AgentCompactionSnapshot {
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
                            summary: Some("large tool output retained".to_string()),
                            usage: None,
                            last_error: None,
                            created_at: 1,
                            updated_at: 2,
                        },
                        replacement_history: vec![canonical.clone()],
                        window_chain: vec![source_window, target_window],
                        future_items_after_item_id: 1,
                        initial_context_digest: [9; 32],
                    },
                },
            }],
        })
        .unwrap();

    let child = store.allocate_session_id().unwrap();
    let mut child_runtime = runtime(child, 0, 2);
    child_runtime.forked_from_session_id = Some(parent);
    store
        .create_fork_at_head(
            parent,
            AgentRevision::new(1),
            AgentConversationCommit {
                runtime: child_runtime,
                history_mutations: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(
        store.load_latest_model_context(parent).unwrap(),
        vec![canonical.clone()]
    );
    assert_eq!(
        store
            .list_items_page(AgentTranscriptPageRequest {
                session_id: child,
                page_size: 10,
                cursor: None,
                direction: AgentHistorySortDirection::Ascending,
                through_revision: None,
            })
            .unwrap()
            .items,
        vec![canonical.clone()]
    );

    store.shutdown_session(parent).unwrap();
    store.shutdown_session(child).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);
    fs::remove_file(&paths.sqlite_database).unwrap();

    let rebuilt_workers = BackgroundWorkerGroup::default();
    let rebuilt = FileAgentHistoryStore::open(paths, rebuilt_workers.clone()).unwrap();
    assert_eq!(
        rebuilt.load_latest_model_context(parent).unwrap(),
        vec![canonical.clone()]
    );
    assert_eq!(
        rebuilt
            .list_items_page(AgentTranscriptPageRequest {
                session_id: child,
                page_size: 10,
                cursor: None,
                direction: AgentHistorySortDirection::Ascending,
                through_revision: None,
            })
            .unwrap()
            .items,
        vec![canonical]
    );
    assert!(rebuilt_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn memory_contribution_modes_rebuild_from_canonical_jsonl() {
    let (_temporary, paths, workers, store) = open_store();
    let modes = [
        hanafubuki_agent_kernel::AgentMemoryContributionMode::Enabled,
        hanafubuki_agent_kernel::AgentMemoryContributionMode::Disabled,
        hanafubuki_agent_kernel::AgentMemoryContributionMode::Polluted,
    ];
    let mut expected = Vec::new();
    for (index, mode) in modes.into_iter().enumerate() {
        let session_id = store.allocate_session_id().unwrap();
        let mut state = runtime(session_id, index as u64, 1);
        state.memory_contribution = hanafubuki_agent_kernel::AgentMemoryContributionState {
            mode,
            revision: index as u64 + 3,
        };
        store
            .create_session(AgentConversationCommit {
                runtime: state,
                history_mutations: Vec::new(),
            })
            .unwrap();
        store.shutdown_session(session_id).unwrap();
        expected.push((session_id, mode, index as u64 + 3));
    }
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);
    fs::remove_file(&paths.sqlite_database).unwrap();

    let rebuilt_workers = BackgroundWorkerGroup::default();
    let rebuilt = FileAgentHistoryStore::open(paths, rebuilt_workers.clone()).unwrap();
    let states = rebuilt.list_memory_source_states().unwrap();
    assert_eq!(states.len(), expected.len());
    for (state, (session_id, mode, revision)) in states.iter().zip(expected) {
        assert_eq!(state.session_id, session_id);
        assert_eq!(state.contribution.mode, mode);
        assert_eq!(state.contribution.revision, revision);
    }
    assert!(rebuilt_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn incompatible_sqlite_schema_is_discarded_and_rebuilt_from_jsonl() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "survives index reset"),
            }],
        })
        .unwrap();
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);
    let connection = rusqlite::Connection::open(&paths.sqlite_database).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    let recovery = paths.root.join(".recovery");
    fs::create_dir(&recovery).unwrap();
    fs::write(recovery.join("index.sqlite3.1.1.invalid"), "old index").unwrap();

    let rebuilt_workers = BackgroundWorkerGroup::default();
    let rebuilt = FileAgentHistoryStore::open(paths.clone(), rebuilt_workers.clone()).unwrap();
    assert_eq!(
        rebuilt.load_latest_model_context(session_id).unwrap(),
        vec![item(1, 0, "survives index reset")]
    );
    assert!(recovery.join("index.sqlite3.1.1.invalid").is_file());
    assert!(rebuilt_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn a_corrupt_rollout_does_not_block_rebuilding_other_sessions() {
    let (_temporary, paths, workers, store) = open_store();
    let corrupt = store.allocate_session_id().unwrap();
    let healthy = store.allocate_session_id().unwrap();
    for (session_id, text) in [(corrupt, "corrupt"), (healthy, "healthy")] {
        store
            .create_session(AgentConversationCommit {
                runtime: runtime(session_id, 0, 2),
                history_mutations: vec![AgentHistoryMutation::AppendItem {
                    item: item(1, 0, text),
                }],
            })
            .unwrap();
        store.shutdown_session(session_id).unwrap();
    }
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);
    let mut files = Vec::new();
    collect_jsonl_files(&paths.active_sessions, &mut files).unwrap();
    let corrupt_path = files
        .iter()
        .find(|path| {
            path.file_stem()
                .unwrap()
                .to_string_lossy()
                .ends_with(&format!("-{corrupt}"))
        })
        .unwrap();
    fs::write(corrupt_path, b"{not-json}\n").unwrap();
    fs::remove_file(&paths.sqlite_database).unwrap();

    let rebuilt_workers = BackgroundWorkerGroup::default();
    let rebuilt = FileAgentHistoryStore::open(paths, rebuilt_workers.clone()).unwrap();
    let mut expected_healthy = item(1, 0, "healthy");
    expected_healthy.source_session_id = healthy;
    assert_eq!(
        rebuilt.load_latest_model_context(healthy).unwrap(),
        vec![expected_healthy]
    );
    assert!(matches!(
        rebuilt.list_items_page(AgentTranscriptPageRequest {
            session_id: corrupt,
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        }),
        Err(hanafubuki_agent_kernel::AgentHistoryStoreError::ProjectionUnavailable(_))
    ));
    assert!(rebuilt_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
