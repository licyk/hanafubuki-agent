use serde_json::{json, Value};

use super::ChatDecoder;
use crate::{
    ModelError, ModelEvent, ModelOutputItem, ModelResponseMetadata, ModelToolCall, ModelUsage,
    MAX_TOOL_ARGUMENT_PREVIEW_BYTES,
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
