use super::*;
use hanafubuki_agent_model_api::{GeminiModelTransport, ModelApiProtocol};

const PROTOCOLS: [ModelApiProtocol; 2] = [
    ModelApiProtocol::GeminiGenerateContent,
    ModelApiProtocol::GeminiInteractions,
];

#[tokio::test]
async fn google_errors_are_classified_and_diagnostics_redact_the_key() {
    for protocol in PROTOCOLS {
        let error = |status, code, message| {
            PlannedResponse::status(
                status,
                json!({"error":{
                    "status":code,"code":status,"message":message
                }})
                .to_string(),
            )
        };
        let server = FakeHttpServer::start(vec![
            error(403, "PERMISSION_DENIED", "Invalid key: provider-secret"),
            error(400, "INVALID_ARGUMENT", "The input token count exceeds the maximum number of tokens allowed (65536)."),
            error(429, "RESOURCE_EXHAUSTED", "Quota exhausted"),
            PlannedResponse::sse(vec![ResponseFragment::immediate(
                b"data: {\"error\":{\"code\":400,\"status\":\"INVALID_ARGUMENT\",\"message\":\"responseMimeType is not supported\"}}\n\n"
            )]),
        ]).await;
        let endpoint =
            ModelEndpoint::parse(&format!("{}/edge?tenant=unit", server.base_url())).unwrap();
        let transport = GeminiModelTransport::new(
            ModelProvider::new(
                endpoint,
                Some(ModelCredential::new("provider-secret").unwrap()),
            )
            .with_retry_policy(hanafubuki_agent_model_api::ModelRetryPolicy {
                maximum_attempts: 1,
                ..Default::default()
            }),
            protocol,
        )
        .unwrap();
        for status in [403, 400, 429] {
            let Err(error) = transport
                .stream(fixture_request(), CancellationToken::default())
                .await
            else {
                panic!("expected HTTP {status}")
            };
            assert!(!error.to_string().contains("provider-secret"));
            assert!(
                match status {
                    403 =>
                        matches!(error, ModelError::Remote {status:Some(403), ref code,..} if code.as_deref()==Some("PERMISSION_DENIED")),
                    400 => matches!(error, ModelError::ContextWindowExceeded { .. }),
                    _ => matches!(
                        error,
                        ModelError::RequestRetriesExhausted {
                            status: Some(429),
                            ..
                        }
                    ),
                },
                "{error}"
            );
        }
        let mut stream = transport
            .stream(fixture_request(), CancellationToken::default())
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await,
            Some(Err(ModelError::UnsupportedStructuredOutput { .. }))
        ));
        assert!(stream.next().await.is_none());
        let requests = server.requests();
        assert_eq!(requests.len(), 4);
        let expected_path = if protocol == ModelApiProtocol::GeminiGenerateContent {
            "/v1/edge/models/fixture-model:streamGenerateContent?tenant=unit&alt=sse"
        } else {
            "/v1/edge/interactions?tenant=unit"
        };
        for request in requests {
            assert_eq!(request.path, expected_path);
            assert_eq!(request.headers["x-goog-api-key"], "provider-secret");
            assert!(!request.headers.contains_key("authorization"));
        }
    }
}

#[tokio::test]
async fn a_partial_stream_ends_on_cancellation_or_eof_without_a_retry() {
    for protocol in PROTOCOLS {
        let first = if protocol == ModelApiProtocol::GeminiGenerateContent {
            json!({"candidates":[{"content":{"parts":[{"text":"partial"}]}}]})
        } else {
            json!({"event_type":"interaction.created","interaction":{"id":"i","status":"in_progress"}})
        };
        for cancel in [false, true] {
            let data = format!("data: {first}\n\n");
            let mut fragments = vec![
                ResponseFragment::immediate(&data.as_bytes()[..3]),
                ResponseFragment::immediate(&data.as_bytes()[3..]),
            ];
            if cancel {
                fragments.push(ResponseFragment::delayed(
                    Duration::from_secs(60),
                    b": still open\n\n",
                ));
            }
            let server = FakeHttpServer::start(vec![PlannedResponse::sse(fragments)]).await;
            let transport = GeminiModelTransport::new(provider(&server), protocol).unwrap();
            let token = CancellationToken::default();
            let mut stream = transport
                .stream(fixture_request(), token.clone())
                .await
                .unwrap();
            assert!(matches!(
                stream.next().await,
                Some(Ok(ModelEvent::ResponseStarted { .. }))
            ));
            if cancel {
                token.cancel();
            }
            let error = loop {
                match stream
                    .next()
                    .await
                    .expect("unfinished stream requires a diagnostic")
                {
                    Ok(ModelEvent::ResponseCompleted { .. }) => panic!("partial stream completed"),
                    Ok(_) => continue,
                    Err(err) => break err,
                }
            };
            if cancel {
                assert_eq!(error, ModelError::Cancelled);
            } else {
                assert!(matches!(error, ModelError::StreamInterrupted(_)), "{error}");
            }
            assert!(stream.next().await.is_none());
            assert_eq!(server.requests().len(), 1);
        }
    }
}
