use super::*;

#[tokio::test]
async fn startup_repairs_an_unmatched_tool_call_in_an_already_failed_turn() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let creating_service = AgentService::with_history_store(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session = creating_service
        .create_session(text_session_config())
        .unwrap();
    assert!(creating_service
        .start_turn(session.id, "fail before upgrade")
        .await
        .is_err());
    drop(creating_service);

    let mut runtime = store.load_runtime_state(session.id).unwrap();
    let turn_id = runtime.turn.as_ref().unwrap().id;
    let item_id = hanafubuki_agent_kernel::AgentHistoryItemId::new(runtime.next_history_item_id);
    runtime.next_history_item_id += 1;
    runtime.revision = AgentRevision::new(runtime.revision.value() + 1);
    let call = AgentHistoryItem {
        source_session_id: session.id,
        item_id,
        item_revision: 0,
        turn_id: Some(turn_id),
        logical_segment: 0,
        logical_order: 0,
        turn_working_millis: None,
        content: serde_json::from_value(json!({
            "type": "tool_call",
            "callId": "legacy-unmatched-call",
            "name": "list_instances",
            "catalogRevision": vec![0; 32],
            "providerId": "builtin",
            "toolId": "list_instances",
            "toolGeneration": vec![0; 32],
            "arguments": {}
        }))
        .unwrap(),
    };
    store
        .append_commit(AgentConversationCommit {
            runtime,
            history_mutations: vec![AgentHistoryMutation::AppendItem { item: call }],
        })
        .unwrap();

    let recovering_service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    assert_eq!(recovering_service.recover_sessions().unwrap().restored, 1);

    let recovered = store.load_latest_model_context(session.id).unwrap();
    assert!(recovered.iter().any(|item| matches!(
        &item.content,
        ConversationItem::ToolOutput { call_id, output, .. }
            if call_id == "legacy-unmatched-call" && output.is_error
    )));
    let turn = recovering_service
        .snapshot(session.id)
        .unwrap()
        .turn
        .unwrap();
    assert_eq!(turn.phase, hanafubuki_agent_kernel::AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(hanafubuki_agent_kernel::AgentTurnTerminalReason::ModelRequestRetriesExhausted)
    );
}
#[test]
fn failed_durable_commit_does_not_publish_the_staged_runtime_revision() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let created = service
        .create_model_session(
            AgentSessionConfig {
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
                compaction_strategy: Default::default(),
                scope: Default::default(),
                role_instructions: None,
            },
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            ModelCompactionCapability::LocalSummary,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap();
    store.fail_next_append.store(true, Ordering::SeqCst);

    assert!(matches!(
        service.set_session_mode(
            created.id,
            created.generation,
            created.revision,
            hanafubuki_agent_kernel::AgentSessionMode::Plan,
        ),
        Err(hanafubuki_agent_kernel::AgentError::Persistence(_))
    ));
    let after = service.snapshot(created.id).unwrap();
    assert_eq!(after.revision, created.revision);
    assert_eq!(
        after.mode,
        hanafubuki_agent_kernel::AgentSessionMode::Default
    );
}

#[tokio::test]
async fn failed_history_append_restores_the_exact_history_backing_and_version() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let created = service.create_session(text_session_config()).unwrap();
    let (before_backing, before_version) = service.test_history_backing(created.id).unwrap();
    store.fail_next_append.store(true, Ordering::SeqCst);

    assert!(matches!(
        service.start_turn(created.id, "must roll back").await,
        Err(hanafubuki_agent_kernel::AgentError::Persistence(_))
    ));

    let (after_backing, after_version) = service.test_history_backing(created.id).unwrap();
    assert!(Arc::ptr_eq(&before_backing, &after_backing));
    assert_eq!(after_version, before_version);
    assert_eq!(
        service.snapshot(created.id).unwrap().revision,
        created.revision
    );
}

#[test]
fn model_and_effort_mutations_build_first_and_roll_back_failed_persistence() {
    let store = Arc::new(RecordingHistoryStore::default());
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

    let invalid_transport = service.set_session_model_configuration(
        created.id,
        created.generation,
        created.revision,
        hanafubuki_agent_kernel::ModelApiProtocol::Responses,
        "https://[::1",
        None,
        Vec::new(),
        hanafubuki_agent_kernel::ModelRetryPolicy::default(),
        AgentSessionModelConfigurationUpdate {
            provider_id: "provider-2".into(),
            provider_protocol: hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            model: "replacement".into(),
            instructions: "replacement".into(),
            reasoning_effort: Some(hanafubuki_agent_kernel::ModelReasoningEffort::High),
            context_window_tokens: Some(200_000),
            input_capabilities: Default::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: ModelCompactionCapability::LocalSummary,
            turn_budget: Default::default(),
        },
    );
    assert!(matches!(
        invalid_transport,
        Err(hanafubuki_agent_kernel::AgentError::InvalidInput(_))
    ));
    assert_eq!(service.snapshot(created.id).unwrap(), created);

    store.fail_next_append.store(true, Ordering::SeqCst);
    assert!(matches!(
        service.set_session_reasoning_effort(
            created.id,
            created.generation,
            created.revision,
            Some(hanafubuki_agent_kernel::ModelReasoningEffort::parse("provider-tier").unwrap()),
        ),
        Err(hanafubuki_agent_kernel::AgentError::Persistence(_))
    ));
    assert_eq!(service.snapshot(created.id).unwrap(), created);

    store.fail_next_append.store(true, Ordering::SeqCst);
    assert!(matches!(
        service.set_session_model_configuration(
            created.id,
            created.generation,
            created.revision,
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            AgentSessionModelConfigurationUpdate {
                provider_id: "provider-2".into(),
                provider_protocol: hanafubuki_agent_kernel::ModelApiProtocol::Responses,
                model: "replacement".into(),
                instructions: "replacement".into(),
                reasoning_effort: Some(hanafubuki_agent_kernel::ModelReasoningEffort::High),
                context_window_tokens: Some(200_000),
                input_capabilities: Default::default(),
                auto_compaction_enabled: true,
                auto_compact_threshold_percent: 80,
                compaction_capability: ModelCompactionCapability::LocalSummary,
                turn_budget: Default::default(),
            },
        ),
        Err(hanafubuki_agent_kernel::AgentError::Persistence(_))
    ));
    assert_eq!(service.snapshot(created.id).unwrap(), created);
}

#[test]
fn failed_session_execution_progress_rolls_back_activity_event_and_revision() {
    let store = Arc::new(RecordingHistoryStore::default());
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
    let update = AgentToolProgressUpdate::ExecutionStarted {
        execution_id: 7,
        generation: 2,
        action: AgentActivityAction::fixed("exec_command"),
        status: AgentExecutionProgressStatus::Running,
    };
    store.fail_next_append.store(true, Ordering::SeqCst);

    assert!(matches!(
        service.report_session_execution_progress(created.id, update.clone()),
        Err(hanafubuki_agent_kernel::AgentError::Persistence(_))
    ));
    let rolled_back = service.snapshot(created.id).unwrap();
    assert_eq!(rolled_back.revision, created.revision);
    assert!(rolled_back.active_activities.is_empty());
    assert!(service
        .events_since(created.id, None)
        .unwrap()
        .events
        .is_empty());

    service
        .report_session_execution_progress(created.id, update)
        .unwrap();
    let committed = service.events_since(created.id, None).unwrap().events;
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].turn_id, None);
    assert!(matches!(
        committed[0].kind,
        AgentEventKind::ExecutionProgress { .. }
    ));
}

#[tokio::test]
async fn failed_approved_review_commit_never_authorizes_or_dispatches_the_tool() {
    let tool_call = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "tool-response".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "tool-item".to_string(),
                    call_id: "tool-call".to_string(),
                    namespace: None,
                    name: "control_instance".to_string(),
                    arguments: json!({"instanceId": "instance-a", "action": "start"}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "tool-item".to_string(),
                    call_id: "tool-call".to_string(),
                    namespace: None,
                    name: "control_instance".to_string(),
                    arguments: json!({"instanceId": "instance-a", "action": "start"}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "tool-response".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ];
    let assessment = json!({
        "riskLevel": "high",
        "userAuthorization": "explicit",
        "outcome": "allow",
        "rationale": "The user requested this exact launch."
    })
    .to_string();
    let review = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "review-response".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "review-item".to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: assessment,
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "review-response".to_string(),
            usage: None,
            end_turn: Some(true),
        }),
    ];
    let model = Arc::new(ScriptedPersistenceModel {
        responses: Mutex::new(VecDeque::from([tool_call, review])),
        requests: AtomicUsize::new(0),
    });
    let tools = FakeTools::new([]).await;
    let store = Arc::new(RecordingHistoryStore::default());
    let service =
        AgentService::with_history_store(model.clone(), tools.registries(), store.clone());
    let session = service.create_session(text_session_config()).unwrap();
    store
        .fail_permission_review_completion
        .store(true, Ordering::SeqCst);

    let result = service
        .start_turn(
            session.id,
            AgentTurnInput {
                text: "start it".to_string(),
                images: Vec::new(),
                skills: Vec::new(),
                turn: Default::default(),
                world_state: Default::default(),
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::AutoReview,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await;

    assert!(
        matches!(
            result,
            Err(hanafubuki_agent_kernel::AgentError::Persistence(_))
        ),
        "unexpected turn result: {result:?}"
    );
    assert_eq!(model.requests.load(Ordering::SeqCst), 2);
    assert!(tools.calls().is_empty());
    assert!(store
        .load_latest_model_context(session.id)
        .unwrap()
        .iter()
        .any(|item| matches!(
            item.content,
            ConversationItem::PermissionReview {
                status: hanafubuki_agent_kernel::PermissionReviewStatus::Reviewing,
                assessment: None,
                completed_at: None,
                ..
            }
        )));
}

#[test]
fn runtime_revision_is_not_observable_until_the_durable_commit_is_acknowledged() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = Arc::new(AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    ));
    let created = service
        .create_model_session(
            AgentSessionConfig {
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
                compaction_strategy: Default::default(),
                scope: Default::default(),
                role_instructions: None,
            },
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            ModelCompactionCapability::LocalSummary,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap();
    let gate = Arc::new(AppendGate::new());
    *store.append_gate.lock().unwrap() = Some(gate.clone());

    let mutation_service = service.clone();
    let mutation = thread::spawn(move || {
        mutation_service.set_session_mode(
            created.id,
            created.generation,
            created.revision,
            hanafubuki_agent_kernel::AgentSessionMode::Plan,
        )
    });
    gate.started.wait();

    let snapshot_service = service.clone();
    let (snapshot_sender, snapshot_receiver) = mpsc::channel();
    let snapshot = thread::spawn(move || {
        snapshot_sender
            .send(snapshot_service.snapshot(created.id))
            .unwrap();
    });
    assert!(snapshot_receiver
        .recv_timeout(Duration::from_millis(100))
        .is_err());

    gate.release.wait();
    let updated = mutation.join().unwrap().unwrap();
    let observed = snapshot_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap();
    snapshot.join().unwrap();
    assert_eq!(observed.revision, updated.revision);
    assert_eq!(
        observed.mode,
        hanafubuki_agent_kernel::AgentSessionMode::Plan
    );
}

#[test]
fn session_mutation_waits_for_inflight_execution_progress_commit() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = Arc::new(AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    ));
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
    let session_id = created.id;
    let initial_revision = created.revision;
    let gate = Arc::new(AppendGate::new());
    *store.append_gate.lock().unwrap() = Some(gate.clone());

    let progress_service = service.clone();
    let progress = thread::spawn(move || {
        progress_service.report_session_execution_progress(
            session_id,
            AgentToolProgressUpdate::ExecutionStarted {
                execution_id: 9,
                generation: 1,
                action: AgentActivityAction::fixed("exec_command"),
                status: AgentExecutionProgressStatus::Running,
            },
        )
    });
    gate.started.wait();

    let mutation_service = service.clone();
    let mutation = thread::spawn(move || {
        mutation_service.set_session_turn_budget_policy(
            session_id,
            hanafubuki_agent_kernel::AgentTurnBudgetPolicy::default(),
        )
    });
    thread::sleep(Duration::from_millis(50));
    assert!(!mutation.is_finished());

    gate.release.wait();
    progress.join().unwrap().unwrap();
    mutation.join().unwrap().unwrap();
    let snapshot = service.snapshot(session_id).unwrap();
    assert_eq!(snapshot.revision.value(), initial_revision.value() + 2);
    assert_eq!(snapshot.active_activities.len(), 1);
}
