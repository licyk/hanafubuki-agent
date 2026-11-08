use super::*;

#[tokio::test]
async fn cumulative_model_budget_rejects_response_commit_atomically() {
    let first_request = request(vec![ModelInputItem::user_text("first")]);
    let tool_output =
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false);
    let second_request = request(vec![
        ModelInputItem::user_text("first"),
        ModelInputItem::ToolCall {
            call_id: FIXTURE_CALL_ID.to_string(),
            namespace: None,
            name: "list_instances".to_string(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: FIXTURE_CALL_ID.to_string(),
            output: tool_output.clone(),
        },
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(
            first_request,
            tool_call_events_with_usage(ModelUsage {
                input_tokens: 9,
                output_tokens: 1,
            }),
        ),
        FakeModelExchange::new(
            second_request,
            assistant_events_with_usage(
                "must-not-commit",
                Some(ModelUsage {
                    input_tokens: 2,
                    output_tokens: 1,
                }),
            ),
        ),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        tool_output,
    )])
    .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let mut settings = config();
    settings.auto_compaction_enabled = false;
    let session = service.create_session(settings).unwrap();
    service
        .set_session_turn_budget_policy(
            session.id,
            AgentTurnBudgetPolicy {
                maximum_cumulative_model_input_tokens: 10,
                ..AgentTurnBudgetPolicy::default()
            },
        )
        .unwrap();
    assert_eq!(
        service.start_turn(session.id, "first").await,
        Err(AgentError::LimitExceeded {
            resource: "cumulative model input tokens",
            limit: 10,
        })
    );
    assert!(!test_session_view(&service, session.id).unwrap().model_context.iter().any(
        |item| matches!(item, ConversationItem::AssistantMessage { text, .. } if text == "must-not-commit")
    ));
}

#[tokio::test]
async fn configured_context_window_does_not_reject_sampling_before_transport() {
    let expected = request(vec![ModelInputItem::user_text("too large")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected.clone(),
        deterministic_assistant_events("provider accepted the request"),
    )]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            context_window_tokens: Some(1),
            input_capabilities: Default::default(),
            ..config()
        })
        .unwrap();

    assert_eq!(
        service
            .start_turn(session.id, "too large")
            .await
            .unwrap()
            .final_answer,
        "provider accepted the request"
    );
    assert_eq!(model.requests(), vec![expected]);
}

#[tokio::test]
async fn history_limit_rejects_completed_items_without_partial_commit() {
    let limits = AgentHardLimits {
        maximum_history_items: 1,
        ..FIRST_BATCH_HARD_LIMITS
    };
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("world"),
    )]);
    let service = AgentService::with_limits(
        Arc::new(model),
        FakeTools::new([]).await.registries(),
        limits,
    );
    let session = service.create_session(config()).unwrap();

    assert_eq!(
        service.start_turn(session.id, "hello".to_string()).await,
        Err(AgentError::LimitExceeded {
            resource: "history items",
            limit: 1,
        })
    );
    assert_eq!(
        test_session_view(&service, session.id)
            .unwrap()
            .model_context,
        Vec::<ConversationItem>::new()
    );
}

#[tokio::test]
async fn oversized_tool_output_is_canonically_truncated_and_the_turn_continues() {
    const OUTPUT_LIMIT: usize = 512;
    let first_request = request(vec![ModelInputItem::user_text("large tool output")]);
    let original_output = hanafubuki_agent_tool_contracts::ToolOutput::structured(
        json!({ "value": format!("HEAD-{}-TAIL", "花火🙂".repeat(2_000)) }),
        false,
    );
    let (bounded_output, expected_truncation) =
        bound_tool_output_for_test(original_output.clone(), OUTPUT_LIMIT);
    let second_request = request(vec![
        ModelInputItem::user_text("large tool output"),
        ModelInputItem::ToolCall {
            call_id: FIXTURE_CALL_ID.to_string(),
            namespace: None,
            name: "list_instances".to_string(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: FIXTURE_CALL_ID.to_string(),
            output: bounded_output.clone(),
        },
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request, deterministic_tool_call_events()),
        FakeModelExchange::new(
            second_request,
            deterministic_assistant_events("continued after truncation"),
        ),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        original_output,
    )])
    .await;
    let limits = AgentHardLimits {
        maximum_tool_output_bytes: OUTPUT_LIMIT,
        ..FIRST_BATCH_HARD_LIMITS
    };
    let service = AgentService::with_limits(Arc::new(model), tools.registries(), limits);
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "large tool output")
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "continued after truncation");
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert!(history.iter().any(|item| matches!(
        item,
        ConversationItem::ToolOutput { call_id, output, truncation, .. }
            if call_id == FIXTURE_CALL_ID
                && output == &bounded_output
                && truncation == &expected_truncation
    )));
}

#[tokio::test]
async fn invalid_post_execution_output_commits_outcome_unknown_without_replaying() {
    const OUTPUT_LIMIT: usize = 512;
    let first_request = request(vec![ModelInputItem::user_text("invalid tool output")]);
    let invalid_output = hanafubuki_agent_tool_contracts::ToolOutput {
        content: vec![hanafubuki_agent_tool_contracts::ToolOutputContent::Image {
            mime_type: "text/plain".to_string(),
            data: "YWxyZWFkeSBleGVjdXRlZA==".to_string(),
            detail: None,
        }],
        structured_content: None,
        is_error: false,
    };
    let (outcome_unknown, expected_truncation) =
        canonicalize_tool_output_for_test(invalid_output.clone(), OUTPUT_LIMIT);
    let second_request = request(vec![
        ModelInputItem::user_text("invalid tool output"),
        ModelInputItem::ToolCall {
            call_id: FIXTURE_CALL_ID.to_string(),
            namespace: None,
            name: "list_instances".to_string(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: FIXTURE_CALL_ID.to_string(),
            output: outcome_unknown.clone(),
        },
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request, deterministic_tool_call_events()),
        FakeModelExchange::new(
            second_request,
            deterministic_assistant_events("reported uncertain outcome"),
        ),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        invalid_output,
    )])
    .await;
    let service = AgentService::with_limits(
        Arc::new(model),
        tools.registries(),
        AgentHardLimits {
            maximum_tool_output_bytes: OUTPUT_LIMIT,
            ..FIRST_BATCH_HARD_LIMITS
        },
    );
    let session = service.create_session(config()).unwrap();

    assert_eq!(
        service
            .start_turn(session.id, "invalid tool output")
            .await
            .unwrap()
            .final_answer,
        "reported uncertain outcome"
    );
    assert!(test_session_view(&service, session.id)
        .unwrap()
        .model_context
        .iter()
        .any(|item| matches!(
            item,
            ConversationItem::ToolOutput { output, truncation, .. }
                if output == &outcome_unknown && truncation == &expected_truncation
        )));
}

#[tokio::test]
async fn no_progress_budget_stops_a_repeating_tool_loop_with_paired_outputs() {
    let first_request = request(vec![ModelInputItem::user_text("loop")]);
    let tool_output =
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false);
    let second_request = request(vec![
        ModelInputItem::user_text("loop"),
        ModelInputItem::ToolCall {
            call_id: FIXTURE_CALL_ID.to_string(),
            namespace: None,
            name: "list_instances".to_string(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: FIXTURE_CALL_ID.to_string(),
            output: tool_output.clone(),
        },
    ]);
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request.clone(), deterministic_tool_call_events()),
        FakeModelExchange::new(second_request.clone(), deterministic_tool_call_events()),
    ]);
    let tools = FakeTools::new([
        FakeRegisteredExchange::success("list_instances", json!({}), tool_output.clone()),
        FakeRegisteredExchange::success("list_instances", json!({}), tool_output),
    ])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();
    service
        .set_session_turn_budget_policy(
            session.id,
            AgentTurnBudgetPolicy {
                maximum_consecutive_stalled_steps: 1,
                ..AgentTurnBudgetPolicy::default()
            },
        )
        .unwrap();

    assert_eq!(
        service.start_turn(session.id, "loop".to_string()).await,
        Err(AgentError::LimitExceeded {
            resource: "consecutive stalled sampling steps",
            limit: 1,
        })
    );
    assert_eq!(model.requests(), vec![first_request, second_request]);
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert_eq!(
        history
            .iter()
            .filter(|item| matches!(item, ConversationItem::ToolCall { .. }))
            .count(),
        history
            .iter()
            .filter(|item| matches!(item, ConversationItem::ToolOutput { .. }))
            .count()
    );
    assert_eq!(
        test_session_view(&service, session.id)
            .unwrap()
            .runtime
            .turn
            .unwrap()
            .phase,
        AgentTurnPhase::Failed
    );
}
