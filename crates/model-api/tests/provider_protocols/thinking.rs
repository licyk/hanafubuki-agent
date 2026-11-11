//! Reasoning on Chat Completions and Anthropic Messages: the fields each
//! sends, Anthropic's thinking forms and `max_tokens`, and its thinking
//! blocks sent back through a tool loop.

use super::*;
use hanafubuki_agent_model_api::ModelReasoningReplay;

fn reasoning(effort: &str, max_output_tokens: Option<u64>) -> Option<ModelReasoningConfig> {
    Some(ModelReasoningConfig {
        effort: ModelReasoningEffort::parse(effort).unwrap(),
        max_output_tokens,
    })
}

/// A request that opens a turn: no tool result to continue.
fn opening_request(model: &str, effort: Option<&str>) -> ModelRequest {
    let mut request = fixture_request();
    request.model = model.into();
    request.input.truncate(5);
    request.reasoning = effort.and_then(|effort| reasoning(effort, None));
    request
}

async fn sent_body(
    transport: &dyn ModelPort,
    server: &FakeHttpServer,
    request: ModelRequest,
) -> Value {
    collect_request(transport, request).await;
    serde_json::from_slice(&server.requests().last().unwrap().body).unwrap()
}

fn chat_done() -> PlannedResponse {
    PlannedResponse::sse(vec![ResponseFragment::immediate(concat!(
        "data: {\"id\":\"c-1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    ))])
}

fn anthropic_done() -> PlannedResponse {
    PlannedResponse::sse(vec![ResponseFragment::immediate(concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-done\"}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Done\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    ))])
}

/// A response that thinks (a signed block and a redacted one) and calls a
/// tool.
fn anthropic_thinking_tool_call() -> PlannedResponse {
    PlannedResponse::sse(vec![ResponseFragment::immediate(concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-1\"}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"List them first.\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"one\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"redacted_thinking\",\"data\":\"opaque-data\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"list_instances\",\"input\":{}}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    ))])
}

/// The request after `events`, as the Agent builds it: the turn's
/// reasoning blocks, then its tool calls, then their results.
fn continuation(mut request: ModelRequest, events: &[ModelEvent]) -> ModelRequest {
    for event in events {
        let ModelEvent::OutputItemCompleted { item } = event else {
            continue;
        };
        match item {
            ModelOutputItem::Reasoning {
                replay: Some(replay),
                ..
            } => request.input.push(ModelInputItem::Reasoning {
                replay: replay.clone(),
            }),
            ModelOutputItem::ToolCall { call } => {
                request.input.push(ModelInputItem::ToolCall {
                    call_id: call.call_id.clone(),
                    namespace: None,
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
                request.input.push(ModelInputItem::ToolOutput {
                    call_id: call.call_id.clone(),
                    output: json!({"instances": []}).into(),
                });
            }
            _ => {}
        }
    }
    request
}

#[tokio::test]
async fn chat_completions_sends_reasoning_effort_only_when_configured() {
    let server = FakeHttpServer::start(vec![chat_done(), chat_done()]).await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();
    let with = sent_body(&transport, &server, opening_request("gpt-5", Some("xhigh"))).await;
    assert_eq!(with["reasoning_effort"], "xhigh");
    let without = sent_body(&transport, &server, opening_request("gpt-5", None)).await;
    assert!(without.get("reasoning_effort").is_none());
    assert!(without.get("max_completion_tokens").is_none());
}

#[tokio::test]
async fn anthropic_adaptive_models_take_the_effort_and_keep_no_sampling_parameters() {
    let server =
        FakeHttpServer::start(vec![anthropic_done(), anthropic_done(), anthropic_done()]).await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();
    let body = sent_body(
        &transport,
        &server,
        opening_request("claude-opus-5-5", Some("high")),
    )
    .await;
    assert_eq!(
        body["thinking"],
        json!({"type": "adaptive", "display": "summarized"})
    );
    assert_eq!(body["output_config"], json!({"effort": "high"}));
    assert_eq!(body["max_tokens"], 16_384, "no largest output known");
    for parameter in ["temperature", "top_p", "top_k"] {
        assert!(body.get(parameter).is_none(), "{parameter}");
    }

    // `minimal` is not one of Anthropic's levels; the largest output caps
    // `max_tokens`.
    let mut request = opening_request("claude-sonnet-4-6", None);
    request.reasoning = reasoning("minimal", Some(128_000));
    let body = sent_body(&transport, &server, request).await;
    assert_eq!(body["output_config"], json!({"effort": "low"}));
    assert_eq!(body["max_tokens"], 32_768);

    let body = sent_body(
        &transport,
        &server,
        opening_request("claude-opus-5-5", None),
    )
    .await;
    assert!(body.get("thinking").is_none() && body.get("output_config").is_none());
    assert_eq!(body["max_tokens"], 16_384);
}

#[tokio::test]
async fn anthropic_manual_models_take_a_budget_under_max_tokens() {
    let server =
        FakeHttpServer::start(vec![anthropic_done(), anthropic_done(), anthropic_done()]).await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();

    // Largest output unknown: `max_tokens` stays 16384 and the budget fits
    // under it with room for the answer.
    let body = sent_body(
        &transport,
        &server,
        opening_request("claude-sonnet-4-5-20250929", Some("high")),
    )
    .await;
    assert_eq!(
        body["thinking"],
        json!({"type": "enabled", "budget_tokens": 12_288})
    );
    assert_eq!(body["max_tokens"], 16_384);
    assert!(body.get("output_config").is_none());

    // Known: the answer's room is added, under the largest output.
    let mut request = opening_request("claude-opus-4-1", None);
    request.reasoning = reasoning("max", Some(32_000));
    let body = sent_body(&transport, &server, request).await;
    assert_eq!(
        body["thinking"],
        json!({"type": "enabled", "budget_tokens": 27_904})
    );
    assert_eq!(body["max_tokens"], 32_000);

    let mut request = opening_request("vendor-compatible-model", None);
    request.reasoning = reasoning("low", Some(64_000));
    let body = sent_body(&transport, &server, request).await;
    assert_eq!(
        body["thinking"],
        json!({"type": "enabled", "budget_tokens": 4_096})
    );
    assert_eq!(body["max_tokens"], 20_480);
}

#[tokio::test]
async fn anthropic_manual_thinking_is_left_out_where_it_would_be_refused() {
    let server = FakeHttpServer::start(vec![anthropic_done(), anthropic_done()]).await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();
    // A forced tool.
    let mut request = opening_request("claude-haiku-4-5", Some("medium"));
    request.tool_choice = ModelToolChoice::Required {
        name: "list_instances".into(),
    };
    let body = sent_body(&transport, &server, request).await;
    assert!(body.get("thinking").is_none());
    // A tool result continuing an assistant message with no thinking block
    // to send back (the fixture's call).
    let mut request = fixture_request();
    request.model = "claude-haiku-4-5".into();
    request.reasoning = reasoning("medium", None);
    let body = sent_body(&transport, &server, request).await;
    assert!(body.get("thinking").is_none());
    assert_eq!(body["max_tokens"], 16_384);
}

#[tokio::test]
async fn anthropic_manual_thinking_sends_the_turns_blocks_back_unchanged() {
    let server =
        FakeHttpServer::start(vec![anthropic_thinking_tool_call(), anthropic_done()]).await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();
    let first = opening_request("claude-sonnet-4-5", Some("low"));
    let events = collect_request(&transport, first.clone()).await;
    let blocks = events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::Reasoning { replay, .. },
            } => Some(replay.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        blocks,
        [
            Some(ModelReasoningReplay::Thinking {
                thinking: "List them first.".into(),
                signature: "sig-one".into(),
            }),
            Some(ModelReasoningReplay::RedactedThinking {
                data: "opaque-data".into(),
            }),
        ]
    );
    assert!(!serde_json::to_string(&events).unwrap().contains("sig-one"));

    collect_request(&transport, continuation(first, &events)).await;
    let second: Value = serde_json::from_slice(&server.requests()[1].body).unwrap();
    assert_eq!(second["thinking"]["type"], "enabled");
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(
        messages[messages.len() - 2],
        json!({"role": "assistant", "content": [
            {"type": "thinking", "thinking": "List them first.", "signature": "sig-one"},
            {"type": "redacted_thinking", "data": "opaque-data"},
            {"type": "tool_use", "id": "call-1", "name": "list_instances", "input": {}},
        ]})
    );
    assert_eq!(
        messages[messages.len() - 1]["content"][0]["type"],
        "tool_result"
    );
}

#[tokio::test]
async fn anthropic_adaptive_thinking_sends_no_block_back() {
    let server =
        FakeHttpServer::start(vec![anthropic_thinking_tool_call(), anthropic_done()]).await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();
    let first = opening_request("claude-opus-5-5", Some("high"));
    let events = collect_request(&transport, first.clone()).await;
    collect_request(&transport, continuation(first, &events)).await;
    let second: Value = serde_json::from_slice(&server.requests()[1].body).unwrap();
    assert_eq!(second["thinking"]["type"], "adaptive");
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(
        messages[messages.len() - 2],
        json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "call-1", "name": "list_instances", "input": {}},
        ]})
    );
}

#[test]
fn reasoning_blocks_never_show_in_debug_output() {
    let block = ModelReasoningReplay::Thinking {
        thinking: "private".into(),
        signature: "sig".into(),
    };
    let shown = format!("{block:?}");
    assert!(
        !shown.contains("private") && !shown.contains("sig\""),
        "{shown}"
    );
}
