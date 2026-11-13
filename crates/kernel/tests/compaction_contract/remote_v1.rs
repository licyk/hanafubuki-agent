use super::*;

#[tokio::test]
async fn responses_compact_v1_filters_execution_items_and_keeps_typed_opaque_replay() {
    let payload = ModelOpaqueCompactionPayload::parse("opaque-v1".to_string()).unwrap();
    let model = ScriptedModel::remote(
        ModelCompactionCapability::ResponsesCompact,
        [response("answer", "world", 10)],
        [Ok(ModelCompactionResult {
            replacement_history: vec![
                ModelInputItem::ContextFragment {
                    role: hanafubuki_agent_model_api::ModelContextRole::Developer,
                    kind: "remote_compaction".to_string(),
                    content: "provider instruction copy".to_string(),
                },
                ModelInputItem::user_text("hello"),
                ModelInputItem::AssistantMessage {
                    phase: Some(ModelMessagePhase::FinalAnswer),
                    text: "world".to_string(),
                },
                ModelInputItem::ToolCall {
                    call_id: "discard".to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                },
                ModelInputItem::Compaction {
                    payload: payload.clone(),
                },
            ],
            usage: Some(ModelUsage {
                input_tokens: 10,
                output_tokens: 1,
                ..Default::default()
            }),
        })],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut settings = config();
    settings.compaction_capability = ModelCompactionCapability::ResponsesCompact;
    settings.reasoning_effort = Some(hanafubuki_agent_model_api::ModelReasoningEffort::High);
    let session = service.create_session(settings).unwrap();
    service.start_turn(session.id, "hello").await.unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert_eq!(history.len(), 3);
    assert!(matches!(history[0], ConversationItem::UserMessage { .. }));
    assert!(matches!(
        history[1],
        ConversationItem::AssistantMessage { .. }
    ));
    assert!(matches!(
        &history[2],
        ConversationItem::ProviderCompaction { payload } if payload.as_str() == "opaque-v1"
    ));
    let requests = model.compact_requests();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].tools.is_empty());
    assert_eq!(
        requests[0]
            .reasoning
            .as_ref()
            .map(|reasoning| &reasoning.effort),
        Some(&hanafubuki_agent_model_api::ModelReasoningEffort::High)
    );
}
