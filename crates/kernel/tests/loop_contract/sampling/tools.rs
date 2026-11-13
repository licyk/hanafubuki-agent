use super::*;

fn named_tool_call_events(
    name: &str,
    arguments: serde_json::Value,
) -> Vec<ModelResult<ModelEvent>> {
    let call = ModelToolCall {
        item_id: "named-tool-item".to_string(),
        call_id: "named-tool-call".to_string(),
        namespace: None,
        name: name.to_string(),
        arguments,
        argument_error: None,
    };
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "named-tool-response".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall { call: call.clone() },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "named-tool-response".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ]
}

#[derive(Clone)]
struct NativeToolSearchScriptedModel {
    inner: ScriptedEventModel,
}

impl NativeToolSearchScriptedModel {
    fn new(responses: impl IntoIterator<Item = Vec<ModelResult<ModelEvent>>>) -> Self {
        Self {
            inner: ScriptedEventModel::new(responses),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.inner.requests()
    }
}

impl ModelPort for NativeToolSearchScriptedModel {
    fn tool_capabilities(&self) -> hanafubuki_agent_tool_contracts::ToolProviderCapabilities {
        hanafubuki_agent_tool_contracts::ToolProviderCapabilities {
            function_tools: true,
            strict_json_schema: false,
            maximum_tool_name_bytes: 64,
            namespace_tools: false,
            native_tool_search: true,
            freeform_tools: false,
            hosted_web_search: false,
            parallel_tool_calls: false,
            multimodal_tool_output_transport:
                hanafubuki_agent_tool_contracts::MultimodalToolOutputTransport::Unsupported,
            required_named_tool_choice: false,
        }
    }

    fn stream(
        &self,
        request: ModelRequest,
        cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        self.inner.stream(request, cancellation)
    }
}

#[tokio::test]
async fn an_unselected_deferred_tool_returns_recovery_output_without_dispatch() {
    let model = ScriptedEventModel::new([
        named_tool_call_events(
            "configure_instance",
            json!({ "instanceId": "instance-a", "action": "set_launch_args", "args": [] }),
        ),
        deterministic_assistant_events("I need to discover that tool before retrying."),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "Configure the instance".to_string())
        .await
        .unwrap();

    assert_eq!(
        outcome.final_answer,
        "I need to discover that tool before retrying."
    );
    assert!(tools.calls().is_empty());
    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].tools.iter().any(|tool| {
        tool.name()
            .is_some_and(|name| name.flattened_alias() == "configure_instance")
    }));
    assert!(requests[0].tools.iter().any(|tool| {
        tool.name()
            .is_some_and(|name| name.flattened_alias() == "tool_search")
    }));
    assert!(requests[1].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ToolOutput { call_id, output }
            if call_id == "named-tool-call"
                && output.is_error
                && output.structured_content.as_ref().is_some_and(|content| {
                    content["error"] == "tool requires discovery before use"
                        && content["tool"] == "configure_instance"
                        && content["recovery"]["tool"] == "tool_search"
                })
    )));
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert!(history.iter().any(|item| matches!(
        item,
        ConversationItem::ToolOutput { call_id, output, .. }
            if call_id == "named-tool-call" && output.is_error
    )));
}

#[tokio::test]
async fn native_search_protocol_also_recovers_an_unselected_deferred_tool() {
    let model = NativeToolSearchScriptedModel::new([
        named_tool_call_events(
            "configure_instance",
            json!({ "instanceId": "instance-a", "action": "set_launch_args", "args": [] }),
        ),
        deterministic_assistant_events("I will search before retrying."),
    ]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "Configure the instance".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "I will search before retrying.");
    assert!(tools.calls().is_empty());
    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].tools.iter().any(|tool| matches!(
        tool,
        hanafubuki_agent_tool_contracts::ToolSpec::ToolSearch(_)
    )));
    assert!(!requests[0].tools.iter().any(|tool| {
        tool.name()
            .is_some_and(|name| name.flattened_alias() == "configure_instance")
    }));
    assert!(requests[1].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ToolOutput { output, .. }
            if output.is_error
                && output.structured_content.as_ref().is_some_and(|content| {
                    content["recovery"]["tool"] == "tool_search"
                })
    )));
}

#[tokio::test]
async fn fake_model_and_tools_complete_a_deterministic_two_sampling_loop() {
    let first_request = request(vec![ModelInputItem::user_text("What instances exist?")]);
    let tool_output =
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false);
    let second_request = request(vec![
        ModelInputItem::user_text("What instances exist?"),
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
        FakeModelExchange::new(
            second_request.clone(),
            deterministic_assistant_events("No instances are configured."),
        ),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        tool_output.clone(),
    )])
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let created = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(created.id, "What instances exist?".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.generation.value(), 1);
    assert_eq!(outcome.final_answer, "No instances are configured.");
    assert_eq!(model.requests(), vec![first_request, second_request]);
    assert_eq!(model.remaining_exchanges(), 0);
    assert_eq!(
        tools.calls(),
        vec![("list_instances".to_string(), json!({}))]
    );
    assert_eq!(tools.remaining_exchanges(), 0);

    let snapshot = test_session_view(&service, created.id).unwrap();
    assert_eq!(
        snapshot.usage,
        Some(hanafubuki_agent_kernel::AgentModelUsage {
            last_response_input_tokens: 3,
            last_response_output_tokens: 2,
            cumulative_input_tokens: 7,
            cumulative_output_tokens: 3,
            context_window_tokens: None,
            context_consumed_tokens: Some(3),
            source: AgentUsageSource::ProviderReported,
            last_response_cache_read_tokens: None,
            last_response_cache_write_tokens: None,
            cumulative_cache_read_tokens: 0,
            cumulative_cache_write_tokens: 0,
            cache_reporting_responses: 0,
        })
    );
    let final_answer_message_id = snapshot
        .turn
        .as_ref()
        .unwrap()
        .final_answer_message_id
        .unwrap();
    assert_eq!(snapshot.model_context.len(), 5);
    assert!(matches!(
        &snapshot.model_context[0],
        ConversationItem::TurnStart { turn_id }
            if *turn_id == snapshot.turn.as_ref().unwrap().id
    ));
    assert!(matches!(
        &snapshot.model_context[1],
        ConversationItem::UserMessage { text, .. } if text == "What instances exist?"
    ));
    assert!(matches!(
        &snapshot.model_context[2],
        ConversationItem::ToolCall {
            call_id,
            name,
            provider_id,
            tool_id,
            catalog_revision,
            tool_generation,
            arguments,
            ..
        } if call_id == FIXTURE_CALL_ID
            && name == "list_instances"
            && provider_id.as_str() == "builtin"
            && tool_id.as_str() == "list_instances"
            && catalog_revision.0 != [0; 32]
            && tool_generation.0 != [0; 32]
            && arguments == &json!({})
    ));
    assert!(matches!(
        &snapshot.model_context[3],
        ConversationItem::ToolOutput { call_id, output, .. }
            if call_id == FIXTURE_CALL_ID && output == &tool_output
    ));
    assert!(matches!(
        &snapshot.model_context[4],
        ConversationItem::AssistantMessage {
            message_id,
            phase: Some(hanafubuki_agent_kernel::AgentMessagePhase::FinalAnswer),
            text,
            sampling_step: 2,
            ..
        } if *message_id == final_answer_message_id
            && text == "No instances are configured."
    ));
    let turn = snapshot.runtime.turn.as_ref().unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Completed);
    assert_eq!(turn.sampling_step, 2);
    assert!(snapshot.revision.value() > created.revision.value());

    let events = service.events_since(created.id, None).unwrap();
    assert_eq!(
        events
            .events
            .iter()
            .filter(|event| matches!(
                event.kind,
                AgentEventKind::HistoryCommitted { usage: Some(_), .. }
            ))
            .count(),
        2
    );
    assert!(events
        .events
        .windows(2)
        .all(|events| events[1].sequence.value() == events[0].sequence.value() + 1));
    assert_eq!(
        events
            .events
            .iter()
            .filter(|event| event.kind.is_terminal())
            .count(),
        1
    );
    assert!(matches!(
        events.events.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnCompleted {
            final_answer_message_id: event_final_answer_message_id,
        }) if *event_final_answer_message_id == Some(final_answer_message_id)
    ));
}

#[tokio::test]
async fn completed_tool_item_is_authoritative_when_preview_item_id_changes() {
    let first_request = request(vec![ModelInputItem::user_text("What instances exist?")]);
    let tool_output =
        hanafubuki_agent_tool_contracts::ToolOutput::structured(json!({ "instances": [] }), false);
    let second_request = request(vec![
        ModelInputItem::user_text("What instances exist?"),
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
    let tool_events = vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "response-tool".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "tool-preview".to_string(),
                    call_id: FIXTURE_CALL_ID.to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: serde_json::Value::Null,
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall {
                call: ModelToolCall {
                    item_id: "tool-completed".to_string(),
                    call_id: FIXTURE_CALL_ID.to_string(),
                    namespace: None,
                    name: "list_instances".to_string(),
                    arguments: json!({}),
                    argument_error: None,
                },
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "response-tool".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ];
    let model = FakeModelTransport::new([
        FakeModelExchange::new(first_request, tool_events),
        FakeModelExchange::new(
            second_request,
            deterministic_assistant_events("No instances are configured."),
        ),
    ]);
    let tools = FakeTools::new([FakeRegisteredExchange::success(
        "list_instances",
        json!({}),
        tool_output,
    )])
    .await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let created = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(created.id, "What instances exist?".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "No instances are configured.");
    let snapshot = test_session_view(&service, created.id).unwrap();
    assert_eq!(
        snapshot
            .model_context
            .iter()
            .filter(|item| matches!(item, ConversationItem::ToolCall { .. }))
            .count(),
        1
    );
}
