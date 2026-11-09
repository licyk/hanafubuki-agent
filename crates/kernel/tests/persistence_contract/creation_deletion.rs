use super::*;

#[tokio::test]
async fn image_only_turn_is_durable_and_unsupported_capability_fails_before_admission() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let unsupported = service
        .create_session(AgentSessionConfig {
            provider_id: "provider-1".to_string(),
            provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
            model: "text-only".to_string(),
            instructions: "test".to_string(),
            mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
            reasoning_effort: None,
            context_window_tokens: None,
            input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilityPolicy {
                image_input: hanafubuki_agent_model_api::ModelImageInputPolicy::Disabled,
            },
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: ModelCompactionCapability::LocalSummary,
            sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
        })
        .unwrap();
    let forged = hanafubuki_agent_kernel::AgentImageAttachmentRef {
        attachment_id: hanafubuki_agent_kernel::AgentAttachmentId::from_sha256("0".repeat(64)),
        source_session_id: unsupported.id,
        mime_type: "image/png".to_string(),
        byte_length: 1,
        width: 1,
        height: 1,
        display_name: "pixel.png".to_string(),
        detail: hanafubuki_agent_kernel::AgentImageDetail::Auto,
    };
    assert!(matches!(
        service
            .start_turn(
                unsupported.id,
                AgentTurnInput {
                    text: String::new(),
                    images: vec![forged],
                    skills: Vec::new(),
                    turn: Default::default(),
                    world_state: Default::default(),
                    permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                    origin: Default::default(),
                    provisional_title: None,
                },
            )
            .await,
        Err(hanafubuki_agent_kernel::AgentError::InvalidInput(_))
    ));
    assert!(store
        .load_latest_model_context(unsupported.id)
        .unwrap()
        .is_empty());

    let supported = service
        .create_session(AgentSessionConfig {
            provider_id: "provider-1".to_string(),
            provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
            model: "vision".to_string(),
            instructions: "test".to_string(),
            mode: hanafubuki_agent_kernel::AgentSessionMode::Default,
            reasoning_effort: None,
            context_window_tokens: None,
            input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilityPolicy::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: ModelCompactionCapability::LocalSummary,
            sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
        })
        .unwrap();
    let image = service
        .stage_image_attachment(
            supported.id,
            hanafubuki_agent_kernel::PreparedAgentImage {
                bytes: vec![7],
                mime_type: "image/png".to_string(),
                width: 1,
                height: 1,
                display_name: "pixel.png".to_string(),
                detail: hanafubuki_agent_kernel::AgentImageDetail::Auto,
            },
        )
        .unwrap();
    assert!(service
        .start_turn(
            supported.id,
            AgentTurnInput {
                text: String::new(),
                images: vec![image.clone()],
                skills: Vec::new(),
                turn: Default::default(),
                world_state: Default::default(),
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await
        .is_err());
    assert!(store
        .load_latest_model_context(supported.id)
        .unwrap()
        .iter()
        .any(|item| matches!(
            &item.content,
            ConversationItem::UserMessage { text, images }
                if text.is_empty() && images.len() == 1 && images[0] == image
        )));
}

#[tokio::test]
async fn conversational_plan_projection_survives_persistence_and_startup_recovery() {
    let canonical = "Intro\n<proposed_plan>\n- Persist it.\n</proposed_plan>";
    let events = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "plan-response".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: "plan-item".to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: String::new(),
            },
        }),
        Ok(ModelEvent::AssistantTextDelta {
            item_id: "plan-item".to_string(),
            delta: canonical.to_string(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "plan-item".to_string(),
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: canonical.to_string(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "plan-response".to_string(),
            usage: None,
            end_turn: Some(true),
        }),
    ];
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let model = Arc::new(ScriptedPersistenceModel {
        responses: Mutex::new(VecDeque::from([events])),
        requests: AtomicUsize::new(0),
    });
    let service =
        AgentService::with_history_store(model, AgentKernelRegistries::default(), store.clone());
    let mut config = text_session_config();
    config.mode = hanafubuki_agent_kernel::AgentSessionMode::Plan;
    let session = service.create_session(config).unwrap();
    service.start_turn(session.id, "Plan it").await.unwrap();

    let persisted = store.load_latest_model_context(session.id).unwrap();
    assert!(persisted.iter().any(|item| matches!(
        &item.content,
        ConversationItem::AssistantMessage {
            text,
            model_text: Some(model_text),
            proposed_plan: Some(plan),
            ..
        } if text == "Intro\n"
            && model_text == canonical
            && plan.text == "- Persist it.\n"
    )));
    drop(service);

    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store,
    );
    assert_eq!(recovered.recover_sessions().unwrap().restored, 1);
    let opened = recovered.open_session(session.id, 50).unwrap();
    assert!(opened.transcript.items.iter().any(|item| matches!(
        &item.content,
        ConversationItem::AssistantMessage {
            model_text: Some(model_text),
            proposed_plan: Some(plan),
            ..
        } if model_text == canonical && plan.text == "- Persist it.\n"
    )));
}

#[test]
fn prepared_deletion_can_be_cancelled_or_completed_with_the_exact_capability() {
    let service = AgentService::new(
        Arc::new(RequestRetriesExhaustedModel),
        AgentKernelRegistries::default(),
    );
    let created = service.create_session(text_session_config()).unwrap();
    let prepared = service
        .prepare_session_deletion(created.id, created.generation, created.revision, &[])
        .unwrap();
    service.cancel_prepared_session_deletion(prepared);
    assert!(service.snapshot(created.id).is_ok());

    let prepared = service
        .prepare_session_deletion(created.id, created.generation, created.revision, &[])
        .unwrap();
    let confirmed = service
        .complete_session_deletion_confirmed(prepared)
        .unwrap();
    assert_eq!(confirmed.session_id, created.id);
    assert!(matches!(
        service.snapshot(created.id),
        Err(hanafubuki_agent_kernel::AgentError::SessionNotFound)
    ));
}

#[test]
fn session_creation_uses_a_small_runtime_checkpoint_without_history_snapshot() {
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

    let commits = store.commits.lock().unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].runtime.id, created.id);
    assert!(commits[0].history_mutations.is_empty());
    assert_eq!(commits[0].runtime.next_history_item_id, 1);
}

#[tokio::test]
async fn unconfigured_session_is_durable_but_rejects_turn_admission() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let mut config = text_session_config();
    config.model.clear();
    config.instructions.clear();
    config.reasoning_effort = None;
    config.context_window_tokens = None;
    config.compaction_capability = ModelCompactionCapability::LocalSummary;
    let created = service
        .create_unconfigured_session(
            config,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap();

    assert!(created.model.is_empty());
    let error = service
        .start_turn(created.id, "must not be admitted")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no model is selected"));
    let persisted = store.load_runtime_state(created.id).unwrap();
    assert_eq!(persisted.revision, created.revision);
    assert!(store
        .load_latest_model_context(created.id)
        .unwrap()
        .is_empty());
}

#[test]
fn reasoning_choices_are_durable_and_survive_session_recovery() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(ResponseStartedThenErrorModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let mut config = text_session_config();
    config.reasoning_effort = Some(hanafubuki_agent_model_api::ModelReasoningEffort::High);
    let session = service.create_session(config).unwrap();
    let persisted = store.load_runtime_state(session.id).unwrap();
    assert_eq!(
        persisted.config.reasoning_effort,
        Some(hanafubuki_agent_model_api::ModelReasoningEffort::High)
    );
    drop(service);

    let recovered = AgentService::without_default_model_with_history_store(
        AgentKernelRegistries::default(),
        store.clone(),
    );
    assert_eq!(recovered.recover_sessions().unwrap().restored, 1);
    let persisted_after_recovery = store.load_runtime_state(session.id).unwrap();
    assert_eq!(persisted_after_recovery.config, persisted.config);
}

/// Reasoning effort applies only to the Responses protocol; elsewhere it is
/// ignored with a warning, never a refused session.
#[test]
fn reasoning_effort_on_another_protocol_is_ignored() {
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(ResponseStartedThenErrorModel),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let mut config = text_session_config();
    config.provider_protocol = hanafubuki_agent_model_api::ModelApiProtocol::ChatCompletions;
    config.reasoning_effort = Some(hanafubuki_agent_model_api::ModelReasoningEffort::High);
    let session = service.create_session(config).unwrap();
    assert_eq!(session.reasoning_effort, None);
    assert_eq!(
        store
            .load_runtime_state(session.id)
            .unwrap()
            .config
            .reasoning_effort,
        None
    );
}

#[test]
fn stable_history_item_identity_is_independent_from_conversation_payload() {
    let item = AgentHistoryItem {
        source_session_id: AgentSessionId::new(1),
        item_id: hanafubuki_agent_kernel::AgentHistoryItemId::new(7),
        item_revision: 0,
        turn_id: Some(AgentTurnId::new(3)),
        logical_segment: 0,
        logical_order: 0,
        turn_working_millis: None,
        content: ConversationItem::UserMessage {
            text: "hello".to_string(),
            images: Vec::new(),
        },
    };
    let mutation = AgentHistoryMutation::AppendItem { item: item.clone() };
    assert_eq!(mutation, AgentHistoryMutation::AppendItem { item });
}
