use super::*;

#[tokio::test]
async fn anthropic_preserves_image_inside_the_corresponding_tool_result() {
    let payload = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"anthropic-image\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();
    let mut request = fixture_request();
    request.input = vec![
        ModelInputItem::ToolCall {
            call_id: "image-output".into(),
            namespace: None,
            name: "view_image".into(),
            arguments: json!({"path": "fixture.png"}),
        },
        ModelInputItem::ToolOutput {
            call_id: "image-output".into(),
            output: ToolOutput {
                content: vec![ToolOutputContent::Image {
                    mime_type: "image/png".into(),
                    data: "AA==".into(),
                    detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::High),
                }],
                structured_content: None,
                is_error: false,
            },
        },
    ];

    let _ = collect_request(&transport, request).await;
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(
        body["messages"][1]["content"][0]["content"][0],
        json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "AA=="
            }
        })
    );
}

#[tokio::test]
async fn anthropic_groups_parallel_tool_history_and_tool_free_requests_omit_tool_controls() {
    let completed = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-grouped\",\"usage\":{\"input_tokens\":4,\"output_tokens\":1}}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    let server = FakeHttpServer::start(vec![
        PlannedResponse::sse(vec![ResponseFragment::immediate(completed.as_bytes())]),
        PlannedResponse::sse(vec![ResponseFragment::immediate(completed.as_bytes())]),
    ])
    .await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();
    let mut grouped = fixture_request();
    grouped.input = vec![
        ModelInputItem::AssistantMessage {
            phase: Some(ModelMessagePhase::Commentary),
            text: "using two".into(),
        },
        ModelInputItem::ToolCall {
            call_id: "call-1".into(),
            namespace: None,
            name: "one".into(),
            arguments: json!({"a": 1}),
        },
        ModelInputItem::ToolCall {
            call_id: "call-2".into(),
            namespace: None,
            name: "two".into(),
            arguments: json!({"b": 2}),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-1".into(),
            output: json!({"ok": 1}).into(),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-2".into(),
            output: json!({"ok": 2}).into(),
        },
    ];
    let _ = collect_request(&transport, grouped).await;

    let mut tool_free = fixture_request();
    tool_free.input = vec![ModelInputItem::user_text("compact")];
    tool_free.tools.clear();
    let _ = collect_request(&transport, tool_free).await;

    let requests = server.requests();
    let grouped: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(grouped["messages"][0]["role"], "assistant");
    assert_eq!(
        grouped["messages"][0]["content"].as_array().unwrap().len(),
        3
    );
    assert_eq!(grouped["messages"][1]["role"], "user");
    assert_eq!(
        grouped["messages"][1]["content"].as_array().unwrap().len(),
        2
    );
    let tool_free: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert!(tool_free.get("tools").is_none());
    assert!(tool_free.get("tool_choice").is_none());
}

#[tokio::test]
async fn anthropic_messages_stream_maps_tools_usage_and_authentication() {
    let payload = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-1\",\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Checking\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu-1\",\"name\":\"list_instances\",\"input\":{}}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"scope\\\":\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"all\\\"}\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":18}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await;

    let text_started = events
        .iter()
        .position(|event| {
            matches!(
                event,
                ModelEvent::OutputItemAdded {
                    item: ModelOutputItem::AssistantMessage { .. },
                }
            )
        })
        .unwrap();
    let tool_started = events
        .iter()
        .position(|event| {
            matches!(
                event,
                ModelEvent::OutputItemAdded {
                    item: ModelOutputItem::ToolCall { .. },
                }
            )
        })
        .unwrap();
    assert!(text_started < tool_started);
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { text, .. }
        } if text == "Checking"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call }
        }
            if call.call_id == "toolu-1"
                && call.arguments == json!({"scope": "all"})
    )));
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "msg-1".into(),
            usage: Some(ModelUsage {
                input_tokens: 12,
                output_tokens: 18,
            }),
            end_turn: Some(false),
        })
    );
    let request = &server.requests()[0];
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.headers.get("x-api-key").unwrap(), "provider-secret");
    assert_eq!(
        request.headers.get("anthropic-version").unwrap(),
        "2023-06-01"
    );
    assert!(!request.headers.contains_key("authorization"));
    assert_eq!(
        request.headers.get("x-provider-scope").unwrap(),
        "workspace-a"
    );
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["system"], "Use typed tools.");
    assert_eq!(body["max_tokens"], 16_384);
    assert_eq!(
        body["tool_choice"],
        json!({"type": "auto", "disable_parallel_tool_use": true})
    );
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("catalog metadata"));
    assert!(body["messages"][1]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("instance_chooser"));
    assert!(body["messages"][2]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("untrusted workflow"));
    assert!(body["messages"][3]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("- [in_progress] Implement"));
    assert_eq!(body["messages"][5]["content"][0]["type"], "tool_use");
    assert_eq!(body["messages"][6]["content"][0]["type"], "tool_result");
}
