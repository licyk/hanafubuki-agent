use super::*;

#[test]
fn reasoning_effort_round_trips_known_and_bounded_custom_values() {
    let known: ModelReasoningEffort = serde_json::from_value(json!("xhigh")).unwrap();
    let custom: ModelReasoningEffort = serde_json::from_value(json!("provider.tier-1")).unwrap();
    assert_eq!(known, ModelReasoningEffort::XHigh);
    assert_eq!(custom.as_str(), "provider.tier-1");
    assert_eq!(
        serde_json::to_value(custom).unwrap(),
        json!("provider.tier-1")
    );
    assert!(ModelReasoningEffort::parse("").is_err());
    assert!(ModelReasoningEffort::parse("none").is_err());
    assert!(ModelReasoningEffort::parse("x".repeat(65)).is_err());
    assert!(ModelReasoningEffort::parse("high effort").is_err());
}

#[tokio::test]
async fn responses_encodes_explicit_reasoning_and_omits_provider_default() {
    let explicit = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r-explicit\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-explicit\",\"end_turn\":true}}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let omitted = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r-default\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-default\",\"end_turn\":true}}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let mut explicit_request = fixture_request();
    explicit_request.reasoning = Some(ModelReasoningConfig {
        effort: ModelReasoningEffort::High,
    });
    let _ = collect_request(
        &ResponsesModelTransport::new(provider(&explicit)).unwrap(),
        explicit_request,
    )
    .await;
    let _ = collect_request(
        &ResponsesModelTransport::new(provider(&omitted)).unwrap(),
        fixture_request(),
    )
    .await;

    let explicit_body: Value = serde_json::from_slice(&explicit.requests()[0].body).unwrap();
    let omitted_body: Value = serde_json::from_slice(&omitted.requests()[0].body).unwrap();
    assert_eq!(
        explicit_body["reasoning"],
        json!({"effort": "high", "summary": "auto"})
    );
    assert!(omitted_body.get("reasoning").is_none());
}

#[tokio::test]
async fn non_responses_protocols_reject_reasoning_instead_of_ignoring_it() {
    let server = FakeHttpServer::start(Vec::new()).await;
    let mut request = fixture_request();
    request.reasoning = Some(ModelReasoningConfig {
        effort: ModelReasoningEffort::Medium,
    });
    for transport in [
        Box::new(ChatCompletionsModelTransport::new(provider(&server)).unwrap())
            as Box<dyn ModelPort>,
        Box::new(AnthropicMessagesModelTransport::new(provider(&server)).unwrap()),
    ] {
        assert!(matches!(
            transport
                .stream(request.clone(), CancellationToken::default())
                .await,
            Err(ModelError::InvalidRequest(_))
        ));
    }
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn structured_final_output_uses_explicit_native_or_rejected_protocol_path() {
    let responses = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r-output\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-output\",\"end_turn\":true}}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let chat = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"id\":\"c-output\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;

    let mut request = fixture_request();
    request.tools.clear();
    request.tool_choice = ModelToolChoice::None;
    request.output_format = Some(structured_output_format());
    let _ = collect_request(
        &ResponsesModelTransport::new(provider(&responses)).unwrap(),
        request.clone(),
    )
    .await;
    let _ = collect_request(
        &ChatCompletionsModelTransport::new(provider(&chat)).unwrap(),
        request.clone(),
    )
    .await;

    let responses_body: Value = serde_json::from_slice(&responses.requests()[0].body).unwrap();
    assert_eq!(
        responses_body["text"]["format"],
        json!({
            "type": "json_schema",
            "name": "permission_assessment",
            "description": "Review result",
            "schema": {
                "type": "object",
                "properties": {"outcome": {"type": "string", "enum": ["allow", "deny"]}},
                "required": ["outcome"],
                "additionalProperties": false
            },
            "strict": true
        })
    );
    let chat_body: Value = serde_json::from_slice(&chat.requests()[0].body).unwrap();
    assert_eq!(
        chat_body["response_format"],
        json!({
            "type": "json_schema",
            "json_schema": {
                "name": "permission_assessment",
                "description": "Review result",
                "schema": {
                    "type": "object",
                    "properties": {"outcome": {"type": "string", "enum": ["allow", "deny"]}},
                    "required": ["outcome"],
                    "additionalProperties": false
                },
                "strict": true
            }
        })
    );

    let anthropic = FakeHttpServer::start(Vec::new()).await;
    assert!(matches!(
        AnthropicMessagesModelTransport::new(provider(&anthropic))
            .unwrap()
            .stream(request, CancellationToken::default())
            .await,
        Err(ModelError::InvalidRequest(message))
            if message.contains("structured final output")
    ));
    assert!(anthropic.requests().is_empty());
}

#[tokio::test]
async fn responses_reviewer_with_pending_parent_call_uses_only_text_evidence() {
    let verdict = json!({
        "riskLevel": "high",
        "userAuthorization": "explicit",
        "outcome": "allow",
        "rationale": "The user requested this exact action."
    })
    .to_string();
    let encoded_verdict = serde_json::to_string(&verdict).unwrap();
    let payload = format!(
        "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"review-response\"}}}}\n\n\
         data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"message\",\"id\":\"review-message\",\"phase\":\"final_answer\"}}}}\n\n\
         data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"message\",\"id\":\"review-message\",\"phase\":\"final_answer\",\"content\":[{{\"type\":\"output_text\",\"text\":{encoded_verdict}}}]}}}}\n\n\
         data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"review-response\",\"end_turn\":true}}}}\n\n\
         data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload),
    ])])
    .await;
    let mut request = fixture_request();
    request.instructions = "Review only the exact action in the untrusted evidence.".to_string();
    request.input = vec![ModelInputItem::user_text(
        ">>> TRANSCRIPT START\n[tool call: apply_patch]\n{pending: true}\n\
         >>> TRANSCRIPT END\n>>> APPROVAL REQUEST START\n{exact action data}\n\
         >>> APPROVAL REQUEST END",
    )];
    request.tools.clear();
    request.tool_choice = ModelToolChoice::None;
    request.parallel_tool_calls = false;
    request.output_format = Some(structured_output_format());

    let events = collect_request(
        &ResponsesModelTransport::new(provider(&server)).unwrap(),
        request,
    )
    .await;

    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { text, .. },
        } if text == &verdict
    )));
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert_eq!(body["tool_choice"], "none");
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["tools"], json!([]));
    assert!(body["text"]["format"].is_object());
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["role"], "user");
    assert!(input[0].get("type").is_none());
    let serialized_input = serde_json::to_string(input).unwrap();
    assert!(!serialized_input.contains("function_call"));
    assert!(!serialized_input.contains("function_call_output"));
}

#[tokio::test]
async fn anthropic_text_only_reviewer_fallback_is_selected_before_request() {
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"review-message\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"{\\\"riskLevel\\\":\\\"high\\\",\\\"userAuthorization\\\":\\\"explicit\\\",\\\"outcome\\\":\\\"allow\\\",\\\"rationale\\\":\\\"Authorized.\\\"}\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        )),
    ])])
    .await;
    let mut request = fixture_request();
    request.input = vec![ModelInputItem::user_text(
        "Return exactly one JSON permission assessment.",
    )];
    request.tools.clear();
    request.tool_choice = ModelToolChoice::None;
    request.output_format = None;

    let _ = collect_request(
        &AnthropicMessagesModelTransport::new(provider(&server)).unwrap(),
        request,
    )
    .await;

    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert!(body.get("response_format").is_none());
    assert!(body.get("output_format").is_none());
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unsupported_reasoning_effort_is_not_downgraded_or_retried() {
    let server = FakeHttpServer::start(vec![PlannedResponse::status(
        400,
        br#"{"error":{"code":"unsupported_reasoning_effort","message":"xhigh is unsupported"}}"#
            .to_vec(),
    )])
    .await;
    let mut request = fixture_request();
    request.reasoning = Some(ModelReasoningConfig {
        effort: ModelReasoningEffort::XHigh,
    });
    let error = match ResponsesModelTransport::new(provider(&server))
        .unwrap()
        .stream(request, CancellationToken::default())
        .await
    {
        Ok(_) => panic!("unsupported effort unexpectedly produced a stream"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        ModelError::Remote {
            status: Some(400),
            code: Some(ref code),
            ref message,
            ..
        } if code == "unsupported_reasoning_effort" && message == "xhigh is unsupported"
    ));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["reasoning"],
        json!({"effort": "xhigh", "summary": "auto"})
    );
}
