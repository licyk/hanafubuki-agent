use super::*;

#[tokio::test]
async fn responses_compact_uses_exact_endpoint_body_headers_and_typed_replacement() {
    let server = FakeHttpServer::start(vec![PlannedResponse::status(
        200,
        br#"{"output":[{"type":"message","role":"developer","content":[{"type":"input_text","text":"duplicate"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]},{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"world"}]},{"type":"compaction","encrypted_content":"opaque-checkpoint"}]}"#,
    )])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server))
        .unwrap()
        .with_compaction_capability(ModelCompactionCapability::ResponsesCompact);
    let mut request = compact_request();
    request.reasoning = Some(ModelReasoningConfig {
        effort: ModelReasoningEffort::Low,
    });
    let result = transport
        .compact(request, CancellationToken::default())
        .await
        .unwrap();
    assert_eq!(result.replacement_history.len(), 4);
    assert!(matches!(
        result.replacement_history.last(),
        Some(ModelInputItem::Compaction { payload }) if payload.as_str() == "opaque-checkpoint"
    ));

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/v1/responses/compact");
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer credential-secret")
    );
    assert_eq!(
        requests[0]
            .headers
            .get("x-workspace-key")
            .map(String::as_str),
        Some("header-secret")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
        json!({
            "model": "fixture-model",
            "instructions": "Keep the exact base instructions.",
            "input": [
                {"role":"user","content":[{"type":"input_text","text":"hello"}]},
                {"role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"world"}]}
            ],
            "tools": [{
                "type":"function",
                "name":"list_instances",
                "description":"List instances",
                "parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false},
                "strict":true
            }],
            "parallel_tool_calls": true,
            "reasoning": {"effort": "low", "summary": "auto"}
        })
    );
}

#[tokio::test]
async fn compact_capability_is_explicit_and_empty_input_never_probes_the_endpoint() {
    let server = FakeHttpServer::start(Vec::new()).await;
    let disabled = ResponsesModelTransport::new(provider(&server)).unwrap();
    assert!(matches!(
        disabled
            .compact(compact_request(), CancellationToken::default())
            .await,
        Err(ModelError::InvalidRequest(_))
    ));
    let mut empty = compact_request();
    empty.input.clear();
    let enabled = disabled.with_compaction_capability(ModelCompactionCapability::ResponsesCompact);
    assert!(enabled
        .compact(empty, CancellationToken::default())
        .await
        .unwrap()
        .replacement_history
        .is_empty());
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn responses_compact_retries_to_exhaustion_and_rejects_malformed_output() {
    let retry = || PlannedResponse::status(503, br#"{"error":{"message":"busy"}}"#);
    let server = FakeHttpServer::start(vec![retry(), retry(), retry()]).await;
    let transport = ResponsesModelTransport::new(provider(&server))
        .unwrap()
        .with_compaction_capability(ModelCompactionCapability::ResponsesCompact);
    assert!(matches!(
        transport
            .compact(compact_request(), CancellationToken::default())
            .await,
        Err(ModelError::RequestRetriesExhausted { attempts: 3, .. })
    ));
    assert_eq!(server.requests().len(), 3);

    let malformed = FakeHttpServer::start(vec![PlannedResponse::status(200, b"not-json")]).await;
    let transport = ResponsesModelTransport::new(provider(&malformed))
        .unwrap()
        .with_compaction_capability(ModelCompactionCapability::ResponsesCompact);
    assert!(matches!(
        transport
            .compact(compact_request(), CancellationToken::default())
            .await,
        Err(ModelError::Protocol(_))
    ));
}

#[tokio::test]
async fn responses_compact_full_unary_timeout_and_cancellation_are_bounded() {
    let timeout_server = FakeHttpServer::start(vec![PlannedResponse {
        status: 200,
        header_delay: Duration::ZERO,
        headers: vec![("content-type".into(), "application/json".into())],
        fragments: vec![ResponseFragment::delayed(
            Duration::from_secs(1),
            br#"{"output":[]}"#,
        )],
    }])
    .await;
    let transport = ResponsesModelTransport::new(provider(&timeout_server))
        .unwrap()
        .with_compaction_capability(ModelCompactionCapability::ResponsesCompact);
    assert!(matches!(
        transport
            .compact(compact_request(), CancellationToken::default())
            .await,
        Err(ModelError::RequestRetriesExhausted { .. })
    ));

    let cancel_server = FakeHttpServer::start(vec![PlannedResponse {
        status: 200,
        header_delay: Duration::from_secs(1),
        headers: vec![("content-type".into(), "application/json".into())],
        fragments: vec![],
    }])
    .await;
    let transport = ResponsesModelTransport::new(provider(&cancel_server))
        .unwrap()
        .with_compaction_capability(ModelCompactionCapability::ResponsesCompact);
    let cancellation = CancellationToken::default();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        cancel.cancel();
    });
    assert_eq!(
        transport.compact(compact_request(), cancellation).await,
        Err(ModelError::Cancelled)
    );
}

#[test]
fn opaque_compaction_payload_round_trips_without_debug_disclosure() {
    let payload = ModelOpaqueCompactionPayload::parse("secret-opaque-payload".into()).unwrap();
    let encoded = serde_json::to_string(&payload).unwrap();
    let decoded: ModelOpaqueCompactionPayload = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.as_str(), "secret-opaque-payload");
    assert!(!format!("{decoded:?}").contains("secret-opaque-payload"));
    assert!(ModelOpaqueCompactionPayload::parse(String::new()).is_err());
    assert!(ModelOpaqueCompactionPayload::parse("x".repeat(1024 * 1024 + 1)).is_err());
}

#[tokio::test]
async fn responses_v2_decodes_one_fragmented_compaction_item_and_replays_it_opaquely() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"v2\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"compaction\",\"id\":\"compact-1\"}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"compaction\",\"id\":\"compact-1\",\"encrypted_content\":\"opaque-v2\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"v2\",\"usage\":{\"input_tokens\":11,\"output_tokens\":2},\"end_turn\":true}}\n\n",
        "data: [DONE]\n\n"
    );
    let fragments = payload
        .as_bytes()
        .chunks(2)
        .map(|chunk| ResponseFragment::immediate(chunk.to_vec()))
        .collect();
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(fragments)]).await;
    let transport = ResponsesModelTransport::new(provider(&server))
        .unwrap()
        .with_compaction_capability(ModelCompactionCapability::ResponsesCompactV2);
    let mut request = compact_request();
    request.input.push(ModelInputItem::CompactionTrigger);
    let mut stream = transport
        .stream(request, CancellationToken::default())
        .await
        .unwrap();
    let mut completed = Vec::new();
    while let Some(event) = stream.next().await {
        if let ModelEvent::OutputItemCompleted { item } = event.unwrap() {
            completed.push(item);
        }
    }
    assert!(matches!(
        completed.as_slice(),
        [ModelOutputItem::Compaction { item_id, payload }]
            if item_id == "compact-1" && payload.as_str() == "opaque-v2"
    ));
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert_eq!(
        body["input"].as_array().unwrap().last(),
        Some(&json!({"type":"compaction_trigger"}))
    );
}
