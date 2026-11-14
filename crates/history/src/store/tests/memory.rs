use super::*;
use crate::{AgentMemoryEvidenceRole, AgentMemoryHistoryQuery};

fn terminal_runtime(
    session_id: AgentSessionId,
    revision: u64,
    next_history_item_id: u64,
) -> PersistedAgentRuntimeState {
    let mut state = runtime(session_id, revision, next_history_item_id);
    state.updated_at = 1_800_000_000_000;
    state.turn = Some(hanafubuki_agent_kernel::PersistedAgentTurn {
        id: AgentTurnId::new(1),
        generation: AgentGeneration::new(0),
        phase: hanafubuki_agent_kernel::AgentTurnPhase::Completed,
        sampling_step: 1,
        reasoning: Default::default(),
        final_answer_message_id: None,
        last_error: None,
        terminal_reason: None,
        created_at: state.updated_at - 10,
        updated_at: state.updated_at,
        recovery: None,
        budget_ledger: Default::default(),
        no_progress: Default::default(),
        pending_steering: Vec::new(),
        next_steering_sequence: 0,
        last_consumed_steering_sequence: None,
        model_session_generation: hanafubuki_agent_kernel::AgentModelSessionGeneration::new(0),
        permission_mode: Default::default(),
    });
    state
}

#[test]
fn memory_candidates_require_terminal_synced_available_sessions() {
    let (_temporary, _paths, workers, store) = open_store();
    let terminal = store.allocate_session_id().unwrap();
    let active = store.allocate_session_id().unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: terminal_runtime(terminal, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: AgentHistoryItem {
                    source_session_id: terminal,
                    ..item(1, 0, "remember this")
                },
            }],
        })
        .unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime: runtime(active, 0, 2),
            history_mutations: vec![AgentHistoryMutation::AppendItem {
                item: AgentHistoryItem {
                    source_session_id: active,
                    ..item(1, 0, "still active")
                },
            }],
        })
        .unwrap();

    let candidates = store
        .list_memory_candidates(AgentMemoryHistoryQuery {
            current_session_id: Some(active),
            now_millis: 1_800_000_100_000,
            minimum_idle_millis: 1_000,
            maximum_age_millis: 1_000_000,
            limit: 10,
            older_than: None,
        })
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].session_id, terminal);

    store
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn memory_candidates_are_newest_first_and_page_after_a_candidate() {
    let (_temporary, _paths, workers, store) = open_store();
    let older = store.allocate_session_id().unwrap();
    let tied_low = store.allocate_session_id().unwrap();
    let tied_high = store.allocate_session_id().unwrap();
    for (session_id, updated_at) in [
        (older, 1_800_000_000_000),
        (tied_low, 1_800_000_010_000),
        (tied_high, 1_800_000_010_000),
    ] {
        let mut state = terminal_runtime(session_id, 0, 1);
        state.updated_at = updated_at;
        state.turn.as_mut().unwrap().updated_at = updated_at;
        store
            .create_session(AgentConversationCommit {
                runtime: state,
                history_mutations: Vec::new(),
            })
            .unwrap();
    }
    let page = |limit, older_than| {
        store
            .list_memory_candidates(AgentMemoryHistoryQuery {
                current_session_id: None,
                now_millis: 1_800_000_100_000,
                minimum_idle_millis: 1_000,
                maximum_age_millis: 1_000_000,
                limit,
                older_than,
            })
            .unwrap()
    };
    let ids = |candidates: &[crate::AgentMemoryHistoryCandidate]| {
        candidates
            .iter()
            .map(|candidate| candidate.session_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&page(3, None)), vec![tied_high, tied_low, older]);

    // One at a time, each page continues after the last candidate, across
    // equal update times too.
    let mut paged = Vec::new();
    let mut older_than = None;
    loop {
        let next = page(1, older_than);
        let Some(candidate) = next.first().copied() else {
            break;
        };
        paged.push(candidate.session_id);
        older_than = Some(candidate);
    }
    assert_eq!(paged, vec![tied_high, tied_low, older]);
    store
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn memory_evidence_is_rebuilt_from_bounded_physical_jsonl_items() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    let mut user = item(1, 0, "original");
    user.source_session_id = session_id;
    let assistant = AgentHistoryItem {
        source_session_id: session_id,
        item_id: hanafubuki_agent_kernel::AgentHistoryItemId::new(2),
        item_revision: 0,
        turn_id: Some(AgentTurnId::new(1)),
        logical_segment: 0,
        logical_order: 1,
        turn_working_millis: None,
        content: ConversationItem::AssistantMessage {
            message_id: hanafubuki_agent_kernel::AgentMessageId::new(1),
            phase: Some(hanafubuki_agent_kernel::AgentMessagePhase::FinalAnswer),
            text: "durable answer".to_string(),
            model_text: None,
            proposed_plan: None,
            sampling_step: 1,
            memory_citations: Vec::new(),
        },
    };
    let tool_call = AgentHistoryItem {
        source_session_id: session_id,
        item_id: hanafubuki_agent_kernel::AgentHistoryItemId::new(3),
        item_revision: 0,
        turn_id: Some(AgentTurnId::new(1)),
        logical_segment: 0,
        logical_order: 2,
        turn_working_millis: None,
        content: serde_json::from_value(serde_json::json!({
            "type": "tool_call",
            "callId": "call-1",
            "name": "read_task_log",
            "catalogRevision": vec![0; 32],
            "providerId": "builtin",
            "toolId": "read_task_log",
            "toolGeneration": vec![0; 32],
            "arguments": {"taskId": "task-1", "token": "secret"}
        }))
        .unwrap(),
    };
    let tool_output = AgentHistoryItem {
        source_session_id: session_id,
        item_id: hanafubuki_agent_kernel::AgentHistoryItemId::new(4),
        item_revision: 0,
        turn_id: Some(AgentTurnId::new(1)),
        logical_segment: 0,
        logical_order: 3,
        turn_working_millis: None,
        content: serde_json::from_value(serde_json::json!({
            "type": "tool_output",
            "callId": "call-1",
            "output": {
                "content": [{"type": "text", "text": "completed"}],
                "structuredContent": null,
                "isError": false
            }
        }))
        .unwrap(),
    };
    store
        .create_session(AgentConversationCommit {
            runtime: terminal_runtime(session_id, 0, 5),
            history_mutations: vec![
                AgentHistoryMutation::AppendItem { item: user.clone() },
                AgentHistoryMutation::AppendItem {
                    item: assistant.clone(),
                },
                AgentHistoryMutation::AppendItem { item: tool_call },
                AgentHistoryMutation::AppendItem { item: tool_output },
            ],
        })
        .unwrap();
    user.item_revision = 1;
    if let ConversationItem::UserMessage { text, .. } = &mut user.content {
        *text = "updated user evidence".to_string();
    }
    store
        .append_commit(AgentConversationCommit {
            runtime: terminal_runtime(session_id, 1, 5),
            history_mutations: vec![
                AgentHistoryMutation::UpsertItemSnapshot { item: user },
                AgentHistoryMutation::SetModelContextBoundary {
                    boundary: AgentModelContextBoundary::Full,
                },
            ],
        })
        .unwrap();

    let evidence = store
        .load_memory_evidence(
            crate::AgentMemoryHistoryCandidate {
                session_id,
                source_revision: AgentRevision::new(1),
                updated_at: 1_800_000_000_000,
            },
            10,
            1_000,
        )
        .unwrap();
    assert_eq!(evidence.items.len(), 4);
    assert_eq!(evidence.items[0].role, AgentMemoryEvidenceRole::User);
    assert_eq!(evidence.items[0].text, "updated user evidence");
    assert_eq!(evidence.items[1].role, AgentMemoryEvidenceRole::Assistant);
    assert_eq!(evidence.items[1].text, "durable answer");
    assert_eq!(
        evidence.items[2].tool_name.as_deref(),
        Some("read_task_log")
    );
    assert_eq!(evidence.items[2].call_id.as_deref(), Some("call-1"));
    assert_eq!(evidence.items[3].call_id.as_deref(), Some("call-1"));
    assert!(evidence.items[3].text.contains("completed"));
    assert!(!evidence.truncated);

    store
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[test]
fn fork_memory_evidence_contains_only_child_local_jsonl_items() {
    let (_temporary, _paths, workers, store) = open_store();
    let parent = store.allocate_session_id().unwrap();
    let mut parent_item = item(2, 0, "parent evidence");
    parent_item.source_session_id = parent;
    store
        .create_session(AgentConversationCommit {
            runtime: terminal_runtime(parent, 0, 4),
            history_mutations: vec![
                AgentHistoryMutation::AppendItem {
                    item: turn_start(1, 1),
                },
                AgentHistoryMutation::AppendItem { item: parent_item },
                AgentHistoryMutation::AppendItem {
                    item: turn_start(3, 2),
                },
            ],
        })
        .unwrap();

    let child = store.allocate_session_id().unwrap();
    let mut child_runtime = terminal_runtime(child, 0, 2);
    child_runtime.forked_from_session_id = Some(parent);
    let mut child_item = item(1, 0, "child-local evidence");
    child_item.source_session_id = child;
    store
        .create_fork(
            parent,
            AgentTurnId::new(2),
            AgentConversationCommit {
                runtime: child_runtime,
                history_mutations: vec![AgentHistoryMutation::AppendItem { item: child_item }],
            },
        )
        .unwrap();

    let evidence = store
        .load_memory_evidence(
            crate::AgentMemoryHistoryCandidate {
                session_id: child,
                source_revision: AgentRevision::new(0),
                updated_at: 1_800_000_000_000,
            },
            10,
            1_000,
        )
        .unwrap();
    assert_eq!(evidence.items.len(), 1);
    assert_eq!(evidence.items[0].text, "child-local evidence");

    store
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

/// A `WakeEvents` item, written to and read back from the session's file,
/// is what background work reported: a call's output of that name.
#[test]
fn memory_evidence_reads_wake_events_as_a_background_events_output() {
    let (_temporary, _paths, workers, store) = open_store();
    let session_id = store.allocate_session_id().unwrap();
    let mut user = item(1, 0, "start the agents");
    user.source_session_id = session_id;
    let wake = AgentHistoryItem {
        source_session_id: session_id,
        item_id: hanafubuki_agent_kernel::AgentHistoryItemId::new(2),
        item_revision: 0,
        turn_id: Some(AgentTurnId::new(1)),
        logical_segment: 0,
        logical_order: 1,
        turn_working_millis: None,
        content: ConversationItem::WakeEvents {
            events: vec![hanafubuki_agent_kernel::AgentWakeEvent {
                sequence: 1,
                queued_at: 5,
                source: hanafubuki_agent_kernel::AgentWakeSource::Subagent {
                    session_id: AgentSessionId::new(41),
                    name: "explore_api".to_string(),
                    event: hanafubuki_agent_kernel::AgentSubagentWake::Completed {
                        answer: Some("three endpoints".to_string()),
                        truncated: false,
                    },
                },
            }],
        },
    };
    store
        .create_session(AgentConversationCommit {
            runtime: terminal_runtime(session_id, 0, 3),
            history_mutations: vec![
                AgentHistoryMutation::AppendItem { item: user },
                AgentHistoryMutation::AppendItem { item: wake },
            ],
        })
        .unwrap();

    let evidence = store
        .load_memory_evidence(
            crate::AgentMemoryHistoryCandidate {
                session_id,
                source_revision: AgentRevision::new(0),
                updated_at: 1_800_000_000_000,
            },
            10,
            10_000,
        )
        .unwrap();
    assert_eq!(evidence.items.len(), 2);
    let wake = &evidence.items[1];
    assert_eq!(wake.role, AgentMemoryEvidenceRole::Tool);
    assert_eq!(wake.tool_name.as_deref(), Some("background_events"));
    assert!(
        wake.text.starts_with("<background_events>"),
        "{}",
        wake.text
    );
    assert!(wake.text.contains("three endpoints"), "{}", wake.text);

    store
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(5))
        .into_iter()
        .for_each(drop);
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
