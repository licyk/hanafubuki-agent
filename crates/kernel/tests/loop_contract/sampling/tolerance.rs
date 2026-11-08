//! Model quirks a turn survives: a call the step cannot run gets one
//! model-visible error and the turn goes on; a provider that ends a response
//! early or completes an item it never announced keeps what it sent.

use super::*;

fn call(name: &str, arguments: serde_json::Value) -> ModelToolCall {
    ModelToolCall {
        item_id: "tolerance-item".to_string(),
        call_id: "tolerance-call".to_string(),
        namespace: None,
        name: name.to_string(),
        arguments,
        argument_error: None,
    }
}

fn call_events(call: ModelToolCall) -> Vec<ModelResult<ModelEvent>> {
    vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "tolerance-response".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall { call: call.clone() },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "tolerance-response".to_string(),
            usage: None,
            end_turn: Some(false),
        }),
    ]
}

/// Runs one turn whose first response is `first` and whose second answers
/// "recovered", and returns the error the model read back for the call.
async fn rejected_call_output(
    first: Vec<ModelResult<ModelEvent>>,
    mode: hanafubuki_agent_kernel::AgentSessionMode,
) -> serde_json::Value {
    let model = ScriptedEventModel::new([first, deterministic_assistant_events("recovered")]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let mut session_config = config();
    session_config.mode = mode;
    let session = service.create_session(session_config).unwrap();

    let outcome = service
        .start_turn(session.id, "Call a tool".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "recovered");
    // Nothing ran and nothing asked for approval: the turn would wait.
    assert!(tools.calls().is_empty());
    assert_eq!(tools.approved_calls(), 0);
    // The call and its output stay paired in history and in the request.
    let history = test_session_view(&service, session.id)
        .unwrap()
        .model_context;
    assert!(history.iter().any(|item| matches!(
        item,
        ConversationItem::ToolCall { call_id, .. } if call_id == "tolerance-call"
    )));
    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.iter().any(|item| matches!(
        item,
        ModelInputItem::ToolCall { call_id, .. } if call_id == "tolerance-call"
    )));
    requests[1]
        .input
        .iter()
        .find_map(|item| match item {
            ModelInputItem::ToolOutput { call_id, output } if call_id == "tolerance-call" => {
                assert!(output.is_error);
                output.structured_content.clone()
            }
            _ => None,
        })
        .expect("the rejected call has its output")
}

#[tokio::test]
async fn unknown_and_hidden_tools_get_one_identical_error_and_never_run() {
    let unknown = rejected_call_output(
        call_events(call("unknown_mutation", json!({}))),
        hanafubuki_agent_kernel::AgentSessionMode::Default,
    )
    .await;
    assert_eq!(
        unknown,
        json!({ "error": "no tool named `unknown_mutation` is available in this step" })
    );
    // Plan mode hides mutations, deferred and direct alike: a call names
    // them exactly as it would a tool that does not exist.
    for hidden in ["configure_instance", "control_instance"] {
        let output = rejected_call_output(
            call_events(call(
                hidden,
                json!({ "instanceId": "instance-a", "action": "start" }),
            )),
            hanafubuki_agent_kernel::AgentSessionMode::Plan,
        )
        .await;
        assert_eq!(
            output,
            json!({ "error": format!("no tool named `{hidden}` is available in this step") })
        );
    }
}

#[tokio::test]
async fn a_malformed_tool_search_is_a_field_named_error() {
    for (arguments, expected) in [
        (
            json!([1, 2]),
            "invalid tool_search arguments: the arguments must be a JSON object with a `query` string",
        ),
        (
            json!({ "limit": 3 }),
            "invalid tool_search arguments: `query` is required",
        ),
        (
            json!({ "query": 7 }),
            "invalid tool_search arguments: `query` must be a string",
        ),
    ] {
        let output = rejected_call_output(
            call_events(call("tool_search", arguments)),
            hanafubuki_agent_kernel::AgentSessionMode::Default,
        )
        .await;
        assert_eq!(output, json!({ "error": expected }));
    }
}

#[tokio::test]
async fn oversized_arguments_are_answered_without_running_the_tool() {
    let output = rejected_call_output(
        call_events(call(
            "control_instance",
            json!({ "instanceId": "instance-a", "action": "x".repeat(70 * 1024) }),
        )),
        hanafubuki_agent_kernel::AgentSessionMode::Default,
    )
    .await;
    let error = output["error"].as_str().unwrap();
    assert!(
        error.starts_with("the tool arguments could not be used: the arguments are ")
            && error.ends_with("bytes, over the 65536-byte limit"),
        "{error}"
    );
    assert_eq!(output["tool"], "control_instance");
    assert_eq!(
        output["receivedArguments"].as_str().unwrap().len(),
        hanafubuki_agent_model_api::MAX_TOOL_ARGUMENT_PREVIEW_BYTES
    );
}

#[tokio::test]
async fn unparseable_arguments_are_answered_with_a_preview_and_no_approval() {
    // A provider decoder hands the kernel `{}` and the parse error; the
    // mutation it names is neither approved nor run.
    let mut unparseable = call("control_instance", json!({}));
    unparseable.argument_error = Some(hanafubuki_agent_model_api::ModelToolArgumentError::new(
        "expected `,` or `}` at line 1 column 20",
        "{\"instanceId\": \"a\" \"action\"",
    ));
    let output = rejected_call_output(
        call_events(unparseable),
        hanafubuki_agent_kernel::AgentSessionMode::Default,
    )
    .await;
    assert_eq!(
        output,
        json!({
            "error": "the tool arguments could not be used: expected `,` or `}` at line 1 column 20",
            "tool": "control_instance",
            "receivedArguments": "{\"instanceId\": \"a\" \"action\"",
            "retry": "call the tool again with one JSON object that matches its schema"
        })
    );
}

#[tokio::test]
async fn an_incomplete_response_keeps_its_partial_answer() {
    let model = ScriptedEventModel::new([vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "partial".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning {
                item_id: "thinking".to_string(),
                summary: Vec::new(),
            },
        }),
        Ok(ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id: "answer".to_string(),
                phase: None,
                text: String::new(),
            },
        }),
        Ok(ModelEvent::AssistantTextDelta {
            item_id: "answer".to_string(),
            delta: "The partial ans".to_string(),
        }),
        Ok(ModelEvent::ResponseIncomplete {
            response_id: "partial".to_string(),
            usage: None,
            reason: "max_output_tokens".to_string(),
        }),
    ]]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "Explain".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "The partial ans");
}

#[tokio::test]
async fn an_incomplete_response_with_nothing_streamed_still_fails() {
    let model = ScriptedEventModel::new([vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "empty".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::ResponseIncomplete {
            response_id: "empty".to_string(),
            usage: None,
            reason: "max_output_tokens".to_string(),
        }),
    ]]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let result = service.start_turn(session.id, "Explain".to_string()).await;

    assert!(matches!(result, Err(AgentError::Protocol(_))), "{result:?}");
}

#[tokio::test]
async fn items_completed_without_being_announced_are_kept() {
    let model = ScriptedEventModel::new([vec![
        Ok(ModelEvent::ResponseStarted {
            response_id: "unannounced".to_string(),
            metadata: ModelResponseMetadata::default(),
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: "thinking".to_string(),
                summary: vec!["Considered it".to_string()],
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Other {
                item_id: None,
                item_type: "web_search_call".to_string(),
            },
        }),
        Ok(ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id: "answer".to_string(),
                phase: None,
                text: "Done without an added event.".to_string(),
            },
        }),
        Ok(ModelEvent::ResponseCompleted {
            response_id: "unannounced".to_string(),
            usage: None,
            end_turn: Some(true),
        }),
    ]]);
    let tools = FakeTools::new([]).await;
    let service = AgentService::new(Arc::new(model), tools.registries());
    let session = service.create_session(config()).unwrap();

    let outcome = service
        .start_turn(session.id, "Answer".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Done without an added event.");
}
