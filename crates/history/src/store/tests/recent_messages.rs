use super::*;
use hanafubuki_agent_kernel::{AgentSteeringSequence, RECENT_USER_MESSAGES_MAX};

fn with(session_id: AgentSessionId, id: u64, content: ConversationItem) -> AgentHistoryItem {
    AgentHistoryItem {
        source_session_id: session_id,
        content,
        ..item(id, 0, "")
    }
}

fn user(session_id: AgentSessionId, id: u64, text: &str) -> AgentHistoryItem {
    AgentHistoryItem {
        source_session_id: session_id,
        ..item(id, 0, text)
    }
}

fn create(
    store: &FileAgentHistoryStore,
    parent: Option<AgentSessionId>,
    items: impl FnOnce(AgentSessionId) -> Vec<AgentHistoryItem>,
) -> AgentSessionId {
    let session_id = store.allocate_session_id().unwrap();
    let items = items(session_id);
    let mut state = runtime(session_id, 0, items.len() as u64 + 1);
    state.parent_session_id = parent;
    store
        .create_session(AgentConversationCommit {
            runtime: state,
            history_mutations: items
                .into_iter()
                .map(|item| AgentHistoryMutation::AppendItem { item })
                .collect(),
        })
        .unwrap();
    session_id
}

#[test]
fn the_persons_messages_come_newest_first_each_once_bounded() {
    let (_temporary, _paths, workers, store) = open_store();
    let first = create(&store, None, |session| {
        vec![
            user(session, 1, "fix the parser"),
            with(
                session,
                2,
                ConversationItem::AssistantMessage {
                    message_id: hanafubuki_agent_kernel::AgentMessageId::new(1),
                    phase: None,
                    text: "an answer is not the person's".to_string(),
                    model_text: None,
                    proposed_plan: None,
                    sampling_step: 1,
                    memory_citations: Vec::new(),
                },
            ),
            user(session, 3, "run the tests"),
        ]
    });
    // A subagent's messages are its parent model's, not the person's.
    create(&store, Some(first), |session| {
        vec![user(session, 1, "a prompt the model wrote")]
    });
    std::thread::sleep(Duration::from_millis(5));
    create(&store, None, |session| {
        vec![
            user(session, 1, "fix the parser"),
            with(
                session,
                2,
                ConversationItem::SteeringMessage {
                    sequence: AgentSteeringSequence::new(1),
                    text: "and the lexer".to_string(),
                    images: Vec::new(),
                },
            ),
            with(
                session,
                3,
                ConversationItem::UserShellCommand {
                    call_id: "user-shell-1".to_string(),
                    command: "cargo test".to_string(),
                    cwd: None,
                    exit_code: Some(0),
                    duration_millis: 10,
                    output: "ok".to_string(),
                    truncated: false,
                    styled: None,
                },
            ),
            // An image alone has no text to find.
            user(session, 4, ""),
        ]
    });

    assert_eq!(
        store.recent_user_messages(10, None).unwrap(),
        [
            "!cargo test",
            "and the lexer",
            "fix the parser",
            "run the tests"
        ]
    );
    assert_eq!(
        store.recent_user_messages(2, None).unwrap(),
        ["!cargo test", "and the lexer"]
    );
    assert_eq!(
        store
            .recent_user_messages(RECENT_USER_MESSAGES_MAX + 1, Some(0))
            .unwrap(),
        Vec::<String>::new()
    );
    let later = 1_800_000_000_000 * 10;
    assert_eq!(
        store.recent_user_messages(10, Some(later)).unwrap().len(),
        4
    );

    store
        .shutdown(Instant::now() + Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn an_item_that_does_not_decode_is_left_out() {
    let (_temporary, _paths, workers, store) = open_store();
    let session = create(&store, None, |session| {
        vec![user(session, 1, "kept"), user(session, 2, "broken")]
    });
    {
        let _database = store.inner.database_gate.lock();
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE agent_history_items SET item_json = '{' WHERE session_id = ? AND item_id = 2",
                [session.value() as i64],
            )
            .unwrap();
    }
    assert_eq!(store.recent_user_messages(10, None).unwrap(), ["kept"]);

    store
        .shutdown(Instant::now() + Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
