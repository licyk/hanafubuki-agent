use super::*;

#[tokio::test]
async fn model_catalog_supports_openai_and_paginated_anthropic_shapes() {
    let openai = FakeHttpServer::start(vec![PlannedResponse::status(
        200,
        br#"{"object":"list","data":[{"id":"z-model","default_reasoning_level":"medium","supported_reasoning_levels":[{"effort":"low","description":"Fast"},{"effort":"future-tier","description":"Future"}],"context_length":131072},{"id":"a-model","context_length":"large"}]}"#.to_vec(),
    )])
    .await;
    let catalog = ModelCatalogClient::new(provider(&openai)).unwrap();
    let entries = catalog
        .list(
            ModelApiProtocol::ChatCompletions,
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-model", "z-model"]
    );
    assert_eq!(openai.requests()[0].path, "/v1/models");
    assert_eq!(entries[0].context_window, None);
    assert_eq!(entries[1].context_window, Some(131_072));
    assert_eq!(entries[0].default_reasoning_effort, None);
    assert!(entries[0].supported_reasoning_efforts.is_empty());
    assert_eq!(
        entries[1]
            .default_reasoning_effort
            .as_ref()
            .unwrap()
            .as_str(),
        "medium"
    );
    assert_eq!(
        entries[1]
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "future-tier"]
    );
    let serialized = serde_json::to_value(&entries[1]).unwrap();
    assert_eq!(serialized["defaultReasoningEffort"], "medium");
    assert_eq!(
        serialized["supportedReasoningEfforts"],
        json!(["low", "future-tier"])
    );
    assert_eq!(
        openai.requests()[0].headers.get("authorization").unwrap(),
        "Bearer provider-secret"
    );
    assert_eq!(
        openai.requests()[0]
            .headers
            .get("x-provider-scope")
            .unwrap(),
        "workspace-a"
    );

    let anthropic = FakeHttpServer::start(vec![
        PlannedResponse::status(
            200,
            br#"{"data":[{"id":"claude-b","display_name":"Claude B","max_input_tokens":1000000}],"has_more":true,"last_id":"claude-b"}"#.to_vec(),
        ),
        PlannedResponse::status(
            200,
            br#"{"data":[{"id":"claude-a","display_name":"Claude A"}],"has_more":false,"last_id":"claude-a"}"#.to_vec(),
        ),
    ])
    .await;
    let catalog = ModelCatalogClient::new(provider(&anthropic)).unwrap();
    let entries = catalog
        .list(
            ModelApiProtocol::AnthropicMessages,
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(entries[0].display_name.as_deref(), Some("Claude A"));
    assert_eq!(entries[0].context_window, None);
    assert_eq!(entries[1].context_window, Some(1_000_000));
    assert_eq!(entries[0].default_reasoning_effort, None);
    assert!(entries[0].supported_reasoning_efforts.is_empty());
    let requests = anthropic.requests();
    assert_eq!(requests[0].path, "/v1/models?limit=100");
    assert_eq!(requests[1].path, "/v1/models?limit=100&after_id=claude-b");
    assert_eq!(
        requests[0].headers.get("x-api-key").unwrap(),
        "provider-secret"
    );
    assert_eq!(
        requests[0].headers.get("x-provider-scope").unwrap(),
        "workspace-a"
    );
}

#[tokio::test]
async fn streaming_provider_errors_preserve_credentials() {
    let payload =
        "data: {\"error\":{\"code\":\"bad\",\"message\":\"provider-secret rejected\"}}\n\n";
    let server =
        FakeHttpServer::start(vec![PlannedResponse::sse(vec![ResponseFragment::delayed(
            Duration::from_millis(1),
            payload.as_bytes(),
        )])])
        .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let mut stream = transport
        .stream(fixture_request(), CancellationToken::default())
        .await
        .unwrap();
    let error = stream.next().await.unwrap().unwrap_err().to_string();
    assert!(error.contains("provider-secret"));
}

#[tokio::test]
async fn every_protocol_classifies_explicit_image_rejection_before_streaming() {
    let body =
        br#"{"error":{"code":"unsupported_image","message":"model only supports text input"}}"#
            .to_vec();
    let responses = FakeHttpServer::start(vec![PlannedResponse::status(400, body.clone())]).await;
    let chat = FakeHttpServer::start(vec![PlannedResponse::status(415, body.clone())]).await;
    let anthropic = FakeHttpServer::start(vec![PlannedResponse::status(422, body)]).await;

    for transport in [
        Box::new(ResponsesModelTransport::new(provider(&responses)).unwrap()) as Box<dyn ModelPort>,
        Box::new(ChatCompletionsModelTransport::new(provider(&chat)).unwrap()),
        Box::new(AnthropicMessagesModelTransport::new(provider(&anthropic)).unwrap()),
    ] {
        assert!(matches!(
            transport
                .stream(fixture_request(), CancellationToken::default())
                .await,
            Err(ModelError::UnsupportedImageInput { .. })
        ));
    }
}

#[tokio::test]
async fn every_protocol_classifies_context_window_rejection_before_streaming() {
    let body = br#"{"error":{"code":"context_length_exceeded","message":"maximum context length exceeded"}}"#
        .to_vec();
    let responses = FakeHttpServer::start(vec![PlannedResponse::status(400, body.clone())]).await;
    let chat = FakeHttpServer::start(vec![PlannedResponse::status(400, body.clone())]).await;
    let anthropic = FakeHttpServer::start(vec![PlannedResponse::status(400, body)]).await;

    for transport in [
        Box::new(ResponsesModelTransport::new(provider(&responses)).unwrap()) as Box<dyn ModelPort>,
        Box::new(ChatCompletionsModelTransport::new(provider(&chat)).unwrap()),
        Box::new(AnthropicMessagesModelTransport::new(provider(&anthropic)).unwrap()),
    ] {
        assert!(matches!(
            transport
                .stream(fixture_request(), CancellationToken::default())
                .await,
            Err(ModelError::ContextWindowExceeded { .. })
        ));
    }
}

#[tokio::test]
async fn chat_stream_error_before_response_is_classified_without_transport_replay() {
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(
            b"data: {\"error\":{\"code\":\"unsupported_image\",\"message\":\"vision input is not supported\"}}\n\n",
        ),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let mut stream = transport
        .stream(fixture_request(), CancellationToken::default())
        .await
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(Err(ModelError::UnsupportedImageInput { .. }))
    ));
    assert_eq!(server.requests().len(), 1);
}
