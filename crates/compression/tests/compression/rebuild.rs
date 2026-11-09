// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the `rebuildCompressionState` cases of `hide-merge-rebuild.test.ts`
// and `parse-compress-input.test.ts`. The `compress` call takes no ref here
// (it is protected), so the message after it is `m00003`.

use hanafubuki_agent_compression::{rebuild_state, Message, RebuildResult, State, BLOCKED_REF};
use serde_json::json;

use crate::fixtures::{assistant, call, config, user};

fn rebuild(messages: &[Message]) -> RebuildResult {
    rebuild_state(State::default(), messages, &config(), 0)
}

fn history(arguments: &str) -> Vec<Message> {
    let mut messages = vec![
        user("raw1", "first message"),
        assistant("raw2", "second message"),
    ];
    if !arguments.is_empty() {
        messages.push(call("raw3", "call1", "compress", arguments));
    }
    messages
}

fn summaries(result: &RebuildResult) -> Vec<&str> {
    result
        .state
        .blocks
        .iter()
        .map(|block| block.summary.as_str())
        .collect()
}

#[test]
fn historical_compress_calls_are_replayed() {
    let arguments = json!({"content": [{
        "startId": "m00001",
        "endId": "m00002",
        "summary": "rebuilt summary of early talk",
        "topic": "intro",
    }]})
    .to_string();
    let mut messages = history(&arguments);
    messages.push(user("raw4", "later message"));
    let result = rebuild(&messages);
    assert_eq!(result.blocks_rebuilt, 1);
    let block = &result.state.blocks[0];
    assert_eq!(block.summary, "rebuilt summary of early talk");
    assert_eq!(block.compress_call_id.as_deref(), Some("call1"));
    assert_eq!(block.effective_message_ids, ["raw1", "raw2"]);
    let refs = &result.state.message_refs.by_raw;
    assert_eq!(refs["raw1"], "m00001");
    assert_eq!(refs["raw3"], BLOCKED_REF);
    assert_eq!(refs["raw4"], "m00003");
}

#[test]
fn no_compress_call_rebuilds_nothing() {
    let result = rebuild(&[user("raw1", "plain")]);
    assert_eq!(result.blocks_rebuilt, 0);
    assert!(result.state.blocks.is_empty());
}

#[test]
fn a_strict_json_object_rebuilds() {
    let arguments =
        json!({"content": [{"startId": "m00001", "endId": "m00002", "summary": "strict object"}]});
    let result = rebuild(&history(&arguments.to_string()));
    assert_eq!(result.blocks_rebuilt, 1);
    assert_eq!(summaries(&result), ["strict object"]);
    assert_eq!(
        result.state.blocks[0].compress_call_id.as_deref(),
        Some("call1")
    );
}

#[test]
fn a_stringified_content_array_rebuilds() {
    let content = json!([{"startId": "m00001", "endId": "m00002", "summary": "vllm stringified"}]);
    let arguments = json!({"content": content.to_string()});
    let result = rebuild(&history(&arguments.to_string()));
    assert_eq!(result.blocks_rebuilt, 1);
    assert_eq!(summaries(&result), ["vllm stringified"]);
}

#[test]
fn a_truncated_call_rebuilds_its_complete_entries() {
    let document = json!({"content": [
        {"startId": "m00001", "endId": "m00002", "summary": "survives truncation"},
        {"startId": "m00003", "endId": "m00004", "summary": "cut off here"},
    ]})
    .to_string();
    let result = rebuild(&history(&document[..document.len() - 12]));
    assert_eq!(result.blocks_rebuilt, 1);
    assert_eq!(summaries(&result), ["survives truncation"]);
}

#[test]
fn garbage_arguments_rebuild_nothing() {
    let result = rebuild(&history("total garbage not json"));
    assert_eq!(result.blocks_rebuilt, 0);
    assert!(result.state.blocks.is_empty());
}

#[test]
fn a_call_with_no_valid_range_is_skipped() {
    let result = rebuild(&history(&json!({"content": []}).to_string()));
    assert_eq!(result.blocks_rebuilt, 0);
    assert!(result.state.blocks.is_empty());
}

#[test]
fn a_single_object_content_rebuilds() {
    let arguments =
        json!({"content": {"startId": "m00001", "endId": "m00002", "summary": "single object"}});
    let result = rebuild(&history(&arguments.to_string()));
    assert_eq!(result.blocks_rebuilt, 1);
    assert_eq!(summaries(&result), ["single object"]);
}

#[test]
fn a_later_call_over_an_earlier_block_rebuilds_the_tier() {
    let first =
        json!({"content": [{"startId": "m00001", "endId": "m00002", "summary": "first fold"}]});
    let second = json!({"content": [{"startId": "b1", "endId": "b1", "summary": "distilled"}]});
    let messages = [
        user("raw1", "first message"),
        assistant("raw2", "second message"),
        call("raw3", "call1", "compress", &first.to_string()),
        call("raw4", "call2", "compress", &second.to_string()),
    ];
    let result = rebuild(&messages);
    assert_eq!(result.blocks_rebuilt, 2);
    let distilled = result.state.block("b2").unwrap();
    assert_eq!(distilled.tier, 2);
    assert_eq!(distilled.direct_block_ids, ["b1"]);
    assert!(!result.state.block("b1").unwrap().active);
}

#[test]
fn a_call_its_receipt_says_applied_nothing_is_not_replayed() {
    let arguments =
        json!({"content": [{"startId": "m00001", "endId": "m00002", "summary": "refused"}]});
    let mut messages = history(&arguments.to_string());
    messages.push(crate::fixtures::result(
        "raw4",
        "call1",
        "compress",
        "[Compression refused: the range is too small]",
    ));
    let result = rebuild(&messages);
    assert_eq!(result.blocks_rebuilt, 0);
    assert!(result.state.blocks.is_empty());
}
