use super::*;

#[tokio::test]
async fn user_images_lower_to_each_protocols_exact_native_shape() {
    let responses = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r-image\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-image\",\"end_turn\":true}}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let chat = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"id\":\"c-image\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let anthropic = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m-image\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        )),
    ])])
    .await;
    let mut request = fixture_request();
    request.input = vec![ModelInputItem::UserMessage {
        content: vec![
            ModelUserContent::InputText {
                text: "look".into(),
            },
            ModelUserContent::InputImage {
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
                detail: ModelImageDetail::High,
            },
        ],
    }];
    request.tools.clear();

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
    let _ = collect_request(
        &AnthropicMessagesModelTransport::new(provider(&anthropic)).unwrap(),
        request,
    )
    .await;

    let responses: Value = serde_json::from_slice(&responses.requests()[0].body).unwrap();
    assert_eq!(
        responses["input"][0]["content"][0],
        json!({"type":"input_text","text":"look"})
    );
    assert_eq!(
        responses["input"][0]["content"][1],
        json!({
            "type":"input_image", "image_url":"data:image/png;base64,AQID", "detail":"high"
        })
    );
    let chat: Value = serde_json::from_slice(&chat.requests()[0].body).unwrap();
    assert_eq!(
        chat["messages"][1]["content"][1],
        json!({
            "type":"image_url", "image_url":{"url":"data:image/png;base64,AQID","detail":"high"}
        })
    );
    let anthropic: Value = serde_json::from_slice(&anthropic.requests()[0].body).unwrap();
    assert_eq!(
        anthropic["messages"][0]["content"][1],
        json!({
            "type":"image", "source":{"type":"base64","media_type":"image/png","data":"AQID"}
        })
    );
}

#[tokio::test]
async fn sanitized_tool_images_remain_text_tool_results_in_all_protocols() {
    let responses = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r-sanitized\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-sanitized\",\"end_turn\":true}}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let chat = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"id\":\"c-sanitized\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let anthropic = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m-sanitized\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        )),
    ])])
    .await;
    let mut request = fixture_request();
    request.tools.clear();
    request.input = vec![
        ModelInputItem::ToolCall {
            call_id: "image-call".into(),
            namespace: None,
            name: "view_image".into(),
            arguments: json!({"path": "fixture.png"}),
        },
        ModelInputItem::ToolOutput {
            call_id: "image-call".into(),
            output: ToolOutput {
                content: vec![
                    ToolOutputContent::Text {
                        text: "before".into(),
                    },
                    ToolOutputContent::Image {
                        mime_type: "image/png".into(),
                        data: "AQID".into(),
                        detail: None,
                    },
                ],
                structured_content: Some(json!({"kept": true})),
                is_error: false,
            },
        },
    ];
    let sanitized = hanafubuki_agent_model_api::sanitize_images_for_unsupported_input(&request).0;

    let _ = collect_request(
        &ResponsesModelTransport::new(provider(&responses)).unwrap(),
        sanitized.clone(),
    )
    .await;
    let _ = collect_request(
        &ChatCompletionsModelTransport::new(provider(&chat)).unwrap(),
        sanitized.clone(),
    )
    .await;
    let _ = collect_request(
        &AnthropicMessagesModelTransport::new(provider(&anthropic)).unwrap(),
        sanitized,
    )
    .await;

    let marker = hanafubuki_agent_model_api::IMAGE_INPUT_OMITTED_PLACEHOLDER;
    let responses: Value = serde_json::from_slice(&responses.requests()[0].body).unwrap();
    let response_output = responses["input"][1]["output"].as_array().unwrap();
    assert!(response_output.iter().any(|part| part["text"] == marker));
    assert!(!response_output
        .iter()
        .any(|part| part["type"] == "input_image"));
    let chat: Value = serde_json::from_slice(&chat.requests()[0].body).unwrap();
    let chat_messages = chat["messages"].as_array().unwrap();
    assert_eq!(chat_messages.last().unwrap()["role"], "tool");
    assert_eq!(chat_messages.last().unwrap()["tool_call_id"], "image-call");
    assert!(chat_messages.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains(marker));
    assert!(!chat_messages
        .iter()
        .any(|message| message["role"] == "user"));
    let anthropic: Value = serde_json::from_slice(&anthropic.requests()[0].body).unwrap();
    let tool_result = &anthropic["messages"][1]["content"][0];
    assert_eq!(tool_result["type"], "tool_result");
    assert_eq!(tool_result["tool_use_id"], "image-call");
    assert!(tool_result["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["text"] == marker));
    assert!(!tool_result["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["type"] == "image"));
}

#[tokio::test]
async fn canonically_truncated_multimodal_output_keeps_its_marker_in_all_protocols() {
    let responses = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r-truncated\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-truncated\",\"end_turn\":true}}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let chat = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"id\":\"c-truncated\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let anthropic = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m-truncated\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        )),
    ])])
    .await;
    let marker = concat!(
        "<hanafubuki-tool-output-truncation>",
        "{\"originalBytes\":8192,\"retainedBytes\":512,\"omittedBytes\":7680,",
        "\"omittedItems\":2,\"omittedContentTypes\":[\"structured\",\"text\"]}",
        "</hanafubuki-tool-output-truncation>"
    );
    let mut request = fixture_request();
    request.tools.clear();
    request.input = vec![
        ModelInputItem::ToolCall {
            call_id: "truncated-call".into(),
            namespace: None,
            name: "capture".into(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: "truncated-call".into(),
            output: ToolOutput {
                content: vec![
                    ToolOutputContent::Text {
                        text: "HEAD\n… <7000 bytes omitted> …\nTAIL".into(),
                    },
                    ToolOutputContent::Image {
                        mime_type: "image/png".into(),
                        data: "AQID".into(),
                        detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::High),
                    },
                    ToolOutputContent::Audio {
                        mime_type: "audio/wav".into(),
                        data: "BAUG".into(),
                    },
                    ToolOutputContent::Text {
                        text: marker.into(),
                    },
                ],
                structured_content: None,
                is_error: true,
            },
        },
    ];

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
    let _ = collect_request(
        &AnthropicMessagesModelTransport::new(provider(&anthropic)).unwrap(),
        request,
    )
    .await;

    let responses: Value = serde_json::from_slice(&responses.requests()[0].body).unwrap();
    let response_output = responses["input"][1]["output"].as_array().unwrap();
    assert!(response_output.iter().any(|part| part["text"] == marker));
    assert!(response_output
        .iter()
        .any(|part| part["type"] == "input_image"));
    assert!(response_output
        .iter()
        .any(|part| part["type"] == "input_audio"));

    let chat: Value = serde_json::from_slice(&chat.requests()[0].body).unwrap();
    let chat_messages = chat["messages"].as_array().unwrap();
    let tool_message = chat_messages
        .iter()
        .find(|message| message["role"] == "tool")
        .unwrap();
    let chat_tool_content: Value =
        serde_json::from_str(tool_message["content"].as_str().unwrap()).unwrap();
    assert!(chat_tool_content["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part == marker));
    assert!(chat_tool_content["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["type"] == "unsupported_audio" && part["mimeType"] == "audio/wav"));
    assert!(chat_messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| part["type"] == "image_url"))
    }));

    let anthropic: Value = serde_json::from_slice(&anthropic.requests()[0].body).unwrap();
    let tool_result = &anthropic["messages"][1]["content"][0];
    assert_eq!(tool_result["is_error"], true);
    assert!(tool_result["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["text"] == marker));
    assert!(tool_result["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["type"] == "image"));
    assert!(tool_result["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["text"]
            .as_str()
            .is_some_and(|text| text.contains("unsupported_audio"))));
}

#[tokio::test]
async fn required_tool_choice_is_encoded_exactly_for_all_three_protocols() {
    let responses_payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"response-required\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-required\",\"end_turn\":true}}\n\n",
        "data: [DONE]\n\n"
    );
    let chat_payload = concat!(
        "data: {\"id\":\"chat-required\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let anthropic_payload = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"anthropic-required\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    let responses_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(responses_payload.as_bytes()),
    ])])
    .await;
    let chat_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(chat_payload.as_bytes()),
    ])])
    .await;
    let anthropic_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(anthropic_payload.as_bytes()),
    ])])
    .await;

    let mut request = fixture_request();
    request.tool_choice = ModelToolChoice::Required {
        name: "list_instances".to_string(),
    };
    let responses = ResponsesModelTransport::new(provider(&responses_server)).unwrap();
    let chat = ChatCompletionsModelTransport::new(provider(&chat_server)).unwrap();
    let anthropic = AnthropicMessagesModelTransport::new(provider(&anthropic_server)).unwrap();
    let _ = collect_request(&responses, request.clone()).await;
    let _ = collect_request(&chat, request.clone()).await;
    let _ = collect_request(&anthropic, request).await;

    let responses_body: Value =
        serde_json::from_slice(&responses_server.requests()[0].body).unwrap();
    let chat_body: Value = serde_json::from_slice(&chat_server.requests()[0].body).unwrap();
    let anthropic_body: Value =
        serde_json::from_slice(&anthropic_server.requests()[0].body).unwrap();
    assert_eq!(
        responses_body["tool_choice"],
        json!({"type": "function", "name": "list_instances"})
    );
    assert_eq!(
        chat_body["tool_choice"],
        json!({"type": "function", "function": {"name": "list_instances"}})
    );
    assert_eq!(
        anthropic_body["tool_choice"],
        json!({
            "type": "tool",
            "name": "list_instances",
            "disable_parallel_tool_use": true
        })
    );
}

/// A namespace spec is native on Responses. Chat Completions and Anthropic
/// Messages take no namespace and do not flatten one: the Agent's tool plan
/// names every tool for the protocol, so a namespace reaching them is
/// refused before anything is sent.
#[tokio::test]
async fn a_namespace_spec_is_native_on_responses_and_refused_by_chat_and_anthropic() {
    let responses_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"end_turn\":true}}\n\ndata: [DONE]\n\n",
        ),
    ])])
    .await;
    let unused = FakeHttpServer::start(Vec::new()).await;
    let schema = json!({
        "type": "object",
        "properties": {
            "value": {"type": "string"}
        },
        "required": ["value"],
        "additionalProperties": false
    });
    let mut request = fixture_request();
    request.tools = vec![ToolSpec::Namespace(NamespaceToolSpec {
        name: ToolName::parse("mcp__fixture").unwrap(),
        description: "Bounded instructions from the fixture MCP server.".into(),
        tools: vec![FunctionToolSpec {
            name: ToolName::parse("echo").unwrap(),
            description: "Echo through the frozen MCP connection.".into(),
            input_schema: schema.clone(),
            strict: false,
            deferred: false,
        }],
    })];
    let _ = collect_request(
        &ResponsesModelTransport::new(provider(&responses_server)).unwrap(),
        request.clone(),
    )
    .await;
    let responses: Value = serde_json::from_slice(&responses_server.requests()[0].body).unwrap();
    assert_eq!(responses["tools"][0]["type"], "namespace");
    assert_eq!(responses["tools"][0]["name"], "mcp__fixture");
    assert_eq!(responses["tools"][0]["tools"][0]["name"], "echo");
    assert_eq!(responses["tools"][0]["tools"][0]["parameters"], schema);

    let chat = ChatCompletionsModelTransport::new(provider(&unused)).unwrap();
    let anthropic = AnthropicMessagesModelTransport::new(provider(&unused)).unwrap();
    for transport in [&chat as &dyn ModelPort, &anthropic] {
        let refused = transport
            .stream(request.clone(), CancellationToken::default())
            .await;
        assert!(matches!(
            refused,
            Err(ModelError::InvalidRequest(message)) if message.contains("namespace tools")
        ));
    }
    assert!(unused.requests().is_empty());
}

/// A call the provider made inside a namespace is replayed with its
/// namespace on Responses; a protocol without namespaces (a session that
/// moved off Responses) replays it flattened to `namespace__name`.
#[tokio::test]
async fn a_namespaced_history_call_replays_natively_on_responses_and_flattened_elsewhere() {
    let responses_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"end_turn\":true}}\n\ndata: [DONE]\n\n",
        ),
    ])])
    .await;
    let chat_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(
            b"data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
        ),
    ])])
    .await;
    let anthropic_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(
            b"data: {\"type\":\"message_start\",\"message\":{\"id\":\"a\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\ndata: {\"type\":\"message_stop\"}\n\n",
        ),
    ])])
    .await;
    let mut request = fixture_request();
    request.tools.clear();
    request.tool_choice = ModelToolChoice::Auto;
    request.input = vec![
        ModelInputItem::user_text("Echo it."),
        ModelInputItem::ToolCall {
            call_id: "call-1".into(),
            namespace: Some("mcp__fixture".into()),
            name: "echo".into(),
            arguments: json!({"value": "hi"}),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-1".into(),
            output: ToolOutput::structured(json!({"value": "hi"}), false),
        },
    ];
    let _ = collect_request(
        &ResponsesModelTransport::new(provider(&responses_server)).unwrap(),
        request.clone(),
    )
    .await;
    let _ = collect_request(
        &ChatCompletionsModelTransport::new(provider(&chat_server)).unwrap(),
        request.clone(),
    )
    .await;
    let _ = collect_request(
        &AnthropicMessagesModelTransport::new(provider(&anthropic_server)).unwrap(),
        request,
    )
    .await;
    let responses: Value = serde_json::from_slice(&responses_server.requests()[0].body).unwrap();
    let chat: Value = serde_json::from_slice(&chat_server.requests()[0].body).unwrap();
    let anthropic: Value = serde_json::from_slice(&anthropic_server.requests()[0].body).unwrap();
    let call = &responses["input"][1];
    assert_eq!(call["type"], "function_call");
    assert_eq!(call["namespace"], "mcp__fixture");
    assert_eq!(call["name"], "echo");
    let chat_call = chat["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|message| message.get("tool_calls"))
        .unwrap();
    assert_eq!(chat_call[0]["function"]["name"], "mcp__fixture__echo");
    let anthropic_call = anthropic["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
        .find(|part| part["type"] == "tool_use")
        .unwrap();
    assert_eq!(anthropic_call["name"], "mcp__fixture__echo");
}

#[tokio::test]
async fn local_summarization_uses_the_exact_tool_free_shape_in_all_three_protocols() {
    let responses_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r-compact\"}}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r-compact\",\"end_turn\":true}}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let chat_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"id\":\"c-compact\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        )),
    ])])
    .await;
    let anthropic_server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"a-compact\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        )),
    ])])
    .await;
    let prompt = "Summarize the conversation for a replacement context window.";
    let mut request = fixture_request();
    request.input = vec![ModelInputItem::user_text(prompt)];
    request.tools.clear();
    request.tool_choice = ModelToolChoice::None;
    request.parallel_tool_calls = false;

    let _ = collect_request(
        &ResponsesModelTransport::new(provider(&responses_server)).unwrap(),
        request.clone(),
    )
    .await;
    let _ = collect_request(
        &ChatCompletionsModelTransport::new(provider(&chat_server)).unwrap(),
        request.clone(),
    )
    .await;
    let _ = collect_request(
        &AnthropicMessagesModelTransport::new(provider(&anthropic_server)).unwrap(),
        request,
    )
    .await;

    let responses: Value = serde_json::from_slice(&responses_server.requests()[0].body).unwrap();
    assert_eq!(responses["instructions"], "Use typed tools.");
    assert_eq!(
        responses["input"],
        json!([{ "role": "user", "content": [{"type": "input_text", "text": prompt}] }])
    );
    assert_eq!(responses["tools"], json!([]));
    assert_eq!(responses["tool_choice"], "none");
    assert_eq!(responses["parallel_tool_calls"], false);

    let chat: Value = serde_json::from_slice(&chat_server.requests()[0].body).unwrap();
    assert_eq!(
        chat["messages"],
        json!([
            {"role": "system", "content": "Use typed tools."},
            {"role": "user", "content": [{"type": "text", "text": prompt}]}
        ])
    );
    assert!(chat.get("tools").is_none());
    assert!(chat.get("tool_choice").is_none());

    let anthropic: Value = serde_json::from_slice(&anthropic_server.requests()[0].body).unwrap();
    assert_eq!(anthropic["system"], "Use typed tools.");
    assert_eq!(
        anthropic["messages"],
        json!([{ "role": "user", "content": [{"type": "text", "text": prompt}] }])
    );
    assert!(anthropic.get("tools").is_none());
    assert!(anthropic.get("tool_choice").is_none());
}
