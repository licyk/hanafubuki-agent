use super::*;

#[tokio::test]
async fn responses_lowers_extended_tools_and_roundtrips_native_tool_search_history() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"response-search\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"tool_search_call\",\"id\":\"search-item\",\"call_id\":\"search-1\",\"execution\":\"client\",\"arguments\":{\"query\":\"calendar\"}}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"tool_search_call\",\"call_id\":\"search-1\",\"execution\":\"client\",\"arguments\":{\"query\":\"calendar\"}}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-search\",\"end_turn\":false}}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let mut request = fixture_request();
    let deferred = FunctionToolSpec {
        name: ToolName::parse("create").unwrap(),
        description: "Create a calendar event".into(),
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
        strict: false,
        deferred: true,
    };
    request.tools = vec![
        ToolSpec::Namespace(NamespaceToolSpec {
            name: ToolName::parse("calendar").unwrap(),
            description: "Calendar tools".into(),
            tools: vec![deferred.clone()],
        }),
        ToolSpec::ToolSearch(ToolSearchSpec {
            execution: "client".into(),
            description: "Search tools".into(),
            parameters: json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}),
        }),
        ToolSpec::HostedWebSearch(HostedWebSearchSpec {
            external_web_access: Some(false),
            indexed_web_access: Some(true),
            allowed_domains: Some(vec!["example.com".into()]),
            search_context_size: Some("low".into()),
        }),
        ToolSpec::Freeform(FreeformToolSpec {
            name: ToolName::parse("render_dsl").unwrap(),
            description: "Render DSL".into(),
            format: FreeformToolFormat {
                format_type: "grammar".into(),
                syntax: "lark".into(),
                definition: "start: WORD".into(),
            },
        }),
    ];
    let events = collect_request(&transport, request).await;
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemAdded { item: ModelOutputItem::ToolCall { call } }
            if call.item_id == "search-item"
                && call.name == hanafubuki_agent_model_api::NATIVE_TOOL_SEARCH_NAME
                && call.origin() == hanafubuki_agent_model_api::ModelToolCallOrigin::ToolSearch
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted { item: ModelOutputItem::ToolCall { call } }
            if call.item_id.starts_with("temporary-tool-item-")
                && call.item_id != call.call_id
                && call.name == hanafubuki_agent_model_api::NATIVE_TOOL_SEARCH_NAME
                && call.call_id == "search-1"
                && call.arguments == json!({"query":"calendar"})
    )));
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert_eq!(body["tools"][0]["type"], "namespace");
    assert_eq!(body["tools"][0]["tools"][0]["defer_loading"], true);
    assert_eq!(body["tools"][1]["type"], "tool_search");
    assert_eq!(body["tools"][2]["type"], "web_search");
    assert_eq!(body["tools"][3]["type"], "custom");
}

#[tokio::test]
async fn responses_preserves_typed_multimodal_and_structured_tool_output_together() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"response-output\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-output\",\"end_turn\":true}}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let mut request = fixture_request();
    request.input = vec![
        ModelInputItem::ToolCall {
            call_id: "typed-output".into(),
            namespace: None,
            name: "capture".into(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: "typed-output".into(),
            output: ToolOutput {
                content: vec![
                    ToolOutputContent::Text {
                        text: "preview".into(),
                    },
                    ToolOutputContent::Image {
                        mime_type: "image/png".into(),
                        data: "AA==".into(),
                        detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::High),
                    },
                ],
                structured_content: Some(json!({"width": 1})),
                is_error: false,
            },
        },
    ];
    let _ = collect_request(&transport, request).await;
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let output = body["input"][1]["output"].as_array().unwrap();
    assert_eq!(output[0], json!({"type":"input_text","text":"preview"}));
    assert_eq!(
        output[1],
        json!({"type":"input_image","image_url":"data:image/png;base64,AA==","detail":"high"})
    );
    assert_eq!(output[2]["type"], "input_text");
    assert_eq!(output[2]["text"], "{\"width\":1}");
}

#[tokio::test]
async fn responses_roundtrips_freeform_custom_tool_calls_and_outputs() {
    let patch = "*** Begin Patch\n*** Add File: note.txt\n+hello\n*** End Patch";
    let payload = format!(
        "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"response-custom\"}}}}\n\n\
         data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"custom_tool_call\",\"id\":\"custom-item\",\"call_id\":\"custom-call\",\"name\":\"apply_patch\",\"input\":\"\"}}}}\n\n\
         data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"custom_tool_call\",\"id\":\"custom-item\",\"call_id\":\"custom-call\",\"name\":\"apply_patch\",\"input\":{}}}}}\n\n\
         data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"response-custom\",\"end_turn\":false}}}}\n\n\
         data: [DONE]\n\n",
        serde_json::to_string(patch).unwrap()
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let mut request = fixture_request();
    request.input.push(ModelInputItem::CustomToolCall {
        call_id: "prior-custom".into(),
        name: "apply_patch".into(),
        input: patch.into(),
    });
    request.input.push(ModelInputItem::CustomToolOutput {
        call_id: "prior-custom".into(),
        output: ToolOutput::structured(json!({"status": "ok"}), false),
    });
    request.tools = vec![ToolSpec::Freeform(FreeformToolSpec {
        name: ToolName::parse("apply_patch").unwrap(),
        description: "Apply a patch".into(),
        format: FreeformToolFormat {
            format_type: "grammar".into(),
            syntax: "lark".into(),
            definition: "start: /(.|\\n)+/".into(),
        },
    })];

    let events = collect_request(&transport, request).await;
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call }
        } if call.name == "apply_patch"
            && call.arguments == json!(patch)
            && call.origin() == hanafubuki_agent_model_api::ModelToolCallOrigin::Freeform
    )));
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert!(body["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["type"] == "custom_tool_call" && item["call_id"] == "prior-custom" }));
    assert!(body["input"].as_array().unwrap().iter().any(|item| {
        item["type"] == "custom_tool_call_output" && item["call_id"] == "prior-custom"
    }));
}

/// A call inside a native namespace decodes into its namespace and its name
/// as sent, never one joined string; a call without one has no namespace.
#[tokio::test]
async fn responses_decodes_a_namespaced_call_into_namespace_and_name() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"response-ns\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"item-1\",\"call_id\":\"call-1\",\"namespace\":\"mcp__fixture\",\"name\":\"echo\",\"arguments\":\"\"}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"item-1\",\"call_id\":\"call-1\",\"namespace\":\"mcp__fixture\",\"name\":\"echo\",\"arguments\":\"{\\\"value\\\":\\\"hi\\\"}\"}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"item-2\",\"call_id\":\"call-2\",\"namespace\":\"\",\"name\":\"list_tasks\",\"arguments\":\"{}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"response-ns\",\"end_turn\":false}}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await;
    let calls = events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::OutputItemAdded {
                item: ModelOutputItem::ToolCall { call },
            }
            | ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::ToolCall { call },
            } => Some((call.namespace.as_deref(), call.name.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls,
        [
            (Some("mcp__fixture"), "echo"),
            (Some("mcp__fixture"), "echo"),
            (None, "list_tasks"),
        ]
    );
}
