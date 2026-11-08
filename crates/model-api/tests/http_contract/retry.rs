use super::*;

#[tokio::test]
async fn request_retry_budget_does_not_replay_an_established_stream() {
    let server = FakeHttpServer::start(vec![
        PlannedResponse::status(503, br#"{"error":{"message":"retry"}}"#.to_vec()),
        PlannedResponse::sse(Vec::new()),
        PlannedResponse::sse(vec![ResponseFragment::immediate(streaming_payload("ok"))]),
    ])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

    assert!(matches!(
        collect(&transport).await,
        Err(ModelError::StreamInterrupted(_))
    ));
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn retryable_http_status_succeeds_with_the_exact_request_attempt_count() {
    let server = FakeHttpServer::start(vec![
        PlannedResponse::status(503, br#"{"error":{"message":"retry"}}"#.to_vec()),
        PlannedResponse::sse(vec![ResponseFragment::immediate(streaming_payload("ok"))]),
    ])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

    assert_eq!(collect(&transport).await.unwrap().len(), 5);
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn retryable_http_status_exhaustion_is_a_terminal_request_error() {
    let server = FakeHttpServer::start(vec![
        PlannedResponse::status(503, br#"{"error":{"message":"retry-1"}}"#.to_vec()),
        PlannedResponse::status(503, br#"{"error":{"message":"retry-2"}}"#.to_vec()),
        PlannedResponse::status(503, br#"{"error":{"message":"retry-3"}}"#.to_vec()),
    ])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

    assert!(matches!(
        transport
            .stream(fixture_request(), CancellationToken::default())
            .await,
        Err(ModelError::RequestRetriesExhausted {
            attempts: 3,
            status: Some(503),
            ..
        })
    ));
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn stream_failure_after_first_model_event_is_never_replayed() {
    let first_event = br#"data: {"type":"response.created","response":{"id":"resp-first"}}

"#;
    let server = FakeHttpServer::start(vec![
        PlannedResponse::sse(vec![ResponseFragment::immediate(first_event.to_vec())]),
        PlannedResponse::sse(vec![ResponseFragment::immediate(streaming_payload(
            "must-not-run",
        ))]),
    ])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();
    let mut stream = transport
        .stream(fixture_request(), CancellationToken::default())
        .await
        .unwrap();

    assert!(matches!(
        stream.next().await.unwrap().unwrap(),
        ModelEvent::ResponseStarted { ref response_id, .. } if response_id == "resp-first"
    ));
    assert!(matches!(
        stream.next().await.unwrap(),
        Err(ModelError::StreamInterrupted(_))
    ));
}
