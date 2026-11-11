use super::*;

#[tokio::test]
async fn fake_server_completes_fragmented_stream_and_records_exact_request() {
    let payload = streaming_payload("你好");
    let split = payload
        .chunks(3)
        .map(|chunk| ResponseFragment::delayed(Duration::from_millis(1), chunk.to_vec()))
        .collect();
    let mut response = PlannedResponse::sse(split);
    response.headers.extend([
        ("x-request-id".into(), "request-1".into()),
        ("x-ratelimit-limit-requests".into(), "100".into()),
        ("x-ratelimit-remaining-requests".into(), "99".into()),
    ]);
    let server = FakeHttpServer::start(vec![response]).await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

    assert_eq!(
        collect(&transport).await.unwrap(),
        vec![
            ModelEvent::ResponseStarted {
                response_id: "resp-1".into(),
                metadata: ModelResponseMetadata {
                    request_id: Some("request-1".into()),
                    rate_limit: RateLimitMetadata {
                        limit_requests: Some("100".into()),
                        remaining_requests: Some("99".into()),
                        ..RateLimitMetadata::default()
                    },
                },
            },
            ModelEvent::OutputItemAdded {
                item: ModelOutputItem::AssistantMessage {
                    item_id: "msg-1".into(),
                    phase: Some(ModelMessagePhase::FinalAnswer),
                    text: String::new(),
                },
            },
            ModelEvent::AssistantTextDelta {
                item_id: "msg-1".into(),
                delta: "你好".into(),
            },
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::AssistantMessage {
                    item_id: "msg-1".into(),
                    phase: Some(ModelMessagePhase::FinalAnswer),
                    text: "你好".into(),
                },
            },
            ModelEvent::ResponseCompleted {
                response_id: "resp-1".into(),
                usage: Some(ModelUsage {
                    input_tokens: 7,
                    output_tokens: 3,
                }),
                end_turn: Some(true),
            },
        ]
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/responses");
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer credential-secret"
    );
    assert_eq!(
        requests[0].headers.get("x-workspace-key").unwrap(),
        "header-secret"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
        json!({
            "model": "fixture-model",
            "instructions": "Use only typed tools.",
            "input": [
                {"role": "developer", "content": [{"type": "input_text", "text": "catalog metadata"}]},
                {"role": "user", "content": [{"type": "input_text", "text": "{\"schemaVersion\":1,\"page\":{\"kind\":\"instance_chooser\"}}"}]},
                {"role": "user", "content": [{"type": "input_text", "text": "untrusted workflow"}]},
                {"role": "user", "content": [{"type": "input_text", "text": "hello"}]},
                {"role": "assistant", "phase": "commentary", "content": [{"type": "output_text", "text": "checking"}]},
                {"type": "function_call", "call_id": "call-1", "name": "list_instances", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "call-1", "output": "{\"instances\":[]}"}
            ],
            "tools": [{
                "type": "function",
                "name": "list_instances",
                "description": "List instances",
                "parameters": {"type": "object", "properties": {}, "required": [], "additionalProperties": false},
                "strict": true
            }],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stream": true,
            "store": false
        })
    );
}

#[tokio::test]
async fn responses_preserves_multiple_assistant_items_and_explicit_or_missing_phase() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-many\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"comment\",\"phase\":\"commentary\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"comment\",\"delta\":\"Checking\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"comment\",\"phase\":\"commentary\",\"content\":[{\"type\":\"output_text\",\"text\":\"Checking\"}]}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"answer\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"answer\",\"delta\":\"Done\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-many\",\"end_turn\":true}}\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await.unwrap();

    let started = events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::OutputItemAdded {
                item: ModelOutputItem::AssistantMessage { item_id, phase, .. },
            } => Some((item_id.as_str(), *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        started,
        vec![
            ("comment", Some(ModelMessagePhase::Commentary),),
            ("answer", None,),
        ]
    );
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "resp-many".into(),
            usage: None,
            end_turn: Some(true),
        })
    );
}

#[tokio::test]
async fn responses_streams_summary_and_raw_reasoning_deltas_with_their_part_index() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-summary\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"reason-1\",\"summary\":[]}}\n\n",
        "data: {\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"reason-1\",\"output_index\":0,\"summary_index\":0,\"part\":{\"type\":\"summary_text\",\"text\":\"\"}}\n\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"reason-1\",\"output_index\":0,\"summary_index\":0,\"delta\":\"**Mapping\"}\n\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"reason-1\",\"output_index\":0,\"summary_index\":0,\"delta\":\" the app**\"}\n\n",
        "data: {\"type\":\"response.reasoning_summary_text.done\",\"item_id\":\"reason-1\",\"output_index\":0,\"summary_index\":0,\"text\":\"**Mapping the app**\"}\n\n",
        "data: {\"type\":\"response.reasoning_summary_part.done\",\"item_id\":\"reason-1\",\"output_index\":0,\"summary_index\":0,\"part\":{\"type\":\"summary_text\",\"text\":\"**Mapping the app**\"}}\n\n",
        "data: {\"type\":\"response.reasoning_summary_part.added\",\"item_id\":\"reason-1\",\"output_index\":0,\"summary_index\":1,\"part\":{\"type\":\"summary_text\",\"text\":\"\"}}\n\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"reason-1\",\"output_index\":0,\"summary_index\":1,\"delta\":\"Checking routes\"}\n\n",
        "data: {\"type\":\"response.reasoning_text.delta\",\"item_id\":\"reason-1\",\"output_index\":0,\"content_index\":0,\"delta\":\"raw reasoning\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"id\":\"reason-1\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"**Mapping the app**\"},{\"type\":\"summary_text\",\"text\":\"Checking routes\"}],\"content\":[{\"type\":\"reasoning_text\",\"text\":\"raw reasoning\"}]}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"answer\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"answer\",\"delta\":\"Done\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-summary\",\"end_turn\":true}}\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await.unwrap();

    let deltas = events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::ReasoningDelta {
                item_id,
                kind,
                index,
                delta,
            } => Some((item_id.as_str(), *kind, *index, delta.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        deltas,
        vec![
            ("reason-1", ModelReasoningTextKind::Summary, 0, "**Mapping"),
            ("reason-1", ModelReasoningTextKind::Summary, 0, " the app**"),
            (
                "reason-1",
                ModelReasoningTextKind::Summary,
                1,
                "Checking routes"
            ),
            ("reason-1", ModelReasoningTextKind::Raw, 0, "raw reasoning"),
        ]
    );
    assert!(matches!(
        &events[1],
        ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning { item_id, .. },
        } if item_id == "reason-1"
    ));
    assert_eq!(
        events[6],
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning {
                item_id: "reason-1".into(),
                summary: vec!["**Mapping the app**".into(), "Checking routes".into()],
                content: vec!["raw reasoning".into()],
                replay: None,
            },
        }
    );
}

#[tokio::test]
async fn responses_reads_a_reasoning_summary_delta_without_a_part_index_as_the_first_part() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-summary\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"reason-1\",\"summary\":[]}}\n\n",
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"reason-1\",\"output_index\":0,\"delta\":\"Mapping\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-summary\"}}\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

    let events = collect(&transport).await.unwrap();

    assert_eq!(
        events[2],
        ModelEvent::ReasoningDelta {
            item_id: "reason-1".into(),
            kind: ModelReasoningTextKind::Summary,
            index: 0,
            delta: "Mapping".into(),
        }
    );
}

#[tokio::test]
async fn responses_preserves_reasoning_and_unknown_items_without_breaking_the_stream() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-extra\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"reasoning\",\"id\":\"reason-1\",\"summary\":[]}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"reasoning\",\"id\":\"reason-1\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"bounded summary\"}]}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"computer_call\",\"id\":\"other-1\"}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"computer_call\",\"id\":\"other-1\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"answer\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"answer\",\"delta\":\"Done\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"answer\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-extra\",\"end_turn\":true}}\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await.unwrap();

    assert!(matches!(
        &events[1],
        ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Reasoning { item_id, .. },
        } if item_id == "reason-1"
    ));
    assert!(matches!(
        &events[2],
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning { summary, .. },
        } if summary == &["bounded summary"]
    ));
    assert!(matches!(
        &events[3],
        ModelEvent::OutputItemAdded {
            item: ModelOutputItem::Other {
                item_id: Some(item_id),
                item_type,
            },
        } if item_id == "other-1" && item_type == "computer_call"
    ));
    assert!(matches!(
        &events[4],
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Other { .. },
        }
    ));
    assert!(matches!(
        &events[7],
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { text, .. },
        } if text == "Done"
    ));
}

#[tokio::test]
async fn responses_keeps_commentary_and_tool_items_in_provider_order() {
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(tool_call_payload()),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let events = collect(&transport).await.unwrap();

    assert!(matches!(
        &events[1],
        ModelEvent::OutputItemAdded {
            item: ModelOutputItem::AssistantMessage {
                item_id,
                phase: Some(ModelMessagePhase::Commentary),
                ..
            },
        } if item_id == "comment"
    ));
    assert!(matches!(
        &events[3],
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage {
                item_id,
                phase: Some(ModelMessagePhase::Commentary),
                text,
            },
        } if item_id == "comment" && text == "Checking"
    ));
    assert!(matches!(
        &events[4],
        ModelEvent::OutputItemAdded {
            item: ModelOutputItem::ToolCall { call },
        } if call.item_id == "tool-item"
    ));
    assert!(matches!(
        &events[5],
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::ToolCall { call }
        }
            if call.item_id == "tool-item"
                && call.call_id == "call-tool"
                && call.name == "list_instances"
                && call.arguments == json!({})
    ));
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "resp-tool".into(),
            usage: None,
            end_turn: Some(false),
        })
    );
}

#[tokio::test]
async fn provider_usage_keeps_given_counts_and_never_fails_the_response() {
    for (usage, expected) in [
        (
            r#"{"input_tokens":7}"#,
            Some(ModelUsage {
                input_tokens: 7,
                output_tokens: 0,
            }),
        ),
        (
            r#"{"input_tokens":7,"output_tokens":null}"#,
            Some(ModelUsage {
                input_tokens: 7,
                output_tokens: 0,
            }),
        ),
        // A count that is present but invalid is not trusted as zero: the
        // response reports no usage instead.
        (r#"{"input_tokens":-1,"output_tokens":3}"#, None),
        (r#"{"input_tokens":"7","output_tokens":3}"#, None),
        (r#"[]"#, None),
    ] {
        let payload = format!(
            concat!(
                "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-usage\"}}}}\n\n",
                "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-usage\",\"usage\":{}}}}}\n\n"
            ),
            usage
        );
        let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
            ResponseFragment::immediate(payload.into_bytes()),
        ])])
        .await;
        let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

        assert_eq!(
            collect(&transport).await.unwrap().last(),
            Some(&ModelEvent::ResponseCompleted {
                response_id: "resp-usage".into(),
                usage: expected,
                end_turn: None,
            }),
            "{usage}"
        );
    }
}
