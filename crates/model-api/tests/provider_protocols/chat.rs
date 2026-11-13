use super::*;

#[tokio::test]
async fn chat_moves_parallel_tool_images_after_all_corresponding_tool_messages() {
    let payload = concat!(
        "data: {\"id\":\"chat-tool-images\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let mut request = fixture_request();
    request.tools.clear();
    request.input = vec![
        ModelInputItem::ToolCall {
            call_id: "image-one".into(),
            namespace: None,
            name: "view_image".into(),
            arguments: json!({"path": "one.png"}),
        },
        ModelInputItem::ToolCall {
            call_id: "image-two".into(),
            namespace: None,
            name: "view_image".into(),
            arguments: json!({"path": "two.png"}),
        },
        ModelInputItem::ToolOutput {
            call_id: "image-one".into(),
            output: ToolOutput {
                content: vec![
                    ToolOutputContent::Text {
                        text: "first preview".into(),
                    },
                    ToolOutputContent::Image {
                        mime_type: "image/png".into(),
                        data: "AQID".into(),
                        detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::High),
                    },
                ],
                structured_content: None,
                is_error: false,
            },
        },
        ModelInputItem::ToolOutput {
            call_id: "image-two".into(),
            output: ToolOutput {
                content: vec![ToolOutputContent::Image {
                    mime_type: "image/webp".into(),
                    data: "BAUG".into(),
                    detail: Some(hanafubuki_agent_tool_contracts::ToolOutputImageDetail::Original),
                }],
                structured_content: None,
                is_error: false,
            },
        },
    ];

    let _ = collect_request(&transport, request).await;
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["system", "assistant", "tool", "tool", "user"]
    );
    assert_eq!(messages[2]["tool_call_id"], "image-one");
    assert_eq!(messages[3]["tool_call_id"], "image-two");
    assert!(!messages[2]["content"].as_str().unwrap().contains("AQID"));
    assert!(!messages[3]["content"].as_str().unwrap().contains("BAUG"));
    assert_eq!(
        messages[4]["content"][0]["text"],
        "[hanafubuki-agent: image output of tool call image-one]"
    );
    assert_eq!(
        messages[4]["content"][1],
        json!({
            "type": "image_url",
            "image_url": {
                "url": "data:image/png;base64,AQID",
                "detail": "high"
            }
        })
    );
    assert_eq!(
        messages[4]["content"][2]["text"],
        "[hanafubuki-agent: image output of tool call image-two]"
    );
    assert_eq!(
        messages[4]["content"][3],
        json!({
            "type": "image_url",
            "image_url": {
                "url": "data:image/webp;base64,BAUG",
                "detail": "high"
            }
        })
    );
}

#[tokio::test]
async fn chat_flushes_synthetic_tool_images_before_the_next_real_user_message() {
    let payload = concat!(
        "data: {\"id\":\"chat-tool-image-boundary\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let mut request = fixture_request();
    request.tools.clear();
    request.input = vec![
        ModelInputItem::ToolCall {
            call_id: "image-boundary".into(),
            namespace: None,
            name: "view_image".into(),
            arguments: json!({"path": "boundary.png"}),
        },
        ModelInputItem::ToolOutput {
            call_id: "image-boundary".into(),
            output: ToolOutput {
                content: vec![ToolOutputContent::Image {
                    mime_type: "image/png".into(),
                    data: "AQID".into(),
                    detail: None,
                }],
                structured_content: None,
                is_error: false,
            },
        },
        ModelInputItem::user_text("real follow-up"),
    ];

    let _ = collect_request(&transport, request).await;
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["system", "assistant", "tool", "user", "user"]
    );
    assert_eq!(messages[3]["content"][1]["type"], "image_url");
    assert_eq!(messages[4]["content"][0]["text"], "real follow-up");
}

#[tokio::test]
async fn chat_completions_stream_maps_text_usage_and_exact_request() {
    let payload = concat!(
        "data: {\"id\":\"chat-1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chat-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"id\":\"chat-1\",\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n"
    );
    let fragments = payload
        .as_bytes()
        .chunks(5)
        .map(|chunk| ResponseFragment::immediate(chunk.to_vec()))
        .collect();
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(fragments)]).await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();

    let events = collect(&transport).await;
    assert!(matches!(
        &events[0],
        ModelEvent::ResponseStarted { response_id, .. } if response_id == "chat-1"
    ));
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "chat-1".into(),
            usage: Some(ModelUsage {
                input_tokens: 9,
                output_tokens: 2,
                ..Default::default()
            }),
            end_turn: Some(true),
        })
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { text, .. }
        } if text == "hello"
    )));

    let requests = server.requests();
    assert_eq!(requests[0].path, "/v1/chat/completions");
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer provider-secret"
    );
    assert_eq!(
        requests[0].headers.get("x-provider-scope").unwrap(),
        "workspace-a"
    );
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["stream"], true);
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(
        body["messages"][0],
        json!({"role": "system", "content": "Use typed tools."})
    );
    // Never `developer`: that role exists only on OpenAI's own endpoint, and a
    // compatible Chat Completions server rejects the whole request with
    // "unknown variant `developer`".
    assert_eq!(body["messages"][1]["role"], "system");
    assert!(body["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("catalog metadata"));
    assert_eq!(body["messages"][2]["role"], "user");
    assert!(body["messages"][2]["content"]
        .as_str()
        .unwrap()
        .contains("instance_chooser"));
    assert_eq!(body["messages"][3]["role"], "user");
    assert!(body["messages"][3]["content"]
        .as_str()
        .unwrap()
        .contains("untrusted workflow"));
    assert!(body["messages"][4]["content"]
        .as_str()
        .unwrap()
        .contains("- [in_progress] Implement"));
    assert_eq!(body["messages"][6]["tool_calls"][0]["id"], "call-old");
    assert_eq!(body["messages"][7]["role"], "tool");
}

#[tokio::test]
async fn chat_completions_fragmented_tool_call_keeps_turn_open() {
    let payload = concat!(
        "data: {\"id\":\"chat-tool\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"list_\",\"arguments\":\"{\\\"scope\\\":\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chat-tool\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"instances\",\"arguments\":\"\\\"all\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await;
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call }
        }
            if call.call_id == "call-1"
                && call.name == "list_instances"
                && call.arguments == json!({"scope": "all"})
    )));
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "chat-tool".into(),
            usage: None,
            end_turn: Some(false),
        })
    );
}

#[tokio::test]
async fn chat_completions_keeps_text_and_tool_items_from_the_same_choice() {
    let payload = concat!(
        "data: {\"id\":\"chat-mixed\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Checking\",\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"list_instances\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await;

    assert!(matches!(
        &events[1],
        ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id,
                phase: None,
                ..
            },
        } if item_id == "chat-mixed:message"
    ));
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
        } if call.call_id == "call-1"
    )));
}

/// Every role this encoder emits has to be one a compatible Chat Completions
/// server accepts. Those servers implement exactly `system`, `user`,
/// `assistant` and `tool`; anything else fails the whole request during body
/// deserialization, before the model is ever reached.
#[tokio::test]
async fn chat_completions_only_uses_roles_a_compatible_server_accepts() {
    const ACCEPTED_ROLES: [&str; 4] = ["system", "user", "assistant", "tool"];

    let payload = concat!(
        "data: {\"id\":\"chat-roles\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let mut request = fixture_request();
    request.input = vec![
        ModelInputItem::ContextFragment {
            role: ModelContextRole::Developer,
            kind: "skill_catalog".into(),
            content: "catalog metadata".into(),
        },
        ModelInputItem::ContextFragment {
            role: ModelContextRole::User,
            kind: "application_ui_context".into(),
            content: "ui context".into(),
        },
        ModelInputItem::UserMessage {
            content: vec![ModelUserContent::InputText {
                text: "hello".into(),
            }],
        },
        ModelInputItem::AssistantMessage {
            phase: None,
            text: "working".into(),
        },
        ModelInputItem::ToolCall {
            call_id: "call-1".into(),
            namespace: None,
            name: "list_instances".into(),
            arguments: json!({}),
        },
        ModelInputItem::ToolOutput {
            call_id: "call-1".into(),
            output: ToolOutput {
                content: vec![ToolOutputContent::Text { text: "[]".into() }],
                structured_content: None,
                is_error: false,
            },
        },
    ];
    let _ = collect_request(&transport, request).await;

    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let roles = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["role"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(roles.len() >= 6, "{roles:?}");
    for role in &roles {
        assert!(
            ACCEPTED_ROLES.contains(&role.as_str()),
            "role {role:?} is not accepted by compatible servers; saw {roles:?}"
        );
    }
    // The developer-role fragment travels as `system`, the same spelling this
    // encoder already uses for the request instructions.
    assert_eq!(roles[0], "system");
    assert_eq!(roles[1], "system");
}
