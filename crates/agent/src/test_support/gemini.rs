//! Scripted native Gemini event streams for tests of the facade and CLI.

use serde_json::{json, Value};

pub fn gemini_content_events(part: Value) -> Vec<u8> {
    let chunk = json!({
        "responseId": "response",
        "candidates": [{
            "index": 0, "content": {"role": "model", "parts": [part]}, "finishReason": "STOP"
        }]
    });
    format!("data: {chunk}\n\n").into_bytes()
}

pub fn gemini_interaction_events(step: Value, delta: Value, status: &str) -> Vec<u8> {
    let frames = [
        json!({"event_type":"interaction.created","interaction":{"id":"response","status":"in_progress"}}),
        json!({"event_type":"step.start","index":0,"step":step}),
        json!({"event_type":"step.delta","index":0,"delta":delta}),
        json!({"event_type":"step.stop","index":0}),
        json!({"event_type":"interaction.completed","interaction":{"id":"response","status":status}}),
    ];
    frames
        .iter()
        .map(|frame| {
            let kind = frame["event_type"].as_str().unwrap();
            format!("event: {kind}\ndata: {frame}\n\n")
        })
        .collect::<String>()
        .into_bytes()
}
