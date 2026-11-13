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
                    ..Default::default()
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

/// The cache figures of each reported response: the last response's as
/// reported (none when it reported none), summed across responses, with a
/// count of the responses that reported any; the context consumed is the
/// prompt total, cache included.
#[tokio::test]
async fn cache_figures_are_kept_per_response_and_summed() {
    let answered = |turns: &[(&str, &str)], next: &str| {
        let mut input = Vec::new();
        for (prompt, answer) in turns {
            input.push(ModelInputItem::user_text(*prompt));
            input.push(ModelInputItem::AssistantMessage {
                phase: Some(ModelMessagePhase::FinalAnswer),
                text: answer.to_string(),
            });
        }
        input.push(ModelInputItem::user_text(next));
        request(input)
    };
    let reported = |input_tokens, cache_read_tokens, cache_write_tokens| ModelUsage {
        input_tokens,
        output_tokens: 1,
        cache_read_tokens,
        cache_write_tokens,
    };
    let model = FakeModelTransport::new([
        FakeModelExchange::new(
            answered(&[], "first"),
            assistant_events_with_usage("one", Some(reported(100, Some(60), Some(30)))),
        ),
        FakeModelExchange::new(
            answered(&[("first", "one")], "second"),
            assistant_events_with_usage("two", Some(reported(120, Some(90), None))),
        ),
        FakeModelExchange::new(
            answered(&[("first", "one"), ("second", "two")], "third"),
            assistant_events_with_usage("three", Some(reported(130, None, None))),
        ),
    ]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    let mut seen = Vec::new();
    for prompt in ["first", "second", "third"] {
        service.start_turn(session.id, prompt).await.unwrap();
        let usage = test_session_view(&service, session.id)
            .unwrap()
            .usage
            .unwrap();
        seen.push((
            usage.context_consumed_tokens,
            usage.last_response_cache_read_tokens,
            usage.last_response_cache_write_tokens,
            usage.cumulative_input_tokens,
            usage.cumulative_cache_read_tokens,
            usage.cumulative_cache_write_tokens,
            usage.cache_reporting_responses,
        ));
    }

    assert_eq!(
        seen,
        [
            (Some(100), Some(60), Some(30), 100, 60, 30, 1),
            (Some(120), Some(90), None, 220, 150, 30, 2),
            (Some(130), None, None, 350, 150, 30, 2),
        ]
    );
}
