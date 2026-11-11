use super::*;

use hanafubuki_agent_model_api::ModelReasoningTextKind;

/// The decoded events as `(name, item id, text)`, enough to read the order of
/// reasoning and answer items and the reasoning text each carried.
fn outline(events: &[ModelEvent]) -> Vec<(&'static str, String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::OutputItemAdded {
                item: ModelOutputItem::Reasoning { item_id, .. },
            } => Some(("reasoning_added", item_id.clone(), String::new())),
            ModelEvent::ReasoningDelta {
                item_id,
                kind: ModelReasoningTextKind::Raw,
                index: 0,
                delta,
            } => Some(("reasoning_delta", item_id.clone(), delta.clone())),
            ModelEvent::OutputItemCompleted {
                item:
                    ModelOutputItem::Reasoning {
                        item_id,
                        summary,
                        content,
                        ..
                    },
            } => {
                assert!(summary.is_empty());
                Some(("reasoning_done", item_id.clone(), content.concat()))
            }
            ModelEvent::OutputItemAdded {
                item: ModelOutputItem::AssistantMessage { item_id, .. },
            } => Some(("message_added", item_id.clone(), String::new())),
            ModelEvent::OutputItemAdded {
                item: ModelOutputItem::ToolCall { call },
            } => Some(("tool_added", call.item_id.clone(), String::new())),
            _ => None,
        })
        .collect()
}

fn owned(rows: &[(&'static str, &str, &str)]) -> Vec<(&'static str, String, String)> {
    rows.iter()
        .map(|(name, item_id, text)| (*name, (*item_id).to_string(), (*text).to_string()))
        .collect()
}

#[tokio::test]
async fn chat_reasoning_content_is_a_reasoning_item_that_ends_when_the_answer_starts() {
    let payload = concat!(
        "data: {\"id\":\"chat-r\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"Look at \"}}]}\n\n",
        "data: {\"id\":\"chat-r\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"the routes.\",\"reasoning\":\"the routes.\"}}]}\n\n",
        "data: {\"id\":\"chat-r\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Done\"}}]}\n\n",
        "data: {\"id\":\"chat-r\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning\":\"One more thought.\"}}]}\n\n",
        "data: {\"id\":\"chat-r\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();

    let events = collect(&transport).await;

    assert_eq!(
        outline(&events),
        owned(&[
            ("reasoning_added", "chat-r:reasoning:0", ""),
            ("reasoning_delta", "chat-r:reasoning:0", "Look at "),
            ("reasoning_delta", "chat-r:reasoning:0", "the routes."),
            (
                "reasoning_done",
                "chat-r:reasoning:0",
                "Look at the routes."
            ),
            ("message_added", "chat-r:message", ""),
            ("reasoning_added", "chat-r:reasoning:1", ""),
            ("reasoning_delta", "chat-r:reasoning:1", "One more thought."),
            ("reasoning_done", "chat-r:reasoning:1", "One more thought."),
        ]),
        "a delta carrying both fields is read once, and reasoning after the answer is a new item",
    );
    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted { .. })
    ));
}

#[tokio::test]
async fn chat_reasoning_before_a_tool_call_completes_before_the_call_starts() {
    let payload = concat!(
        "data: {\"id\":\"chat-t\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning\":\"Need the list.\"}}]}\n\n",
        "data: {\"id\":\"chat-t\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"list_instances\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = ChatCompletionsModelTransport::new(provider(&server)).unwrap();

    let events = collect(&transport).await;

    assert_eq!(
        outline(&events),
        owned(&[
            ("reasoning_added", "chat-t:reasoning:0", ""),
            ("reasoning_delta", "chat-t:reasoning:0", "Need the list."),
            ("reasoning_done", "chat-t:reasoning:0", "Need the list."),
            ("tool_added", "chat-t:tool:0", ""),
        ])
    );
}

#[tokio::test]
async fn anthropic_thinking_blocks_are_reasoning_items_whose_signatures_are_never_serialized() {
    let payload = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg-think\",\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Weigh \"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"the options.\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"opaque-signature\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Done\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
    );
    let server = FakeHttpServer::start(vec![PlannedResponse::sse(vec![
        ResponseFragment::immediate(payload.as_bytes()),
    ])])
    .await;
    let transport = AnthropicMessagesModelTransport::new(provider(&server)).unwrap();

    let events = collect(&transport).await;

    assert_eq!(
        outline(&events),
        owned(&[
            ("reasoning_added", "msg-think:thinking:0", ""),
            ("reasoning_delta", "msg-think:thinking:0", "Weigh "),
            ("reasoning_delta", "msg-think:thinking:0", "the options."),
            (
                "reasoning_done",
                "msg-think:thinking:0",
                "Weigh the options."
            ),
            ("message_added", "msg-think:text:1", ""),
        ])
    );
    // The signed block is kept, to be sent back, but never serialized.
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::Reasoning { replay: Some(replay), .. }
        } if *replay == hanafubuki_agent_model_api::ModelReasoningReplay::Thinking {
            thinking: "Weigh the options.".into(),
            signature: "opaque-signature".into(),
        }
    )));
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("opaque-signature"));
    let body: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    assert!(
        body.get("thinking").is_none(),
        "thinking is requested only with an effort"
    );
}

/// Every log record that carries reasoning text is at `trace`; the typed
/// events carry it at any level.
#[tokio::test]
async fn reasoning_text_reaches_no_log_record_below_trace() {
    const SECRET: &str = "secret-reasoning-text";
    capture::install();
    let chat = format!(
        concat!(
            "data: {{\"id\":\"chat-log\",\"choices\":[{{\"index\":0,\"delta\":{{\"reasoning_content\":\"{secret}\"}}}}]}}\n\n",
            "data: {{\"id\":\"chat-log\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"ok\"}},\"finish_reason\":\"stop\"}}]}}\n\n",
            "data: [DONE]\n\n"
        ),
        secret = SECRET
    );
    let anthropic = format!(
        concat!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg-log\"}}}}\n\n",
            "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"thinking\",\"thinking\":\"\"}}}}\n\n",
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"thinking_delta\",\"thinking\":\"{secret}\"}}}}\n\n",
            "event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
            "event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
        ),
        secret = SECRET
    );
    let responses = format!(
        concat!(
            "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp-log\"}}}}\n\n",
            "data: {{\"type\":\"response.output_item.added\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-log\",\"summary\":[]}}}}\n\n",
            "data: {{\"type\":\"response.reasoning_summary_text.delta\",\"item_id\":\"reason-log\",\"summary_index\":0,\"delta\":\"{secret}\"}}\n\n",
            "data: {{\"type\":\"response.reasoning_text.delta\",\"item_id\":\"reason-log\",\"content_index\":0,\"delta\":\"{secret}\"}}\n\n",
            "data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"reasoning\",\"id\":\"reason-log\",\"summary\":[{{\"type\":\"summary_text\",\"text\":\"{secret}\"}}]}}}}\n\n",
            "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp-log\"}}}}\n\n"
        ),
        secret = SECRET
    );
    let server = FakeHttpServer::start(
        [chat, anthropic, responses]
            .into_iter()
            .map(|payload| PlannedResponse::sse(vec![ResponseFragment::immediate(payload)]))
            .collect(),
    )
    .await;

    let mut events = collect(&ChatCompletionsModelTransport::new(provider(&server)).unwrap()).await;
    events.extend(collect(&AnthropicMessagesModelTransport::new(provider(&server)).unwrap()).await);
    events.extend(collect(&ResponsesModelTransport::new(provider(&server)).unwrap()).await);

    let carrying = |event: &&ModelEvent| matches!(event, ModelEvent::ReasoningDelta { delta, .. } if delta == SECRET);
    assert_eq!(events.iter().filter(carrying).count(), 4);
    let records = capture::records_containing(SECRET);
    assert!(
        !records.is_empty(),
        "trace is the troubleshooting window into the text"
    );
    assert!(
        records.iter().all(|level| *level == log::Level::Trace),
        "{records:?}"
    );
}

mod capture {
    use std::sync::{Mutex, OnceLock};

    struct Capture(Mutex<Vec<(log::Level, String)>>);

    static CAPTURE: OnceLock<Capture> = OnceLock::new();

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            self.0
                .lock()
                .unwrap()
                .push((record.level(), record.args().to_string()));
        }

        fn flush(&self) {}
    }

    pub(super) fn install() {
        let capture = CAPTURE.get_or_init(|| Capture(Mutex::new(Vec::new())));
        let _ = log::set_logger(capture);
        log::set_max_level(log::LevelFilter::Trace);
    }

    pub(super) fn records_containing(text: &str) -> Vec<log::Level> {
        CAPTURE
            .get()
            .unwrap()
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, message)| message.contains(text))
            .map(|(level, _)| *level)
            .collect()
    }
}
