use super::*;

#[tokio::test]
async fn responses_compact_v2_requires_exactly_one_compaction_item_and_keeps_it_last() {
    let model = ScriptedModel::remote(
        ModelCompactionCapability::ResponsesCompactV2,
        [
            response("answer", "world", 10),
            v2_compaction_response(&["opaque-v2"], None),
        ],
        [],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let mut settings = config();
    settings.compaction_capability = ModelCompactionCapability::ResponsesCompactV2;
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
    assert!(matches!(
        history.last(),
        Some(ConversationItem::ProviderCompaction { payload }) if payload.as_str() == "opaque-v2"
    ));
    assert!(matches!(
        model.requests()[1].input.last(),
        Some(ModelInputItem::CompactionTrigger)
    ));

    for invalid_response in [
        v2_compaction_response(&[], None),
        v2_compaction_response(&["one", "two"], None),
        v2_compaction_response(&[], Some("ordinary summary")),
        v2_compaction_response_with_late_event(),
        v2_compaction_response_with_early_eof(),
    ] {
        let model = ScriptedModel::remote(
            ModelCompactionCapability::ResponsesCompactV2,
            [response("answer", "world", 10), invalid_response],
            [],
        );
        let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
        let mut settings = config();
        settings.compaction_capability = ModelCompactionCapability::ResponsesCompactV2;
        let session = service.create_session(settings).unwrap();
        service.start_turn(session.id, "hello").await.unwrap();
        let before = service.snapshot(session.id).unwrap();
        assert!(matches!(
            service
                .compact_session(session.id, before.generation, before.revision)
                .await,
            Err(AgentError::Protocol(_))
        ));
        assert_eq!(
            service
                .snapshot(session.id)
                .unwrap()
                .compaction
                .unwrap()
                .status,
            AgentCompactionStatus::Failed
        );
    }
}
