use serde_json::{json, Value};

use super::AnthropicDecoder;
use crate::{
    ModelEvent, ModelOutputItem, ModelResponseMetadata, ModelToolCall, ModelUsage,
    MAX_TOOL_ARGUMENT_PREVIEW_BYTES,
};

fn decode_all(frames: &[Value]) -> Vec<ModelEvent> {
    let mut decoder = AnthropicDecoder::new();
    let metadata = ModelResponseMetadata::default();
    let mut events = Vec::new();
    for frame in frames {
        let frame = serde_json::to_vec(frame).unwrap();
        events.extend(decoder.decode(&frame, &metadata).unwrap());
    }
    events
}

fn message_start(usage: Value) -> Value {
    json!({"type": "message_start", "message": {"id": "msg", "usage": usage}})
}

fn tool_frames(initial_input: Option<Value>, partial_json: &[&str]) -> Vec<Value> {
    let mut block = json!({"type": "tool_use", "id": "toolu-1", "name": "write"});
    if let Some(input) = initial_input {
        block["input"] = input;
    }
    let mut frames = vec![
        message_start(json!({"input_tokens": 3, "output_tokens": 1})),
        json!({"type": "content_block_start", "index": 0, "content_block": block}),
    ];
    for partial in partial_json {
        frames.push(json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": partial}
        }));
    }
    frames.extend([
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ]);
    frames
}

fn completed_call(events: &[ModelEvent]) -> &ModelToolCall {
    events
        .iter()
        .find_map(|event| match event {
            ModelEvent::OutputItemCompleted {
                item: ModelOutputItem::ToolCall { call },
            } => Some(call),
            _ => None,
        })
        .expect("a completed tool call")
}

#[test]
fn unparseable_tool_input_completes_the_call_with_an_argument_error() {
    let cut = "好".repeat(300);
    let events = decode_all(&tool_frames(Some(json!({})), &["{\"text\":\"", &cut]));
    let call = completed_call(&events);
    assert_eq!(call.arguments, json!({}));
    let error = call.argument_error.as_ref().unwrap();
    assert!(error.preview.len() <= MAX_TOOL_ARGUMENT_PREVIEW_BYTES);
    assert!(format!("{{\"text\":\"{cut}").starts_with(&error.preview));
    assert!(matches!(
        events.last(),
        Some(ModelEvent::ResponseCompleted {
            end_turn: Some(false),
            ..
        })
    ));
}

#[test]
fn tool_input_that_is_not_an_object_completes_with_an_argument_error() {
    let events = decode_all(&tool_frames(None, &["[1,2]"]));
    let call = completed_call(&events);
    assert_eq!(call.arguments, json!({}));
    assert_eq!(call.argument_error.as_ref().unwrap().preview, "[1,2]");
}

#[test]
fn empty_partial_json_is_the_initial_input_or_an_empty_object() {
    for (initial, partial) in [(Some(json!({})), ""), (None, ""), (Some(json!(null)), "  ")] {
        let events = decode_all(&tool_frames(initial, &[partial]));
        let call = completed_call(&events);
        assert_eq!(call.arguments, json!({}));
        assert_eq!(call.argument_error, None);
    }
    let events = decode_all(&tool_frames(Some(json!({"path": "a"})), &[]));
    assert_eq!(completed_call(&events).arguments, json!({"path": "a"}));
}

#[test]
fn usage_with_a_missing_count_keeps_the_other() {
    let events = decode_all(&[
        message_start(json!({"input_tokens": 3})),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": null}}),
        json!({"type": "message_stop"}),
    ]);
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "msg".into(),
            usage: Some(ModelUsage {
                input_tokens: 3,
                output_tokens: 0,
            }),
            end_turn: Some(true),
        })
    );
}

#[test]
fn an_invalid_count_is_no_usage_and_a_missing_stop_reason_is_an_unknown_end_turn() {
    let events = decode_all(&[
        message_start(json!({"input_tokens": "3", "output_tokens": 1})),
        json!({"type": "message_delta", "delta": {}, "usage": {"output_tokens": 2}}),
        json!({"type": "message_stop"}),
    ]);
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "msg".into(),
            usage: None,
            end_turn: None,
        })
    );
}

#[test]
fn deltas_and_stops_for_an_unknown_block_are_ignored() {
    let events = decode_all(&[
        message_start(json!(null)),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "kept"}}),
        json!({"type": "content_block_delta", "index": 7, "delta": {"type": "text_delta", "text": "lost"}}),
        json!({"type": "content_block_stop", "index": 7}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"}}),
        json!({"type": "message_stop"}),
    ]);
    assert!(events.iter().any(|event| matches!(
        event,
        ModelEvent::OutputItemCompleted {
            item: ModelOutputItem::AssistantMessage { text, .. }
        } if text == "kept"
    )));
    assert_eq!(
        events.last(),
        Some(&ModelEvent::ResponseCompleted {
            response_id: "msg".into(),
            usage: None,
            end_turn: None,
        })
    );
}
