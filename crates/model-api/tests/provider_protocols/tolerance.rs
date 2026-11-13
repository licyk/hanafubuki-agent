//! What the Chat Completions and Anthropic Messages transports tolerate
//! from a provider, and what stays an error.

use super::*;

async fn stream_results(transport: &dyn ModelPort) -> Vec<Result<ModelEvent, ModelError>> {
    let mut stream = transport
        .stream(fixture_request(), CancellationToken::default())
        .await
        .unwrap();
    let mut results = Vec::new();
    while let Some(result) = stream.next().await {
        results.push(result);
    }
    results
}

async fn chat_results(plan: PlannedResponse) -> Vec<Result<ModelEvent, ModelError>> {
    let server = FakeHttpServer::start(vec![plan]).await;
    stream_results(&ChatCompletionsModelTransport::new(provider(&server)).unwrap()).await
}

fn sse(payload: &str) -> PlannedResponse {
    PlannedResponse::sse(vec![ResponseFragment::immediate(payload.as_bytes())])
}

#[tokio::test]
async fn chat_end_of_body_without_done_after_content_completes_the_response() {
    let results = chat_results(sse(concat!(
        "data: {\"id\":\"chat-eof\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}]}\n\n",
        "data: {\"id\":\"chat-eof\",\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\n"
    )))
    .await;

    let events = results.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { text, .. }
        } if text == "hello"
    )));
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "chat-eof".into(),
            usage: Some(ModelUsage {
                input_tokens: 3,
                output_tokens: 1,
                ..Default::default()
            }),
            end_turn: None,
        })
    );
}

#[tokio::test]
async fn chat_end_of_body_without_content_is_still_an_interruption() {
    for payload in [
        "",
        "data: {\"id\":\"chat-empty\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n",
    ] {
        let results = chat_results(sse(payload)).await;
        assert!(
            matches!(results.last(), Some(Err(ModelError::StreamInterrupted(_)))),
            "{payload:?}: {results:?}"
        );
    }
}

#[tokio::test]
async fn chat_end_of_body_inside_an_event_is_still_a_protocol_error() {
    let results = chat_results(sse(concat!(
        "data: {\"id\":\"chat-cut\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}]}\n\n",
        "data: {\"id\":\"chat-cut\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" wor"
    )))
    .await;

    assert!(matches!(results.last(), Some(Err(ModelError::Protocol(_)))));
}

#[tokio::test]
async fn anthropic_end_of_body_before_message_stop_is_still_an_interruption() {
    let server = FakeHttpServer::start(vec![sse(concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-eof\"}}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n"
    ))])
    .await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();

    let results = stream_results(&transport).await;

    assert!(matches!(
        results.last(),
        Some(Err(ModelError::StreamInterrupted(_)))
    ));
}

#[tokio::test]
async fn an_sse_body_under_another_content_type_is_decoded_and_json_is_rejected() {
    let payload = concat!(
        "data: {\"id\":\"chat-plain\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let results = chat_results(PlannedResponse {
        status: 200,
        header_delay: Duration::ZERO,
        headers: vec![("content-type".into(), "application/octet-stream".into())],
        fragments: vec![ResponseFragment::immediate(payload.as_bytes())],
    })
    .await;
    assert!(matches!(
        results.last(),
        Some(Ok(ModelEvent::ResponseCompleted { .. }))
    ));

    let results = chat_results(PlannedResponse::status(
        200,
        br#"{"id":"chat-json","choices":[]}"#.to_vec(),
    ))
    .await;
    assert!(matches!(
        results.as_slice(),
        [Err(ModelError::Protocol(message))] if message.contains("text/event-stream")
    ));
}

/// A call the provider sent unusable arguments for is kept in history with
/// `{}`; replaying it must be a valid request in every protocol.
#[tokio::test]
async fn a_replayed_call_with_empty_arguments_encodes_a_valid_request() {
    let chat = FakeHttpServer::start(vec![sse(
        "data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
    )])
    .await;
    let responses = FakeHttpServer::start(vec![sse(
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\"}}\n\n",
    )])
    .await;
    let anthropic = FakeHttpServer::start(vec![sse(concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"a\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    ))])
    .await;
    // `fixture_request` replays `call-old` with `{}` arguments.
    let _ = collect(&ChatCompletionsModelTransport::new(provider(&chat)).unwrap()).await;
    let _ = collect(&ResponsesModelTransport::new(provider(&responses)).unwrap()).await;
    let _ = collect(&AnthropicMessagesModelTransport::new(provider(&anthropic)).unwrap()).await;

    let body = |server: &FakeHttpServer| -> Value {
        serde_json::from_slice(&server.requests()[0].body).unwrap()
    };
    let chat_call = body(&chat)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|message| message.get("tool_calls"))
        .unwrap()[0]
        .clone();
    assert_eq!(chat_call["function"]["arguments"], "{}");
    let responses_call = body(&responses)["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call")
        .unwrap()
        .clone();
    assert_eq!(responses_call["arguments"], "{}");
    let anthropic_call = body(&anthropic)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .find(|block| block["type"] == "tool_use")
        .unwrap()
        .clone();
    assert_eq!(anthropic_call["input"], json!({}));
}
