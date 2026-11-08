use super::*;

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
