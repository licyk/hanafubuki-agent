use super::*;

#[tokio::test]
async fn idle_timeout_and_cancellation_interrupt_pending_async_io() {
    let idle_server =
        FakeHttpServer::start(vec![PlannedResponse::sse(vec![ResponseFragment::delayed(
            Duration::from_millis(150),
            streaming_payload("late"),
        )])])
        .await;
    let idle_provider = provider(&idle_server).with_timeouts(
        ModelTimeouts::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_millis(20),
        )
        .unwrap(),
    );
    let idle_transport = ResponsesModelTransport::new(idle_provider).unwrap();
    let mut idle_stream = idle_transport
        .stream(fixture_request(), CancellationToken::default())
        .await
        .unwrap();
    assert!(matches!(
        idle_stream.next().await.unwrap(),
        Err(ModelError::IdleTimeout { .. })
    ));

    let cancel_server =
        FakeHttpServer::start(vec![PlannedResponse::sse(vec![ResponseFragment::delayed(
            Duration::from_millis(150),
            streaming_payload("late"),
        )])])
        .await;
    let cancel_transport = ResponsesModelTransport::new(provider(&cancel_server)).unwrap();
    let cancellation = CancellationToken::default();
    let mut stream = cancel_transport
        .stream(fixture_request(), cancellation.clone())
        .await
        .unwrap();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel();
    });
    assert_eq!(stream.next().await.unwrap(), Err(ModelError::Cancelled));
    assert!(stream.next().await.is_none());

    let establishment_server = FakeHttpServer::start(vec![PlannedResponse {
        status: 200,
        header_delay: Duration::from_millis(150),
        headers: vec![("content-type".into(), "text/event-stream".into())],
        fragments: vec![ResponseFragment::immediate(streaming_payload("late"))],
    }])
    .await;
    let establishment_transport =
        ResponsesModelTransport::new(provider(&establishment_server)).unwrap();
    let cancellation = CancellationToken::default();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel();
    });
    assert!(matches!(
        establishment_transport
            .stream(fixture_request(), cancellation)
            .await,
        Err(ModelError::Cancelled)
    ));
    assert_eq!(establishment_server.requests().len(), 1);
}
