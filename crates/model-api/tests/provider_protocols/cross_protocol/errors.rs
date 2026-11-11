use super::*;

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
