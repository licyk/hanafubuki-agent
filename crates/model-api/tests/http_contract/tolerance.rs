//! What the Responses transport tolerates from a provider: one unusable
//! item never fails the rest of the response.

use super::*;

async fn responses_events(payload: &str) -> Result<Vec<ModelEvent>, ModelError> {
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes().to_vec()),
    ])])
    .await;
    collect(&ResponsesModelTransport::new(provider(&server)).unwrap()).await
}

fn completed_calls(events: &[ModelEvent]) -> Vec<&hanafubuki_agent_model_api::ModelToolCall> {
    events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::ToolCall { call },
            } => Some(call),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn unparseable_function_arguments_complete_the_call_with_an_argument_error() {
    let raw = format!("{{\\\"text\\\":\\\"{}", "好".repeat(300));
    let payload = format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-args\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"id\":\"bad\",\"call_id\":\"call-bad\",\"name\":\"write\",\"arguments\":\"{raw}\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"function_call\",\"id\":\"bare\",\"call_id\":\"call-bare\",\"name\":\"list_instances\"}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-args\",\"end_turn\":false}}}}\n\n"
        ),
        raw = raw
    );

    let events = responses_events(&payload).await.unwrap();

    let calls = completed_calls(&events);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].call_id, "call-bad");
    assert_eq!(calls[0].arguments, json!({}));
    let error = calls[0].argument_error.as_ref().unwrap();
    assert!(error.preview.len() <= hanafubuki_agent_model_api::MAX_TOOL_ARGUMENT_PREVIEW_BYTES);
    assert!(error.preview.starts_with("{\"text\":\"好"));
    assert!(error.preview.ends_with('好'));
    assert_eq!(calls[1].call_id, "call-bare");
    assert_eq!(calls[1].arguments, json!({}));
    assert_eq!(calls[1].argument_error, None);
    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted { .. })
    ));
}

#[tokio::test]
async fn an_incomplete_response_keeps_its_completed_items_and_ends_the_stream() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-cut\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"message\",\"id\":\"msg-1\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg-1\",\"delta\":\"partial\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg-1\",\"content\":[{\"type\":\"output_text\",\"text\":\"partial\"}]}}\n\n",
        "data: {\"type\":\"response.incomplete\",\"response\":{\"id\":\"resp-cut\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"},\"usage\":{\"input_tokens\":5,\"output_tokens\":9}}}\n\n"
    );

    let events = responses_events(payload).await.unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { text, .. }
        } if text == "partial"
    )));
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseIncomplete {
            response_id: "resp-cut".into(),
            usage: Some(ModelUsage {
                input_tokens: 5,
                output_tokens: 9,
                ..Default::default()
            }),
            reason: "max_output_tokens".into(),
        })
    );
}

#[tokio::test]
async fn an_incomplete_response_without_details_has_the_default_reason() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-cut\"}}\n\n",
        "data: {\"type\":\"response.incomplete\",\"response\":{\"id\":\"resp-cut\"}}\n\n"
    );

    let events = responses_events(payload).await.unwrap();

    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseIncomplete {
            response_id: "resp-cut".into(),
            usage: None,
            reason: "incomplete".into(),
        })
    );
}

#[tokio::test]
async fn a_failed_response_is_still_an_error() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-failed\"}}\n\n",
        "data: {\"type\":\"response.failed\",\"response\":{\"id\":\"resp-failed\",\"error\":{\"code\":\"server_error\",\"message\":\"failed\"}}}\n\n"
    );

    assert!(responses_events(payload).await.is_err());
}

#[tokio::test]
async fn a_completed_item_without_an_added_event_is_emitted() {
    let payload = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-done\"}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"msg-1\",\"content\":[{\"type\":\"output_text\",\"text\":\"whole\"}]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-done\",\"end_turn\":true}}\n\n"
    );

    let events = responses_events(payload).await.unwrap();

    assert!(matches!(
        &events[1],
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { item_id, text, .. }
        } if item_id == "msg-1" && text == "whole"
    ));
}

#[tokio::test]
async fn an_sse_body_under_another_content_type_is_decoded() {
    let server = FakeHttpServer::start(vec![PlannedResponse {
        status: 200,
        header_delay: Duration::ZERO,
        headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
        fragments: vec![ResponseFragment::immediate(streaming_payload("ok"))],
    }])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

    let events = collect(&transport).await.unwrap();

    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted { .. })
    ));
}

#[tokio::test]
async fn a_body_that_is_clearly_not_sse_is_still_a_protocol_error() {
    let server = FakeHttpServer::start(vec![PlannedResponse::status(
        200,
        br#"{"id":"resp-json","output":[]}"#.to_vec(),
    )])
    .await;
    let transport = ResponsesModelTransport::new(provider(&server)).unwrap();

    assert!(matches!(
        collect(&transport).await,
        Err(ModelError::Protocol(message)) if message.contains("text/event-stream")
    ));
}
