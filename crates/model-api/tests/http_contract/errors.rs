use super::*;

#[tokio::test]
async fn untyped_sse_error_after_response_start_preserves_remote_failure_and_redacts_secrets() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-1\"}}\n\n",
        "data: {\"error\":{\"type\":\"upstream_error\",\"code\":\"credential-secret\",\"message\":\"insufficient tool messages following tool_calls message header-secret\"}}\n\n",
        "data: [DONE]\n\n"
    );
    let mut response = PlannedResponse::sse(vec![ResponseFragment::immediate(
        payload.as_bytes().to_vec(),
    )]);
    response
        .headers
        .push(("x-request-id".into(), "req-1".into()));
    let server = FakeHttpServer::start(vec![response]).await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let mut stream = transport
        .stream(fixture_request(), CancellationToken::default())
        .await
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(Ok(ModelEvent::ResponseStarted { .. }))
    ));
    let error = stream.next().await.unwrap().unwrap_err();
    assert!(matches!(
        &error,
        ModelError::Remote { code: Some(code), message, request_id: Some(id), .. }
            if code == "[redacted]"
                && message == "insufficient tool messages following tool_calls message [redacted]"
                && id == "req-1"
    ));
    assert!(!error.to_string().contains("credential-secret"));
    assert!(!error.to_string().contains("header-secret"));
    assert!(stream.next().await.is_none());
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn untyped_sse_context_error_uses_the_existing_error_classification() {
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(
            b"data: {\"error\":{\"code\":\"context_length_exceeded\",\"message\":\"context window exceeded\"}}\n\n".to_vec(),
        ),
    ])])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    assert!(matches!(
        collect(&transport).await,
        Err(ModelError::ContextWindowExceeded { .. })
    ));
}

#[tokio::test]
async fn untyped_sse_data_without_an_error_object_is_still_rejected() {
    for payload in [r#"{"message":"unknown"}"#, r#"{"error":null}"#] {
        let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
            ResponseFragment::immediate(format!("data: {payload}\n\n").into_bytes()),
        ])])
        .await;
        let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
        assert!(matches!(
            collect(&transport).await,
            Err(ModelError::Protocol(_))
        ));
    }
}

#[tokio::test]
async fn credentials_headers_and_echoed_remote_errors_are_redacted() {
    let mut response = PlannedResponse::status(
        400,
        br#"{"error":{"code":"credential-secret","message":"credential-secret header-secret"}}"#
            .to_vec(),
    );
    response
        .headers
        .push(("x-request-id".into(), "header-secret".into()));
    let server = FakeHttpServer::start(vec![response]).await;
    let provider = provider(&server);
    let debug = format!("{provider:?}");
    assert!(!debug.contains("credential-secret"));
    assert!(!debug.contains("header-secret"));

    let transport = ResponsesModelTransport::new(provider).unwrap();
    let error = match transport
        .stream(fixture_request(), CancellationToken::default())
        .await
    {
        Ok(_) => panic!("remote error unexpectedly produced a stream"),
        Err(error) => error,
    };
    let diagnostic = error.to_string();
    assert!(!diagnostic.contains("credential-secret"));
    assert!(!diagnostic.contains("header-secret"));
    assert!(diagnostic.contains("[redacted]"));
    assert!(matches!(
        error,
        ModelError::Remote {
            status: Some(400),
            code: Some(ref code),
            request_id: Some(ref request_id),
            ..
        } if code == "[redacted]" && request_id == "[redacted]"
    ));
    assert_eq!(server.requests().len(), 1);
}
