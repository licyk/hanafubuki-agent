use super::*;

#[test]
fn startup_marks_inflight_compaction_failed_without_replacing_display_history() {
    let session_id = AgentSessionId::new(8);
    let turn_id = AgentTurnId::new(2);
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let context_window = hanafubuki_agent_kernel::AgentContextWindowSnapshot::initial();
    let target_window = context_window.next().unwrap();
    let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
        serde_json::from_value(json!({
            "id": session_id,
            "forkedFromSessionId": null,
            "generation": 2,
            "revision": 9,
            "config": {
                "providerId": "provider-1",
                "providerProtocol": "responses",
                "model": "fixture",
                "instructions": "test",
                "skillSnapshot": {"catalogRevision": 0, "entries": []},
                "mode": "default",
                "reasoningEffort": null,
                "inputCapabilities": {"imageInput": "auto"},
                "contextWindowTokens": null,
                "autoCompactionEnabled": true,
                "autoCompactThresholdPercent": 80,
                "compactionCapability": "local_summary",
                "samplingRetryPolicy": {
                    "maximumRetries": 2,
                    "initialBackoffMillis": 100,
                    "maximumBackoffMillis": 2000
                }
            },
            "turnBudgetPolicy": hanafubuki_agent_kernel::AgentTurnBudgetPolicy::default(),
            "lifecycle": "available",
            "usage": null,
            "contextWindow": context_window,
            "compaction": {
                "id": 5,
                "generation": 2,
                "trigger": "manual",
                "phase": "standalone_turn",
                "reason": "user_requested",
                "implementation": "local_summary",
                "status": "running",
                "sourceRevision": 8,
                "sourceWindowId": context_window.window_id,
                "targetWindow": target_window,
                "turnGeneration": null,
                "summary": null,
                "usage": null,
                "lastError": null,
                "createdAt": 10,
                "updatedAt": 11
            },
            "turn": null,
            "activities": [],
            "taskPlan": null,
            "pendingUserInput": null,
            "nextMessageId": 2,
            "nextHistoryItemId": 4,
            "modelContextBoundary": {"type": "full"},
            "historyBase": null,
            "createdAt": 1,
            "updatedAt": 11
        }))
        .unwrap();
    let contents = [
        json!({"type": "turn_start", "turnId": turn_id}),
        json!({"type": "user_message", "text": "keep this exact history"}),
        json!({
            "type": "assistant_message",
            "messageId": 1,
            "phase": "final_answer",
            "text": "still original",
            "samplingStep": 1
        }),
    ];
    let original = contents
        .into_iter()
        .enumerate()
        .map(|(index, content)| AgentHistoryItem {
            source_session_id: session_id,
            item_id: hanafubuki_agent_kernel::AgentHistoryItemId::new(index as u64 + 1),
            item_revision: 0,
            turn_id: Some(turn_id),
            logical_segment: 0,
            logical_order: index as u64,
            turn_working_millis: None,
            content: serde_json::from_value(content).unwrap(),
        })
        .collect::<Vec<_>>();
    store
        .create_session(AgentConversationCommit {
            runtime,
            history_mutations: original
                .iter()
                .cloned()
                .map(|item| AgentHistoryMutation::AppendItem { item })
                .collect(),
        })
        .unwrap();
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );

    assert_eq!(service.recover_sessions().unwrap().restored, 1);
    let compaction = service.snapshot(session_id).unwrap().compaction.unwrap();
    assert_eq!(
        compaction.status,
        hanafubuki_agent_kernel::AgentCompactionStatus::Failed
    );
    assert!(compaction.last_error.unwrap().contains("interrupted"));
    assert_eq!(
        store.load_latest_model_context(session_id).unwrap(),
        original
    );
}

#[test]
fn completed_compaction_with_backfilled_prefill_tokens_reloads() {
    let session_id = AgentSessionId::new(9);
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let source_window = hanafubuki_agent_kernel::AgentContextWindowSnapshot::initial();
    let target_window = source_window.next().unwrap();
    let compaction = hanafubuki_agent_kernel::AgentCompactionSnapshot {
        id: hanafubuki_agent_kernel::AgentCompactionId::new(1),
        generation: hanafubuki_agent_kernel::AgentCompactionGeneration::new(1),
        trigger: hanafubuki_agent_kernel::AgentCompactionTrigger::Manual,
        phase: hanafubuki_agent_kernel::AgentCompactionPhase::StandaloneTurn,
        reason: hanafubuki_agent_kernel::AgentCompactionReason::UserRequested,
        implementation: hanafubuki_agent_kernel::AgentCompactionImplementation::LocalSummary,
        status: hanafubuki_agent_kernel::AgentCompactionStatus::Completed,
        source_revision: hanafubuki_agent_kernel::AgentRevision::new(1311),
        source_window_id: source_window.window_id,
        target_window,
        turn_generation: None,
        summary: Some("compacted".to_string()),
        usage: None,
        last_error: None,
        created_at: 100,
        updated_at: 101,
    };
    let snapshot_json = serde_json::to_value(&compaction).unwrap();
    let mut live_window = target_window;
    live_window.prefill_input_tokens = Some(1035);
    let replacement_item = AgentHistoryItem {
        source_session_id: session_id,
        item_id: hanafubuki_agent_kernel::AgentHistoryItemId::new(1),
        item_revision: 0,
        turn_id: None,
        logical_segment: 0,
        logical_order: 0,
        turn_working_millis: None,
        content: ConversationItem::CompactionSummary {
            summary: "compacted".to_string(),
            source_revision: hanafubuki_agent_kernel::AgentRevision::new(1311),
            compacted_item_count: 4,
        },
    };
    let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
        serde_json::from_value(json!({
            "id": session_id,
            "forkedFromSessionId": null,
            "generation": 4,
            "revision": 1559,
            "config": {
                "providerId": "provider-1",
                "providerProtocol": "responses",
                "model": "fixture",
                "instructions": "test",
                "skillSnapshot": {"catalogRevision": 0, "entries": []},
                "mode": "default",
                "reasoningEffort": null,
                "inputCapabilities": {"imageInput": "auto"},
                "contextWindowTokens": null,
                "autoCompactionEnabled": true,
                "autoCompactThresholdPercent": 80,
                "compactionCapability": "local_summary",
                "samplingRetryPolicy": {
                    "maximumRetries": 2,
                    "initialBackoffMillis": 100,
                    "maximumBackoffMillis": 2000
                }
            },
            "turnBudgetPolicy": hanafubuki_agent_kernel::AgentTurnBudgetPolicy::default(),
            "lifecycle": "available",
            "usage": null,
            "contextWindow": live_window,
            "compaction": snapshot_json,
            "turn": null,
            "activities": [],
            "taskPlan": null,
            "pendingUserInput": null,
            "nextMessageId": 2,
            "nextHistoryItemId": 2,
            "modelContextBoundary": {
                "type": "compacted",
                "checkpoint": {
                    "checkpointItemId": 2,
                    "snapshot": snapshot_json,
                    "replacementHistory": [replacement_item],
                    "windowChain": [source_window, target_window],
                    "futureItemsAfterItemId": 2,
                    "initialContextDigest": serde_json::to_value([7u8; 32]).unwrap()
                }
            },
            "historyBase": null,
            "createdAt": 1,
            "updatedAt": 101
        }))
        .unwrap();
    store
        .create_session(AgentConversationCommit {
            runtime,
            history_mutations: vec![],
        })
        .unwrap();
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );

    assert_eq!(service.recover_sessions().unwrap().restored, 1);
    let snapshot = service.snapshot(session_id).unwrap();
    assert_eq!(snapshot.context_window.prefill_input_tokens, Some(1035));
    assert_eq!(
        snapshot.compaction.unwrap().status,
        hanafubuki_agent_kernel::AgentCompactionStatus::Completed
    );
}
