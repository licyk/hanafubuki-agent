use super::*;

#[test]
fn cursors_are_bound_to_the_item_or_turn_scope_that_created_them() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    let mut first = item(1, 0, "turn one");
    first.turn_id = Some(AgentTurnId::new(1));
    let mut second = item(2, 0, "turn two");
    second.turn_id = Some(AgentTurnId::new(2));
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 3),
            history_mutations: vec![
                AgentHistoryMutation::AppendItem {
                    item: first.clone(),
                },
                AgentHistoryMutation::AppendItem { item: second },
            ],
        })
        .unwrap();
    let item_page = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 1,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    let error = store
        .list_turn_items_page(AgentTurnPageRequest {
            session_id,
            turn_id: AgentTurnId::new(1),
            page_size: 1,
            cursor: item_page.newer_cursor,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: Some(AgentRevision::new(0)),
        })
        .unwrap_err();
    assert!(matches!(
        error,
        hanafubuki_agent_kernel::AgentHistoryStoreError::InvalidCursor(_)
    ));
    let turn_page = store
        .list_turn_items_page(AgentTurnPageRequest {
            session_id,
            turn_id: AgentTurnId::new(1),
            page_size: 10,
            cursor: None,
            direction: AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    assert_eq!(turn_page.items, vec![first]);
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn bounded_pages_have_no_gaps_and_keep_the_original_revision_anchor() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 6),
            history_mutations: (1..=5)
                .map(|id| AgentHistoryMutation::AppendItem {
                    item: item(id, 0, &format!("item {id}")),
                })
                .collect(),
        })
        .unwrap();
    let first = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 2,
            cursor: None,
            direction: AgentHistorySortDirection::Descending,
            through_revision: None,
        })
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|item| item.item_id.value())
            .collect::<Vec<_>>(),
        vec![5, 4]
    );
    store
        .append_commit(AgentConversationCommit {
            runtime: runtime(session_id, 1, 7),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(6, 0, "late"),
            }],
        })
        .unwrap();
    let second = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 2,
            cursor: first.older_cursor,
            direction: AgentHistorySortDirection::Descending,
            through_revision: Some(first.through_revision),
        })
        .unwrap();
    let third = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 2,
            cursor: second.older_cursor,
            direction: AgentHistorySortDirection::Descending,
            through_revision: Some(first.through_revision),
        })
        .unwrap();
    assert_eq!(
        second
            .items
            .iter()
            .chain(&third.items)
            .map(|item| item.item_id.value())
            .collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    store.shutdown_session(session_id).unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
#[ignore = "repeatable 10k-session / 1m-item SQLite pagination scale check"]
fn scale_pages_remain_bounded_at_ten_thousand_sessions_and_one_million_items() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(session_id, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: item(1, 0, "fixture"),
            }],
        })
        .unwrap();
    store.shutdown_session(session_id).unwrap();
    {
        let _database = store.inner.database_gate.lock();
        let connection = store.connection().unwrap();
        connection
            .execute_batch(
                r#"
WITH RECURSIVE ids(value) AS (
    SELECT 2 UNION ALL SELECT value + 1 FROM ids WHERE value < 10000
)
INSERT INTO agent_sessions(
    session_id, rollout_path, created_at, updated_at, lifecycle, generation, revision,
    model, mode, title, recovery_needed, runtime_json, summary_json
)
SELECT value, printf('/tmp/scale-%d.jsonl', value), '1', printf('%020d', value),
       'available', 0, 0, 'fixture', '"default"', 'fixture', 0,
       (SELECT runtime_json FROM agent_sessions WHERE session_id = 1),
       (SELECT summary_json FROM agent_sessions WHERE session_id = 1)
FROM ids;
WITH RECURSIVE items(value) AS (
    SELECT 2 UNION ALL SELECT value + 1 FROM items WHERE value <= 1000000
)
INSERT INTO agent_history_items(
    session_id, item_id, first_record_ordinal, first_item_order, item_revision,
    created_at, item_type, item_json, session_revision
)
SELECT 1, value, value, value, 0, '1', 'user_message',
       (SELECT item_json FROM agent_history_items WHERE session_id = 1 AND item_id = 1), 0
FROM items;
                "#,
            )
            .unwrap();
    }
    let sessions = store
        .list_sessions_page(AgentSessionListRequest {
            page_size: 50,
            ..AgentSessionListRequest::default()
        })
        .unwrap();
    assert_eq!(sessions.sessions.len(), 50);
    assert!(sessions.next_cursor.is_some());
    let items = store
        .list_items_page(AgentTranscriptPageRequest {
            session_id,
            page_size: 100,
            cursor: None,
            direction: AgentHistorySortDirection::Descending,
            through_revision: None,
        })
        .unwrap();
    assert_eq!(items.items.len(), 100);
    assert!(items.older_cursor.is_some());
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
