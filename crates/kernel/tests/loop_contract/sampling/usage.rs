use super::*;

#[tokio::test]
async fn missing_usage_keeps_recent_provider_reported_usage_authoritative() {
    let first_request = request(vec![ModelInputItem::user_text("first")]);
    let second_request = request(vec![
        ModelInputItem::user_text("first"),
        ModelInputItem::AssistantMessage {
            phase: Some(ModelMessagePhase::FinalAnswer),
            text: "one".to_string(),
        },
        ModelInputItem::user_text("second"),
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(
            first_request,
            assistant_events_with_usage(
                "one",
                Some(ModelUsage {
                    input_tokens: 11,
                    output_tokens: 4,
                }),
            ),
        ),
        FakeModelExchange::new(second_request, assistant_events_with_usage("two", None)),
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "first").await.unwrap();
    let authoritative = test_session_view(&service, session.id).unwrap().usage;
    service.start_turn(session.id, "second").await.unwrap();

    assert_eq!(
        test_session_view(&service, session.id).unwrap().usage,
        authoritative
    );
    assert_eq!(
        authoritative.unwrap().source,
        AgentUsageSource::ProviderReported
    );
}

#[tokio::test]
async fn live_history_snapshots_share_items_and_copy_only_the_pointer_vector_on_append() {
    let first_request = request(vec![ModelInputItem::user_text("first")]);
    let second_request = request(vec![
        ModelInputItem::user_text("first"),
        ModelInputItem::AssistantMessage {
            phase: Some(ModelMessagePhase::FinalAnswer),
            text: "one".to_string(),
        },
        ModelInputItem::user_text("second"),
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request, deterministic_assistant_events("one")),
        FakeModelExchange::new(second_request, deterministic_assistant_events("two")),
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "first").await.unwrap();
    let (first_backing, first_version) = service.test_history_backing(session.id).unwrap();
    assert!(!first_backing.is_empty());

    service
        .set_session_turn_budget_policy(
            session.id,
            AgentTurnBudgetPolicy {
                maximum_tool_invocations: 255,
                ..AgentTurnBudgetPolicy::default()
            },
        )
        .unwrap();
    let (runtime_only_backing, runtime_only_version) =
        service.test_history_backing(session.id).unwrap();
    assert!(Arc::ptr_eq(&first_backing, &runtime_only_backing));
    assert_eq!(first_version, runtime_only_version);

    service.start_turn(session.id, "second").await.unwrap();
    let (second_backing, second_version) = service.test_history_backing(session.id).unwrap();
    assert!(!Arc::ptr_eq(&first_backing, &second_backing));
    assert!(second_version > first_version);
    assert_eq!(first_backing.len(), runtime_only_backing.len());
    assert!(second_backing.len() > first_backing.len());
    for (before, after) in first_backing.iter().zip(second_backing.iter()) {
        assert!(Arc::ptr_eq(before, after));
    }
    assert!(matches!(
        &first_backing.last().unwrap().content,
        ConversationItem::AssistantMessage { text, .. } if text == "one"
    ));
}

#[tokio::test]
async fn absent_initial_provider_usage_is_explicitly_estimated() {
    let expected = request(vec![ModelInputItem::user_text("estimate")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        assistant_events_with_usage("result", None),
    )]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "estimate").await.unwrap();

    let usage = test_session_view(&service, session.id)
        .unwrap()
        .usage
        .unwrap();
    assert_eq!(usage.source, AgentUsageSource::Estimated);
    assert!(usage.last_response_input_tokens > 0);
    assert!(usage.last_response_output_tokens > 0);
}
