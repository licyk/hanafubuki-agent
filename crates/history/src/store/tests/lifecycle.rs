use super::*;

#[test]
fn archive_and_restore_reconcile_committed_lifecycle_across_restart() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "lifecycle"),
            }],
        })
        .unwrap();
    let mut archived = runtime(session_id, 1, 2);
    archived.lifecycle = AgentSessionLifecycle::Archived;
    store
        .append_commit(AgentConversationCommit {
            runtime: archived,
            history_mutations: Vec::new(),
        })
        .unwrap();
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);

    let archived_workers = BackgroundWorkerGroup::default();
    let archived_store =
        Arc::new(FileAgentHistoryStore::open(paths.clone(), archived_workers.clone()).unwrap());
    let mut archived_files = Vec::new();
    collect_jsonl_files(&paths.archived_sessions, &mut archived_files).unwrap();
    assert_eq!(archived_files.len(), 1);
    let mut available = runtime(session_id, 2, 2);
    available.lifecycle = AgentSessionLifecycle::Available;
    archived_store
        .append_commit(AgentConversationCommit {
            runtime: available,
            history_mutations: Vec::new(),
        })
        .unwrap();
    archived_store.shutdown_session(session_id).unwrap();
    assert!(archived_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(archived_store);

    let restored_workers = BackgroundWorkerGroup::default();
    let restored = FileAgentHistoryStore::open(paths.clone(), restored_workers.clone()).unwrap();
    let mut active_files = Vec::new();
    collect_jsonl_files(&paths.active_sessions, &mut active_files).unwrap();
    assert_eq!(active_files.len(), 1);
    assert!(collect_jsonl_files(&paths.archived_sessions, &mut Vec::new()).is_ok());
    assert_eq!(
        restored.load_runtime_state(session_id).unwrap().lifecycle,
        AgentSessionLifecycle::Available
    );
    assert!(restored_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn lineage_pages_share_parent_records_and_require_closed_batch_delete() {
    let (_temporary, _paths, workers, store) = open_store();
    let parent = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(parent, 0, 3),
            history_mutations: vec![
                AgentHistoryMutation::AppendItem {
                    item: turn_start(1, 1),
                },
                AgentHistoryMutation::AppendItem {
                    item: item(2, 0, "parent"),
                },
            ],
        })
        .unwrap();
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(parent, 1, 4),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: turn_start(3, 2),
            }],
        })
        .unwrap();

    let child = store.allocate_session_id().unwrap();
    let mut child_runtime = runtime(child, 0, 3);
    child_runtime.forked_from_session_id = Some(parent);
    store
        .create_fork(
            parent,
            AgentTurnId::new(2),
            AgentConversationCommit {
                runtime: child_runtime,
                history_mutations: vec![AgentHistoryMutation::AppendItem {
                    item: item(2, 0, "child"),
                }],
            },
        )
        .unwrap();
    let page = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id: child,
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .filter_map(|item| match &item.content {
                ConversationItem::UserMessage { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["parent", "child"]
    );
    let duplicate_local_ids = page
        .items
        .iter()
        .filter(|item| item.item_id == AgentHistoryItemId::new(2))
        .map(|item| item.source_session_id)
        .collect::<Vec<_>>();
    assert_eq!(duplicate_local_ids, vec![parent, child]);
    for refused in [
        store.check_sessions_deletable(&[parent]),
        store.delete_sessions_confirmed(&[parent]),
    ] {
        assert!(matches!(
            refused,
            Err(hanafubuki_agent_kernel::AgentHistoryStoreError::ReferencedByFork { session_id })
                if session_id == parent
        ));
    }
    store.check_sessions_deletable(&[child, parent]).unwrap();
    store.delete_sessions_confirmed(&[child, parent]).unwrap();
    assert!(matches!(
        store.load_runtime_state(parent),
        Err(hanafubuki_agent_kernel::AgentHistoryStoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.load_runtime_state(child),
        Err(hanafubuki_agent_kernel::AgentHistoryStoreError::NotFound { .. })
    ));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn committed_head_fork_freezes_the_exact_parent_cutoff_without_copying_jsonl() {
    let (_temporary, _paths, workers, store) = open_store();
    let parent = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(parent, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "parent-before-fork"),
            }],
        })
        .unwrap();
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(parent, 1, 3),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(2, 1, "parent-head"),
            }],
        })
        .unwrap();

    let stale_child = store.allocate_session_id().unwrap();
    assert_eq!(
        store
            .create_fork_at_head(
                parent,
                AgentRevision::new(0),
                AgentConversationCommit {
                    runtime: runtime(stale_child, 0, 1),
                    history_mutations: Vec::new(),
                },
            )
            .unwrap_err(),
        hanafubuki_agent_kernel::AgentHistoryStoreError::StaleRevision
    );

    let child = store.allocate_session_id().unwrap();
    let mut child_runtime = runtime(child, 0, 4);
    child_runtime.forked_from_session_id = Some(parent);
    let base = store
        .create_fork_at_head(
            parent,
            AgentRevision::new(1),
            AgentConversationCommit {
                runtime: child_runtime,
                history_mutations: vec![AgentHistoryMutation::AppendItem {
                    item: item(3, 0, "child-local"),
                }],
            },
        )
        .unwrap();
    assert_eq!(base.session_id, parent);
    assert!(base.end_ordinal_exclusive > 0);
    assert!(base.end_byte_offset > 0);

    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(parent, 2, 4),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(3, 2, "parent-after-fork"),
            }],
        })
        .unwrap();
    let page = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id: child,
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    let visible = page
        .items
        .iter()
        .filter_map(|item| match &item.content {
            ConversationItem::UserMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec!["parent-before-fork", "parent-head", "child-local"]
    );

    let child_jsonl =
        fs::read_to_string(store.lookup_rollout_path(child).unwrap().unwrap()).unwrap();
    assert!(child_jsonl.contains("child-local"));
    assert!(!child_jsonl.contains("parent-before-fork"));
    assert!(!child_jsonl.contains("parent-after-fork"));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn startup_finishes_the_same_durable_delete_obligation() {
    let (_temporary, paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "delete me"),
            }],
        })
        .unwrap();
    store.shutdown_session(session_id).unwrap();
    let source = store.lookup_rollout_path(session_id).unwrap().unwrap();
    let obligation = super::DeleteObligation {
        version: 1,
        operation_id: "delete-fixture".to_string(),
        session_ids: vec![session_id],
        source_paths: vec![source.clone()],
        attachment_paths: vec![paths.attachments.join(session_id.value().to_string())],
    };
    super::write_delete_obligation(&paths.trash, &obligation).unwrap();
    let manifest = paths
        .trash
        .join(&obligation.operation_id)
        .join("obligation.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    raw["removedProgress"] = serde_json::json!(50);
    fs::write(&manifest, serde_json::to_vec(&raw).unwrap()).unwrap();
    let trash_path = paths
        .trash
        .join(&obligation.operation_id)
        .join(source.file_name().unwrap());
    fs::rename(&source, &trash_path).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
    drop(store);

    let recovered_workers = BackgroundWorkerGroup::default();
    let recovered = FileAgentHistoryStore::open(paths.clone(), recovered_workers.clone()).unwrap();
    assert!(matches!(
        recovered.load_runtime_state(session_id),
        Err(hanafubuki_agent_kernel::AgentHistoryStoreError::NotFound { .. })
    ));
    assert!(!paths.trash.join("delete-fixture").exists());
    assert!(recovered_workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn a_recovery_candidate_that_fails_to_load_is_its_own_entry_and_keeps_its_row() {
    let (_temporary, _paths, workers, store) = open_store();
    let mut sessions = Vec::new();
    for text in ["corrupt", "intact"] {
        let session_id = store.allocate_session_id().unwrap();
        store
            .create_session(AgentConversationCommit {
                runtime: runtime(session_id, 0, 2),
                history_mutations: vec![AgentHistoryMutation::AppendItem {
                    item: item(1, 0, text),
                }],
            })
            .unwrap();
        store.shutdown_session(session_id).unwrap();
        sessions.push(session_id);
    }
    let (corrupt, intact) = (sessions[0], sessions[1]);
    {
        let _database = store.inner.database_gate.lock();
        let connection = store.connection().unwrap();
        connection
            .execute("UPDATE agent_sessions SET recovery_needed = 1", [])
            .unwrap();
        connection
            .execute(
                "UPDATE agent_sessions SET runtime_json = '{not json' WHERE session_id = ?",
                [i64::try_from(corrupt.value()).unwrap()],
            )
            .unwrap();
    }

    let candidates = store.load_recovery_candidates().unwrap();
    assert_eq!(candidates.len(), 2);
    match &candidates[0] {
        Err(failure) => assert_eq!(failure.session_id, corrupt),
        Ok(session) => panic!("the corrupt session loaded: {:?}", session.runtime.id),
    }
    match &candidates[1] {
        Ok(session) => assert_eq!(session.runtime.id, intact),
        Err(failure) => panic!("the intact session failed: {:?}", failure.error),
    }
    let _database = store.inner.database_gate.lock();
    let kept: String = store
        .connection()
        .unwrap()
        .query_row(
            "SELECT runtime_json FROM agent_sessions WHERE session_id = ?",
            [i64::try_from(corrupt.value()).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept, "{not json");
    drop(_database);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
