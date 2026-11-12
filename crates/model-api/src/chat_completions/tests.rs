use serde_json::{json, Value};

use super::ChatDecoder;
use crate::{
    ModelError, ModelEvent, ModelInputItem, ModelOutputItem, ModelReasoningReplay, ModelRequest,
    ModelResponseMetadata, ModelToolCall, ModelToolChoice, ModelUsage,
    MAX_MODEL_REASONING_REPLAY_BYTES, MAX_TOOL_ARGUMENT_PREVIEW_BYTES,
};

fn decode_all(frames: &[Value]) -> (ChatDecoder, Vec<ModelEvent>) {
    let mut decoder = ChatDecoder::new();
    let metadata = ModelResponseMetadata::default();
    let mut events = Vec::new();
    for frame in frames {
        let frame = serde_json::to_vec(frame).unwrap();
        events.extend(decoder.decode(&frame, &metadata).unwrap());
    }
    (decoder, events)
}

fn done(decoder: &mut ChatDecoder) -> Vec<ModelEvent> {
    decoder
        .decode(b"[DONE]", &ModelResponseMetadata::default())
        .unwrap()
}

fn completed_calls(events: &[ModelEvent]) -> Vec<&ModelToolCall> {
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

fn tool_chunk(tool: Value) -> Value {
    json!({"id": "chat", "choices": [{"index": 0, "delta": {"tool_calls": [tool]}}]})
}

#[test]
fn unparseable_tool_arguments_complete_the_call_with_an_argument_error() {
    let raw = format!("{{\"text\":\"{}", "好".repeat(300));
    let (mut decoder, _) = decode_all(&[tool_chunk(json!({
        "index": 0,
        "id": "call-1",
        "function": {"name": "write", "arguments": raw}
    }))]);
    let events = done(&mut decoder);
    let call = completed_calls(&events)[0];
    assert_eq!(call.call_id, "call-1");
    assert_eq!(call.arguments, json!({}));
    let error = call.argument_error.as_ref().unwrap();
    assert!(!error.reason.is_empty());
    assert!(error.preview.len() <= MAX_TOOL_ARGUMENT_PREVIEW_BYTES);
    assert!(raw.starts_with(&error.preview));
    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted { .. })
    ));
}

#[test]
fn empty_tool_arguments_are_an_empty_object_without_an_error() {
    let (mut decoder, _) = decode_all(&[tool_chunk(json!({
        "index": 0,
        "id": "call-1",
        "function": {"name": "list_instances", "arguments": ""}
    }))]);
    let events = done(&mut decoder);
    let call = completed_calls(&events)[0];
    assert_eq!(call.arguments, json!({}));
    assert_eq!(call.argument_error, None);
}

#[test]
fn a_usage_only_chunk_without_choices_records_usage() {
    let (mut decoder, events) = decode_all(&[
        json!({"id": "chat", "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": "stop"}]}),
        json!({"id": "chat", "usage": {"prompt_tokens": 4, "completion_tokens": 1}}),
    ]);
    assert_eq!(events.len(), 3);
    assert_eq!(
        done(&mut decoder).last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "chat".into(),
            usage: Some(ModelUsage {
                input_tokens: 4,
                output_tokens: 1,
            }),
            end_turn: Some(true),
        })
    );
}

#[test]
fn usage_with_a_missing_count_keeps_the_other_and_unusable_usage_is_ignored() {
    let (mut decoder, _) = decode_all(&[
        json!({"id": "chat", "choices": [{"index": 0, "delta": {"content": "hi"}}]}),
        json!({"id": "chat", "choices": [], "usage": {"prompt_tokens": 4}}),
        json!({"id": "chat", "choices": [], "usage": "not an object"}),
    ]);
    assert!(matches!(
        done(&mut decoder).last(),
        Some(ModelEvent::ResponseCompleted {
            usage: Some(ModelUsage {
                input_tokens: 4,
                output_tokens: 0,
            }),
            ..
        })
    ));
}

#[test]
fn fragments_without_an_index_continue_the_latest_call() {
    let (mut decoder, _) = decode_all(&[
        tool_chunk(json!({"id": "call-1", "function": {"name": "list_", "arguments": "{\"a\""}})),
        tool_chunk(json!({"function": {"name": "instances", "arguments": ":1"}})),
        tool_chunk(json!({"id": "call-1", "function": {"arguments": "}"}})),
    ]);
    let events = done(&mut decoder);
    let calls = completed_calls(&events);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].call_id, "call-1");
    assert_eq!(calls[0].name, "list_instances");
    assert_eq!(calls[0].arguments, json!({"a": 1}));
}

#[test]
fn a_delta_without_an_index_and_with_a_new_id_starts_a_new_call() {
    let (mut decoder, added) = decode_all(&[
        tool_chunk(json!({"id": "call-1", "function": {"name": "one", "arguments": "{}"}})),
        tool_chunk(json!({"id": "call-2", "function": {"name": "two", "arguments": "{\"b\":2}"}})),
    ]);
    assert_eq!(
        added
            .iter()
            .filter(|event| matches!(event, ModelEvent::OutputItemAdded { .. }))
            .count(),
        2
    );
    let events = done(&mut decoder);
    let calls = completed_calls(&events);
    assert_eq!(
        calls
            .iter()
            .map(|call| (call.call_id.as_str(), call.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("call-1", "one"), ("call-2", "two")]
    );
    assert_eq!(calls[1].arguments, json!({"b": 2}));
}

#[test]
fn a_missing_finish_reason_completes_with_an_unknown_end_turn() {
    let (mut decoder, _) = decode_all(&[json!({
        "id": "chat",
        "choices": [{"index": 0, "delta": {"content": "hi"}}]
    })]);
    assert_eq!(
        done(&mut decoder).last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "chat".into(),
            usage: None,
            end_turn: None,
        })
    );
}

#[test]
fn a_clean_end_after_content_completes_and_one_without_content_is_interrupted() {
    let (mut decoder, _) = decode_all(&[tool_chunk(json!({
        "index": 0,
        "id": "call-1",
        "function": {"name": "list_instances", "arguments": "{}"}
    }))]);
    let events = decoder.finish_at_eof().unwrap();
    assert_eq!(completed_calls(&events).len(), 1);
    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted { .. })
    ));

    let (mut decoder, _) = decode_all(&[json!({
        "id": "chat",
        "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": "stop"}]
    })]);
    assert!(matches!(
        decoder.finish_at_eof(),
        Err(ModelError::StreamInterrupted(_))
    ));
}

fn completed_replays(events: &[ModelEvent]) -> Vec<Option<ModelReasoningReplay>> {
    events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::Reasoning { replay, .. },
            } => Some(replay.clone()),
            _ => None,
        })
        .collect()
}

fn reasoning_chunk(field: &str, text: &str) -> Value {
    json!({"id": "chat", "choices": [{"index": 0, "delta": {field: text}}]})
}

#[test]
fn reasoning_content_is_kept_to_be_sent_back_and_reasoning_is_not() {
    let (mut decoder, mut events) = decode_all(&[
        reasoning_chunk("reasoning_content", "Look at "),
        reasoning_chunk("reasoning_content", "the routes."),
        json!({"id": "chat", "choices": [{"index": 0, "delta": {"content": "Done"}}]}),
        reasoning_chunk("reasoning", "One more thought."),
        tool_chunk(
            json!({"index": 0, "id": "call-1", "function": {"name": "a", "arguments": "{}"}}),
        ),
        reasoning_chunk("reasoning_content", "Mixed, "),
        reasoning_chunk("reasoning", "then renamed."),
    ]);
    events.extend(done(&mut decoder));
    assert_eq!(
        completed_replays(&events),
        [
            Some(ModelReasoningReplay::ReasoningContent {
                text: "Look at the routes.".into()
            }),
            None,
            None,
        ]
    );
}

#[test]
fn reasoning_content_over_the_bound_is_shown_but_not_sent_back() {
    let text = "x".repeat(MAX_MODEL_REASONING_REPLAY_BYTES + 1);
    let (mut decoder, mut events) = decode_all(&[
        reasoning_chunk("reasoning_content", &text),
        json!({"id": "chat", "choices": [{"index": 0, "delta": {"content": "Done"}}]}),
    ]);
    events.extend(done(&mut decoder));
    assert_eq!(completed_replays(&events), [None]);
}

fn reasoning_content(text: &str) -> ModelInputItem {
    ModelInputItem::Reasoning {
        replay: ModelReasoningReplay::ReasoningContent { text: text.into() },
    }
}

fn tool_call(call_id: &str) -> ModelInputItem {
    ModelInputItem::ToolCall {
        call_id: call_id.into(),
        namespace: None,
        name: "list_instances".into(),
        arguments: json!({}),
    }
}

fn tool_output(call_id: &str) -> ModelInputItem {
    ModelInputItem::ToolOutput {
        call_id: call_id.into(),
        output: json!({"instances": []}).into(),
    }
}

fn assistant(text: &str) -> ModelInputItem {
    ModelInputItem::AssistantMessage {
        phase: None,
        text: text.into(),
    }
}

fn encoded_messages(input: Vec<ModelInputItem>) -> Vec<Value> {
    let request = ModelRequest {
        model: "deepseek-v4-pro".into(),
        instructions: String::new(),
        input,
        tools: Vec::new(),
        tool_choice: ModelToolChoice::Auto,
        parallel_tool_calls: false,
        reasoning: None,
        output_format: None,
    };
    let body: Value =
        serde_json::from_slice(&super::encode_chat_request(&request).unwrap()).unwrap();
    body["messages"].as_array().unwrap().clone()
}

#[test]
fn reasoning_content_goes_back_on_the_assistant_message_it_preceded() {
    let messages = encoded_messages(vec![
        ModelInputItem::user_text("List them."),
        reasoning_content("First, "),
        reasoning_content("the list."),
        assistant("Listing."),
        tool_call("call-1"),
        tool_output("call-1"),
        reasoning_content("Again."),
        ModelInputItem::Reasoning {
            replay: ModelReasoningReplay::Thinking {
                thinking: "Anthropic's".into(),
                signature: "sig".into(),
            },
        },
        tool_call("call-2"),
        tool_output("call-2"),
        reasoning_content("Answer now."),
        assistant("Done."),
        reasoning_content("Cut off."),
        ModelInputItem::user_text("Next."),
        assistant("No reasoning."),
    ]);
    let assistants = messages
        .iter()
        .filter(|message| message["role"] == "assistant")
        .collect::<Vec<_>>();
    assert_eq!(assistants.len(), 4);
    assert_eq!(assistants[0]["content"], "Listing.");
    assert_eq!(assistants[0]["reasoning_content"], "First, the list.");
    assert_eq!(assistants[0]["tool_calls"][0]["id"], "call-1");
    assert_eq!(assistants[1]["content"], Value::Null);
    assert_eq!(assistants[1]["reasoning_content"], "Again.");
    assert_eq!(assistants[1]["tool_calls"][0]["id"], "call-2");
    assert_eq!(
        *assistants[2],
        json!({"role": "assistant", "content": "Done.", "reasoning_content": "Answer now."})
    );
    assert_eq!(
        *assistants[3],
        json!({"role": "assistant", "content": "No reasoning."})
    );
    assert!(!serde_json::to_string(&messages)
        .unwrap()
        .contains("Cut off."));
}

#[test]
fn reasoning_between_the_text_and_the_calls_stays_in_one_message() {
    let messages = encoded_messages(vec![
        ModelInputItem::user_text("List them."),
        assistant("Listing."),
        reasoning_content("Which tool?"),
        tool_call("call-1"),
        tool_output("call-1"),
    ]);
    assert_eq!(
        messages[1],
        json!({
            "role": "assistant",
            "content": "Listing.",
            "reasoning_content": "Which tool?",
            "tool_calls": [{
                "id": "call-1",
                "type": "function",
                "function": {"name": "list_instances", "arguments": "{}"}
            }]
        })
    );
    assert_eq!(messages.len(), 3);
}
