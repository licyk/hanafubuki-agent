use super::*;

#[tokio::test]
async fn request_retry_exhaustion_reason_survives_durable_runtime_reload() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            provider_id: "provider-1".to_string(),
            provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
            model: "fixture".to_string(),
            instructions: "test".to_string(),
            mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
            reasoning_effort: None,
            context_window_tokens: None,
            input_capabilities: Default::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: ModelCompactionCapability::LocalSummary,
            sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
        })
        .unwrap();

    assert!(matches!(
        service.start_turn(session.id, "hello".to_string()).await,
        Err(hanafubuki_agent_kernel::AgentError::ModelRequestRetriesExhausted { attempts: 3, .. })
    ));
    let persisted = store.load_runtime_state(session.id).unwrap();
    let turn = persisted.turn.unwrap();
    assert_eq!(turn.phase, hanafubuki_agent_kernel::AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(hanafubuki_agent_kernel::AgentTurnTerminalReason::ModelRequestRetriesExhausted)
    );
}

#[test]
fn activated_session_continues_activity_ids_after_the_event_journal_resets() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let created = service
        .create_model_session(
            text_session_config(),
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            ModelCompactionCapability::LocalSummary,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap();
    service
        .report_session_execution_progress(
            created.id,
            AgentToolProgressUpdate::ExecutionStarted {
                execution_id: 7,
                generation: 1,
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            },
        )
        .unwrap();
    service
        .report_session_execution_progress(
            created.id,
            AgentToolProgressUpdate::ExecutionDisposed {
                execution_id: 7,
                generation: 1,
            },
        )
        .unwrap();
    drop(service);

    let activated = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    activated.activate_session(created.id).unwrap();
    activated
        .report_session_execution_progress(
            created.id,
            AgentToolProgressUpdate::ExecutionStarted {
                execution_id: 8,
                generation: 1,
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            },
        )
        .unwrap();
    activated
        .report_session_execution_progress(
            created.id,
            AgentToolProgressUpdate::ExecutionDisposed {
                execution_id: 8,
                generation: 1,
            },
        )
        .unwrap();

    let runtime = store.load_runtime_state(created.id).unwrap();
    let ids = runtime
        .activities
        .iter()
        .map(|activity| activity.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), runtime.activities.len());
    assert_eq!(runtime.activities.last().unwrap().id.value(), 2);
}

#[tokio::test]
async fn opening_during_a_transient_stream_event_uses_the_durable_transcript_revision() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(ResponseStartedThenErrorModel),
        AgentKernelRegistries::default(),
        store,
    );
    let session = service.create_session(text_session_config()).unwrap();
    let sink = Arc::new(OpenSessionOnResponseStartedSink {
        service: service.clone(),
        session_id: session.id,
        opened: Mutex::new(None),
    });

    assert!(service
        .start_turn_with_sink(session.id, "hello", sink.clone())
        .await
        .is_err());

    let opened = sink
        .opened
        .lock()
        .unwrap()
        .take()
        .expect("response-start event must open the live session")
        .expect("live session open must use a durable transcript boundary");
    assert!(opened.runtime.revision.value() > opened.transcript.through_revision.value());
    assert_eq!(opened.through_revision, opened.transcript.through_revision);
}

#[test]
fn startup_aborts_unfinished_review_and_pairs_committed_tool_call_without_replay() {
    let session_id = AgentSessionId::new(7);
    let turn_id = AgentTurnId::new(9);
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let context_window = hanafubuki_agent_kernel::AgentContextWindowSnapshot::initial();
    let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
        serde_json::from_value(json!({
            "id": session_id,
            "forkedFromSessionId": null,
            "generation": 4,
            "revision": 12,
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
            "compaction": null,
            "turn": {
                "id": turn_id,
                "generation": 4,
                "phase": "running_tools",
                "samplingStep": 1,
                "reasoning": {
                    "snapshot": {
                        "samplingStep": 1,
                        "phase": "thinking",
                        "activeSince": 10,
                        "stepCompletedMillis": 7,
                        "turnCompletedMillis": 17,
                        "lastCompletedAt": 8
                    },
                    "activeReasoningItemCount": 1
                },
                "finalAnswerMessageId": null,
                "lastError": null,
                "createdAt": 10,
                "updatedAt": 11,
                "recovery": null,
                "budgetLedger": {
                    "samplingAdmissions": 1,
                    "cumulativeModelInputTokens": 0,
                    "cumulativeModelOutputTokens": 0,
                    "toolInvocations": 1,
                    "toolOutputBytes": 0,
                    "hookFragmentBytes": 0,
                    "pendingInputItems": 1,
                    "compactions": 0
                },
                "noProgress": hanafubuki_agent_kernel::AgentNoProgressState::default(),
                "pendingSteering": [],
                "nextSteeringSequence": 1,
                "lastConsumedSteeringSequence": null,
                "modelSessionGeneration": 1,
                "permissionMode": "manual"
            },
            "activities": [],
            "taskPlan": {
                "turnId": turn_id,
                "generation": 4,
                "revision": 2,
                "explanation": "Waiting for the tool",
                "plan": [
                    {"step": "Start the instance", "status": "in_progress"}
                ],
                "updatedAt": 11
            },
            "pendingUserInput": {
                "requestId": 5,
                "callId": "question-1",
                "requestedAt": 9,
                "questions": [{
                    "id": "scope",
                    "header": "Scope",
                    "question": "Choose a scope",
                    "options": [
                        {"id": "focused", "label": "Focused", "description": "Only this feature"},
                        {"id": "broad", "label": "Broad", "description": "Include cleanup"}
                    ]
                }]
            },
            "nextMessageId": 1,
            "nextHistoryItemId": 5,
            "modelContextBoundary": {"type": "full"},
            "historyBase": null,
            "createdAt": 1,
            "updatedAt": 11
        }))
        .unwrap();
    let contents = [
        json!({"type": "turn_start", "turnId": turn_id}),
        json!({"type": "user_message", "text": "perform it"}),
        json!({
            "type": "tool_call",
            "callId": "mutation-1",
            "name": "control_instance",
            "catalogRevision": vec![0; 32],
            "providerId": "builtin",
            "toolId": "control_instance",
            "toolGeneration": vec![0; 32],
            "arguments": {"instanceId": "alpha"}
        }),
        json!({
            "type": "permission_review",
            "reviewId": "permission-review-7-9-4-mutation-1",
            "callId": "mutation-1",
            "actionSummary": "Start instance alpha",
            "status": "reviewing",
            "assessment": null,
            "startedAt": 11,
            "completedAt": null
        }),
    ];
    let items = contents
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
            history_mutations: items
                .into_iter()
                .map(|item| AgentHistoryMutation::AppendItem { item })
                .collect(),
        })
        .unwrap();
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );

    assert_eq!(service.recover_sessions().unwrap().restored, 1);
    let snapshot = service.snapshot(session_id).unwrap();
    assert!(snapshot.pending_user_input.is_none());
    assert_eq!(snapshot.task_plan.as_ref().unwrap().revision, 2);
    assert_eq!(
        snapshot.task_plan.as_ref().unwrap().explanation.as_deref(),
        Some("Waiting for the tool")
    );
    let turn = snapshot.turn.unwrap();
    assert_eq!(
        turn.phase,
        hanafubuki_agent_kernel::AgentTurnPhase::Interrupted
    );
    assert_eq!(
        turn.reasoning.phase,
        hanafubuki_agent_kernel::AgentReasoningPhase::Completed
    );
    assert_eq!(turn.reasoning.active_since, None);
    assert_eq!(turn.reasoning.step_completed_millis, 7);
    assert_eq!(turn.reasoning.turn_completed_millis, 17);
    assert_eq!(turn.reasoning.last_completed_at, Some(8));
    assert!(matches!(
        &turn.recovery,
        Some(
            hanafubuki_agent_kernel::AgentTurnRecovery::InterruptedAtStartup {
                synthetic_tool_outputs: 1,
                ..
            }
        )
    ));
    let turn_elapsed = u64::try_from(turn.updated_at - turn.created_at).unwrap();
    let recovered = store.load_latest_model_context(session_id).unwrap();
    assert_eq!(
        recovered
            .iter()
            .rev()
            .find(|item| item.turn_id == Some(turn.id))
            .and_then(|item| item.turn_working_millis),
        Some(turn_elapsed),
        "startup interruption must settle the recovered loop onto its final item"
    );
    assert_eq!(
        recovered
            .iter()
            .filter(|item| matches!(item.content, ConversationItem::ToolOutput { .. }))
            .count(),
        1
    );
    assert!(recovered.iter().any(|item| matches!(
        item.content,
        ConversationItem::PermissionReview {
            status: hanafubuki_agent_kernel::PermissionReviewStatus::Aborted,
            assessment: None,
            completed_at: Some(_),
            ..
        }
    )));
}

/// A store holding one session created normally, then checkpointed with
/// `corrupt` applied to its runtime state.
fn store_with_corrupted_session(
    corrupt: impl FnOnce(&mut hanafubuki_agent_kernel::PersistedAgentRuntimeState),
) -> (Arc<EphemeralAgentHistoryStore>, AgentSessionId) {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = service
        .create_model_session(
            text_session_config(),
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            ModelCompactionCapability::LocalSummary,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap();
    drop(service);
    let mut runtime = store.load_runtime_state(session.id).unwrap();
    runtime.revision = hanafubuki_agent_kernel::AgentRevision::new(runtime.revision.value() + 1);
    corrupt(&mut runtime);
    store
        .append_commit(AgentConversationCommit {
            runtime,
            history_mutations: Vec::new(),
        })
        .unwrap();
    (store, session.id)
}

#[test]
fn recovery_rejects_a_session_whose_persisted_counter_passes_the_ceiling() {
    let ceiling = u64::MAX / 2;
    type Corrupt = fn(&mut hanafubuki_agent_kernel::PersistedAgentRuntimeState, u64);
    let corruptions: [(&str, Corrupt); 3] = [
        ("revision", |runtime, value| {
            runtime.revision = hanafubuki_agent_kernel::AgentRevision::new(value)
        }),
        ("next message id", |runtime, value| {
            runtime.next_message_id = value
        }),
        ("next history item id", |runtime, value| {
            runtime.next_history_item_id = value
        }),
    ];
    for (counter, corrupt) in corruptions {
        let (store, _) = store_with_corrupted_session(|runtime| corrupt(runtime, ceiling + 1));
        let service = AgentService::without_default_model_with_history_store(
            AgentKernelRegistries::default(),
            store,
        );
        // Quarantined: listed with its reason, not loaded, the others not
        // stopped by it.
        let report = service.recover_sessions().unwrap();
        assert_eq!(report.restored, 0, "{counter}");
        assert_eq!(report.quarantined.len(), 1, "{counter}");
        let message = &report.quarantined[0].reason;
        assert!(
            message.contains(counter) && message.contains("counter ceiling"),
            "{counter}: {message}"
        );

        // At the ceiling itself the session still loads.
        let (store, session_id) = store_with_corrupted_session(|runtime| corrupt(runtime, ceiling));
        let service = AgentService::without_default_model_with_history_store(
            AgentKernelRegistries::default(),
            store,
        );
        assert_eq!(service.recover_sessions().unwrap().restored, 1, "{counter}");
        assert!(service.snapshot(session_id).is_ok());
    }
}

#[test]
fn a_quarantined_session_does_not_stop_the_others_and_keeps_its_stored_state() {
    let (store, corrupt) = store_with_corrupted_session(|runtime| {
        runtime.next_message_id = u64::MAX / 2 + 1;
    });
    let writer = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let intact = writer
        .create_model_session(
            text_session_config(),
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            ModelCompactionCapability::LocalSummary,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap()
        .id;
    drop(writer);
    let stored = store.load_runtime_state(corrupt).unwrap();

    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let report = service.recover_sessions().unwrap();
    assert_eq!(report.restored, 1);
    assert_eq!(
        report
            .quarantined
            .iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        vec![corrupt]
    );
    assert!(service.snapshot(intact).is_ok());
    assert!(service.snapshot(corrupt).is_err());
    assert_eq!(store.load_runtime_state(corrupt).unwrap(), stored);
}

/// A configuration rule that tightened since a session was written repairs
/// that field at recovery instead of quarantining the session.
#[test]
fn recovery_re_normalizes_a_config_an_older_rule_allowed() {
    let (store, session_id) = store_with_corrupted_session(|runtime| {
        runtime.config.auto_compact_threshold_percent = 0;
        runtime.config.context_window_tokens = Some(0);
        runtime.config.sampling_retry_policy = hanafubuki_agent_kernel::AgentSamplingRetryPolicy {
            maximum_retries: 50,
            initial_backoff_millis: 0,
            maximum_backoff_millis: 120_000,
        };
    });
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store,
    );

    let report = service.recover_sessions().unwrap();

    assert_eq!(report.restored, 1);
    assert!(report.quarantined.is_empty(), "{:?}", report.quarantined);
    assert!(service.snapshot(session_id).is_ok());
}

/// Integrity is not repaired: a broken counter still quarantines.
#[test]
fn recovery_still_quarantines_what_it_cannot_repair() {
    let (store, _) = store_with_corrupted_session(|runtime| {
        runtime.config.auto_compact_threshold_percent = 0;
        runtime.config.provider_id = String::new();
    });
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store,
    );
    let report = service.recover_sessions().unwrap();
    assert_eq!(report.restored, 0);
    assert_eq!(report.quarantined.len(), 1);
}

/// A session's context window is resolved when it is created or its model
/// is selected: the configured window, else the built-in table of
/// well-known models, else none. It is written into the persisted
/// configuration, and the usage recorded against the old window moves with
/// it, so the session still recovers.
#[tokio::test]
async fn the_context_window_is_resolved_at_creation_and_model_selection_and_recovers() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(ScriptedPersistenceModel {
            responses: Mutex::new(VecDeque::from([final_answer_events("Done.").into()])),
            requests: AtomicUsize::new(0),
        }),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let create = |model: &str, context_window_tokens: Option<u64>| {
        service
            .create_model_session(
                AgentSessionConfig {
                    model: model.to_string(),
                    context_window_tokens,
                    ..text_session_config()
                },
                hanafubuki_agent_kernel::ModelApiProtocol::Responses,
                "http://127.0.0.1:1/v1",
                None,
                Vec::new(),
                hanafubuki_agent_kernel::ModelRetryPolicy::default(),
                ModelCompactionCapability::LocalSummary,
                hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
            )
            .unwrap()
    };
    let window = |id| {
        store
            .load_runtime_state(id)
            .unwrap()
            .config
            .context_window_tokens
    };
    let known = create("claude-opus-5-5", None);
    let configured = create("claude-opus-5-5", Some(64_000));
    assert_eq!(window(known.id), Some(1_000_000));
    assert_eq!(window(configured.id), Some(64_000));
    let unknown = service.create_session(text_session_config()).unwrap();
    service.start_turn(unknown.id, "hello").await.unwrap();
    let usage = store.load_runtime_state(unknown.id).unwrap().usage.unwrap();
    assert_eq!(usage.context_window_tokens, None);
    let unknown = service.snapshot(unknown.id).unwrap();

    service
        .set_session_model_configuration(
            unknown.id,
            unknown.generation,
            unknown.revision,
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            AgentSessionModelConfigurationUpdate {
                provider_id: "provider-1".into(),
                provider_protocol: hanafubuki_agent_kernel::ModelApiProtocol::Responses,
                model: "gpt-5".into(),
                instructions: "test".into(),
                reasoning_effort: None,
                context_window_tokens: None,
                input_capabilities: Default::default(),
                auto_compaction_enabled: true,
                auto_compact_threshold_percent: 80,
                compaction_capability: ModelCompactionCapability::LocalSummary,
                turn_budget: Default::default(),
            },
        )
        .unwrap();
    assert_eq!(window(unknown.id), Some(272_000));
    assert_eq!(
        store
            .load_runtime_state(unknown.id)
            .unwrap()
            .usage
            .unwrap()
            .context_window_tokens,
        Some(272_000)
    );
    drop(service);

    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    recovered.activate_session(unknown.id).unwrap();
    assert_eq!(window(unknown.id), Some(272_000));
}

/// Steering recorded before the host context had a kind reads back without
/// its context; steering with a host context reads back with its kind.
#[test]
fn recorded_steering_reads_back_with_or_without_a_host_context() {
    let legacy: hanafubuki_agent_kernel::AgentSteeringInput = serde_json::from_value(json!({
        "sequence": 1,
        "text": "also check readiness",
        "applicationContext": { "content": "{\"page\":\"chooser\"}" },
        "acceptedAt": 5,
    }))
    .unwrap();
    assert_eq!(legacy.text, "also check readiness");
    assert_eq!(legacy.host_context, None);

    let current: hanafubuki_agent_kernel::AgentSteeringInput = serde_json::from_value(json!({
        "sequence": 2,
        "text": "then stop",
        "hostContext": { "kind": "host_location", "content": "{\"page\":\"chooser\"}" },
        "acceptedAt": 6,
    }))
    .unwrap();
    let context = current.host_context.unwrap();
    assert_eq!(context.kind.as_str(), "host_location");
    assert_eq!(context.content, "{\"page\":\"chooser\"}");
}
