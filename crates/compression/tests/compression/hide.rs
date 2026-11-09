// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports orphan-fixes (hideConsumedCompressCalls), orphan-endpoint-snap (the
// hide case), issue3769-orphan-sweep, hide-consumed-stub and hide-merge-rebuild
// (the hide cases).

use serde_json::{json, Value};

use hanafubuki_agent_compression::{
    hide_consumed_compress_calls, sync_blocks, Block, Message, State,
};

use crate::fixtures::{
    apply, call, config, ids, range, result, state_for, user, with_call, with_topic,
};

fn block(block_id: &str, call_id: &str, active: bool) -> Block {
    Block {
        block_id: block_id.to_owned(),
        run_id: "r0".to_owned(),
        summary: "summary".to_owned(),
        compress_call_id: Some(call_id.to_owned()),
        active,
        ..Block::default()
    }
}

fn ranged(block_id: &str, call_id: &str, active: bool, start: &str, end: &str) -> Block {
    Block {
        start_ref: Some(start.to_owned()),
        end_ref: Some(end.to_owned()),
        ..block(block_id, call_id, active)
    }
}

fn state_with(blocks: Vec<Block>) -> State {
    State {
        blocks,
        ..State::default()
    }
}

fn compress_call(id: &str, call_id: &str, arguments: &str) -> Message {
    call(id, call_id, "compress", arguments)
}

/// Refs `m00001`, `m00002`, ... by position, in `by_raw` only.
fn sequential_refs(state: &mut State, messages: &[Message]) {
    for (index, message) in messages.iter().enumerate() {
        state
            .message_refs
            .by_raw
            .insert(message.id.clone(), format!("m{:05}", index + 1));
    }
}

/// A failed `compress` exchange; its result carries no tool name.
fn fail_pair(call_id: &str) -> [Message; 2] {
    let arguments =
        json!({"content": [{"startId": "mA", "endId": "mB", "summary": "s"}]}).to_string();
    let mut outcome = result(
        &format!("r-{call_id}"),
        call_id,
        "compress",
        "nothing to compress",
    );
    outcome.tool_name = None;
    [
        compress_call(&format!("c-{call_id}"), call_id, &arguments),
        outcome,
    ]
}

fn content(text: &str) -> Vec<Value> {
    let start = text.find('{').expect("JSON arguments");
    let parsed: Value = serde_json::from_str(&text[start..]).expect("valid JSON");
    match &parsed["content"] {
        Value::Array(entries) => entries.clone(),
        Value::String(inner) => {
            serde_json::from_str::<Vec<Value>>(inner).expect("stringified array")
        }
        other => panic!("unexpected content {other}"),
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn arguments(entries: Value) -> String {
    json!({ "content": entries }).to_string()
}

#[test]
fn a_consumed_blocks_call_and_result_are_both_hidden() {
    let messages = vec![
        user("u1", "hi"),
        compress_call(
            "a1",
            "call-old",
            r#"{"content":[{"startId":"m0","endId":"m2","summary":"old"}]}"#,
        ),
        result("a1r", "call-old", "compress", "Compressed"),
        user("u2", "next"),
    ];
    let hidden =
        hide_consumed_compress_calls(&state_with(vec![block("b0", "call-old", false)]), &messages);
    assert_eq!(hidden.hidden, 2);
    assert_eq!(ids(&hidden.messages), ["u1", "u2"]);
}

#[test]
fn an_active_blocks_call_and_result_are_kept() {
    let messages = vec![
        user("u1", "hi"),
        compress_call(
            "a1",
            "call-active",
            r#"{"content":[{"startId":"m0","endId":"m2","summary":"active"}]}"#,
        ),
        result("a1r", "call-active", "compress", "Compressed"),
    ];
    let hidden = hide_consumed_compress_calls(
        &state_with(vec![block("b0", "call-active", true)]),
        &messages,
    );
    assert_eq!(hidden.hidden, 0);
    assert_eq!(hidden.messages.len(), 3);
}

#[test]
fn only_the_refs_of_hidden_orphan_calls_are_recorded() {
    let generic = r#"{"content":[{"startId":"m00001","endId":"m00002","summary":"s"}]}"#;
    let messages = vec![
        compress_call("k1", "c1", generic),
        result("r1", "c1", "compress", "compressed"),
        compress_call("k2", "c2", generic),
        result("r2", "c2", "compress", "compressed"),
        compress_call("k3", "c3", generic),
        result("r3", "c3", "compress", "compressed"),
    ];
    let mut state = state_for(&messages);
    state.swept_orphan_call_ids = vec!["c1".to_owned()];
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.hidden, 2, "the swept orphan pair stays hidden");
    assert_eq!(hidden.hidden_orphan_refs, ["m00001", "m00002"]);
}

fn three_failures() -> Vec<Message> {
    let mut messages = vec![user("p1", "early")];
    messages.extend(fail_pair("f1"));
    messages.push(user("p2", "mid"));
    messages.extend(fail_pair("f2"));
    messages.push(user("p3", "late"));
    messages.extend(fail_pair("f3"));
    messages
}

#[test]
fn a_quiet_turn_never_rewrites_mid_history() {
    let messages = three_failures();
    let mut state = State::default();
    sequential_refs(&mut state, &messages);
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.hidden, 0);
    assert!(hidden.newly_swept_call_ids.is_empty());
    assert_eq!(hidden.messages, messages);
}

#[test]
fn a_fold_sweeps_orphans_from_its_start_and_keeps_earlier_ones() {
    let messages = three_failures();
    let mut state = State::default();
    sequential_refs(&mut state, &messages);
    state.pending_orphan_sweep_min_ref = Some(5);
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.newly_swept_call_ids, ["f2", "f3"]);
    let kept = ids(&hidden.messages);
    assert!(kept.contains(&"c-f1"));
    assert!(!kept.contains(&"c-f2") && !kept.contains(&"c-f3"));
}

#[test]
fn a_swept_orphan_stays_hidden_on_later_quiet_turns() {
    let mut messages = vec![user("p1", "early")];
    messages.extend(fail_pair("f1"));
    messages.push(user("p2", "mid"));
    messages.extend(fail_pair("f2"));
    let mut state = State::default();
    sequential_refs(&mut state, &messages);
    state.pending_orphan_sweep_min_ref = Some(5);
    let first = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(first.newly_swept_call_ids, ["f2"]);

    first.commit(&mut state);
    assert_eq!(state.swept_orphan_call_ids, ["f2"]);
    assert_eq!(state.pending_orphan_sweep_min_ref, None);
    let second = hide_consumed_compress_calls(&state, &messages);
    assert!(second.newly_swept_call_ids.is_empty());
    let kept = ids(&second.messages);
    assert!(!kept.contains(&"c-f2"));
    assert!(kept.contains(&"c-f1"));
}

#[test]
fn failures_sweep_nothing_until_one_fold_sweeps_them_all() {
    for count in 1..=3 {
        let mut messages = vec![user("p1", "early")];
        for failure in 1..=count {
            messages.extend(fail_pair(&format!("f{failure}")));
        }
        let mut state = State::default();
        sequential_refs(&mut state, &messages);
        let hidden = hide_consumed_compress_calls(&state, &messages);
        assert_eq!(hidden.hidden, 0, "{count} failures, no fold");
        assert!(hidden.newly_swept_call_ids.is_empty());
    }
    let mut messages = vec![user("p1", "early")];
    for failure in 1..=3 {
        messages.extend(fail_pair(&format!("f{failure}")));
    }
    let mut state = State::default();
    sequential_refs(&mut state, &messages);
    state.pending_orphan_sweep_min_ref = Some(1);
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.newly_swept_call_ids, ["f1", "f2", "f3"]);
}

#[test]
fn a_real_fold_marks_the_sweep_and_the_next_pass_removes_the_orphan_behind_it() {
    let mut messages = vec![
        user("h1", "first detailed work item alpha content"),
        user("h2", "second detailed work item beta content"),
        user("h3", "third detailed work item gamma content"),
    ];
    messages.extend(fail_pair("f1"));
    messages.push(user("h4", "fourth detailed work item delta content"));
    messages.push(user("last", "latest user intent theta"));
    let state = state_for(&messages);
    let spec = with_topic(range("m00001", "m00003", "fold the spent head"), "head");
    let outcome = apply(&[spec], &messages, &state, &config());
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.blocks_created >= 1);
    assert_eq!(outcome.state.pending_orphan_sweep_min_ref, Some(1));

    let hidden = hide_consumed_compress_calls(&outcome.state, &messages);
    assert_eq!(hidden.newly_swept_call_ids, ["f1"]);
    assert!(!hidden
        .messages
        .iter()
        .any(|m| m.tool_call_id.as_deref() == Some("f1")));
}

#[test]
fn a_block_range_fold_marks_the_sweep_by_covered_refs_not_the_block_number() {
    let mut messages = vec![user("p1", "early cached prefix alpha")];
    messages.extend(fail_pair("f1"));
    messages.extend([
        user("p2", "mid detailed work item beta content"),
        user("p3", "mid detailed work item gamma content"),
        user("p4", "mid detailed work item delta content"),
        user("p5", "mid detailed work item epsilon content"),
        user("p6", "latest user intent theta"),
    ]);
    let state = state_for(&messages);
    let first = apply(
        &[with_topic(
            range("m00004", "m00005", "fold the mid pair"),
            "mid",
        )],
        &messages,
        &state,
        &config(),
    );
    assert!(first.result.errors.is_empty(), "{:?}", first.result.errors);
    let block_id = first.state.blocks.last().expect("a block").block_id.clone();

    let spec = with_topic(
        range(&block_id, "m00006", "distill the block plus its tail"),
        "distill",
    );
    let outcome = apply(&[spec], &messages, &first.state, &config());
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.blocks_created >= 1);
    assert_eq!(outcome.state.pending_orphan_sweep_min_ref, Some(4));

    let hidden = hide_consumed_compress_calls(&outcome.state, &messages);
    assert!(hidden.newly_swept_call_ids.is_empty());
    assert!(hidden
        .messages
        .iter()
        .any(|m| m.tool_call_id.as_deref() == Some("f1")));
}

#[test]
fn an_upgraded_state_keeps_its_rotation_hidden_orphan_hidden() {
    let messages = three_failures();
    let mut state = State::default();
    sequential_refs(&mut state, &messages);
    state.hidden_orphan_refs = vec!["m00002".to_owned(), "m00003".to_owned()];
    let first = hide_consumed_compress_calls(&state, &messages);
    assert!(first.newly_swept_call_ids.is_empty());
    let kept = ids(&first.messages);
    assert!(!kept.contains(&"c-f1") && !kept.contains(&"r-f1"));
    assert!(kept.contains(&"c-f2") && kept.contains(&"c-f3"));

    state.hidden_orphan_refs = first.hidden_orphan_refs.clone();
    let second = hide_consumed_compress_calls(&state, &messages);
    let kept = ids(&second.messages);
    assert!(!kept.contains(&"c-f1") && !kept.contains(&"r-f1"));
}

fn carrier_history(call_id: &str, call_text: &str) -> Vec<Message> {
    let mut outcome = result(
        &format!("r-{call_id}"),
        call_id,
        "compress",
        "compressed the head",
    );
    outcome.tool_name = None;
    vec![
        user("h1", "first detailed work item alpha content"),
        user("h2", "second detailed work item beta content"),
        user("h3", "third detailed work item gamma content"),
        compress_call(&format!("c-{call_id}"), call_id, call_text),
        outcome,
        user("last", "latest user intent theta"),
    ]
}

fn fold_head(messages: &[Message]) -> State {
    let state = state_for(messages);
    let spec = with_call(
        with_topic(range("m00001", "m00003", "fold the spent head"), "head"),
        "plugin_1234",
    );
    let outcome = apply(&[spec], messages, &state, &config());
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.blocks_created >= 1);
    outcome.state
}

#[test]
fn a_resent_pair_quoting_an_active_blocks_range_is_its_carrier_and_survives_the_sweep() {
    let flat = json!({"startId": "m00001", "endId": "m00003", "topic": "head", "summary": "fold the spent head"});
    let carriers = [flat.to_string(), arguments(json!([flat]))];
    for call_text in carriers {
        let messages = carrier_history("call_echo_1", &call_text);
        let state = fold_head(&messages);
        let hidden = hide_consumed_compress_calls(&state, &messages);
        assert!(hidden.newly_swept_call_ids.is_empty(), "{call_text}");
        assert!(hidden
            .messages
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("call_echo_1")));
    }

    let failure = arguments(json!([{"startId": "mX", "endId": "mY", "summary": "s"}]));
    let messages = carrier_history("call_fail_9", &failure);
    let state = fold_head(&messages);
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.newly_swept_call_ids, ["call_fail_9"]);
}

// The kernel ran this through processTurn; the steps it relies on are sync
// and the hide step with its commit.
#[test]
fn the_fold_marker_reaches_the_hide_step_and_is_consumed_once() {
    let mut messages = vec![user("p1", "early cached prefix alpha")];
    messages.extend(fail_pair("f2"));
    messages.push(user("p2", "mid detailed work item beta content"));
    let mut state = State::default();
    sequential_refs(&mut state, &messages);
    state.pending_orphan_sweep_min_ref = Some(2);

    sync_blocks(&messages, &mut state);
    let hidden = hide_consumed_compress_calls(&state, &messages);
    hidden.commit(&mut state);
    let kept = ids(&hidden.messages);
    assert!(!kept.contains(&"c-f2") && !kept.contains(&"r-f2"));
    assert_eq!(state.swept_orphan_call_ids, ["f2"]);
    assert_eq!(state.pending_orphan_sweep_min_ref, None);

    messages.push(user("p3", "latest user intent theta"));
    sync_blocks(&messages, &mut state);
    let quiet = hide_consumed_compress_calls(&state, &messages);
    assert!(!ids(&quiet.messages).contains(&"c-f2"));
}

fn live_state(summary: &str, start: &str, end: &str) -> State {
    let mut live = ranged("b1", "call1", true, start, end);
    live.summary = summary.to_owned();
    state_with(vec![live])
}

#[test]
fn a_live_calls_long_summaries_are_stubbed() {
    let long = "x".repeat(5000);
    let text =
        arguments(json!([{"startId": "m00001", "endId": "m00050", "topic": "T", "summary": long}]));
    let state = live_state(&long, "m00001", "m00050");
    let hidden = hide_consumed_compress_calls(&state, &[compress_call("mc1", "call1", &text)]);
    assert_eq!(hidden.messages.len(), 1);
    let entries = content(&hidden.messages[0].text);
    assert_eq!(entries.len(), 1);
    let summary = entries[0]["summary"].as_str().expect("summary");
    assert_eq!(utf16_len(summary), 200);
    assert!(summary.ends_with('…') && summary.starts_with("xxxx"));
    assert_eq!(entries[0]["topic"], "T");
    assert_eq!(entries[0]["startId"], "m00001");
}

#[test]
fn short_summaries_stay_byte_identical_and_kept_orphans_are_stubbed() {
    // Written as the model writes it, keys unsorted.
    let short =
        r#"{"content":[{"startId":"m00001","endId":"m00050","summary":"short"}]}"#.to_owned();
    let state = live_state("short", "m00001", "m00050");
    let hidden = hide_consumed_compress_calls(&state, &[compress_call("mc1", "call1", &short)]);
    assert_eq!(hidden.messages[0].text, short);

    let orphan =
        arguments(json!([{"startId": "m00001", "endId": "m00050", "summary": "y".repeat(4000)}]));
    let hidden = hide_consumed_compress_calls(
        &State::default(),
        &[compress_call("mc1", "orphan", &orphan)],
    );
    assert_eq!(hidden.messages.len(), 1);
    let entries = content(&hidden.messages[0].text);
    assert_eq!(
        utf16_len(entries[0]["summary"].as_str().expect("summary")),
        200
    );
}

const TAG: &str = "<acp tokens=\"1.2K\" type=\"compress\">m00010</acp>\n";

// The kernel tagged the call with its "all" render strategy, which the port
// does not keep; a host may still prefix the text with a tag.
#[test]
fn a_tag_prefixed_call_is_still_parsed_and_stubbed() {
    let long = "z".repeat(3000);
    let state = live_state(&long, "m00001", "m00050");
    let text = format!(
        "{TAG}{}",
        arguments(json!([{"startId": "m00001", "endId": "m00050", "summary": long}]))
    );
    let hidden = hide_consumed_compress_calls(&state, &[compress_call("mc1", "call1", &text)]);
    let out = &hidden.messages[0].text;
    assert!(out.starts_with(TAG));
    let entries = content(out);
    assert_eq!(
        utf16_len(entries[0]["summary"].as_str().expect("summary")),
        200
    );
}

fn batch_state(second_active: bool) -> State {
    state_with(vec![
        ranged("b5", "call-batch", true, "m5", "m6"),
        ranged("b8", "call-batch", second_active, "m8", "m9"),
    ])
}

#[test]
fn a_tag_prefixed_mixed_call_still_drops_its_consumed_sibling() {
    let text = format!(
        "{TAG}{}",
        arguments(json!([
            {"startId": "m5", "endId": "m6", "summary": "live entry summary"},
            {"startId": "m8", "endId": "m9", "summary": "c".repeat(300)},
        ]))
    );
    let hidden = hide_consumed_compress_calls(
        &batch_state(false),
        &[compress_call("mc", "call-batch", &text)],
    );
    let out = &hidden.messages[0].text;
    assert!(out.starts_with(TAG));
    let entries = content(out);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["startId"], "m5");
    assert_eq!(entries[0]["summary"], "live entry summary");
}

#[test]
fn a_stringified_content_array_is_handled_and_stays_a_string() {
    let state = state_with(vec![
        ranged("b1", "call-str", true, "m1", "m2"),
        ranged("b2", "call-str", false, "m3", "m4"),
    ]);
    let inner = json!([
        {"startId": "m1", "endId": "m2", "summary": "x".repeat(300)},
        {"startId": "m3", "endId": "m4", "summary": "y".repeat(300)},
    ])
    .to_string();
    let text = json!({ "content": inner }).to_string();
    let hidden = hide_consumed_compress_calls(&state, &[compress_call("m5", "call-str", &text)]);
    let kept = &hidden.messages[0].text;
    let parsed: Value = serde_json::from_str(kept).expect("valid JSON");
    assert!(parsed["content"].is_string(), "string shape preserved");
    let entries = content(kept);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["startId"], "m1");
    let summary = entries[0]["summary"].as_str().expect("summary");
    assert!(utf16_len(summary) <= 200 && summary.ends_with('…'));
}

#[test]
fn active_carriers_and_orphans_stay_until_a_fold() {
    let state = state_with(vec![block("b1", "call-active", true)]);
    let messages = vec![
        compress_call("m1", "call-active", "{}"),
        compress_call("m2", "call-consumed", "{}"),
        compress_call("m3", "call-orphan1", "{}"),
        compress_call("m4", "call-orphan2", "{}"),
        user("m5", "hello"),
    ];
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.hidden, 0);
    assert_eq!(hidden.messages, messages);
}

#[test]
fn orphan_residue_is_swept_onto_a_committed_fold() {
    let mut state = state_with(vec![block("b1", "call-active", true)]);
    let mut messages = vec![compress_call("m1", "call-active", "{}")];
    for failure in 1..=12 {
        let call_id = format!("call-fail{failure}");
        messages.push(compress_call(&format!("c{failure}"), &call_id, "{}"));
        messages.push(result(
            &format!("r{failure}"),
            &call_id,
            "compress",
            "nothing to compress",
        ));
    }
    messages.push(user("mend", "tail"));
    sequential_refs(&mut state, &messages);
    state.pending_orphan_sweep_min_ref = Some(10);

    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.hidden, 16);
    let swept: Vec<String> = (5..=12)
        .map(|failure| format!("call-fail{failure}"))
        .collect();
    assert_eq!(hidden.newly_swept_call_ids, swept);
    let remaining: Vec<&str> = hidden
        .messages
        .iter()
        .filter(|m| {
            m.tool_name.as_deref() == Some("compress")
                && m.kind == hanafubuki_agent_compression::Kind::ToolCall
        })
        .filter_map(|m| m.tool_call_id.as_deref())
        .collect();
    assert_eq!(
        remaining,
        [
            "call-active",
            "call-fail1",
            "call-fail2",
            "call-fail3",
            "call-fail4"
        ]
    );
}

#[test]
fn a_failed_call_and_its_result_stay_visible() {
    let messages = vec![
        user("m1", "q"),
        compress_call("m2", "call-fail", "{}"),
        result(
            "m3",
            "call-fail",
            "compress",
            "Requested range(s) already compressed; nothing to do",
        ),
    ];
    let hidden = hide_consumed_compress_calls(&State::default(), &messages);
    assert_eq!(hidden.hidden, 0);
    assert_eq!(hidden.messages, messages);
}

fn batch_call(entries: Value) -> Message {
    compress_call("mc", "call-batch", &arguments(entries))
}

#[test]
fn a_kept_batch_drops_its_consumed_sibling_entries() {
    let messages = [batch_call(json!([
        {"startId": "m5", "endId": "m6", "summary": "live entry summary"},
        {"startId": "m8", "endId": "m9", "summary": "consumed entry summary"},
    ]))];
    let hidden = hide_consumed_compress_calls(&batch_state(false), &messages);
    assert_eq!(hidden.hidden, 0);
    let entries = content(&hidden.messages[0].text);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["startId"], "m5");
    assert_eq!(entries[0]["summary"], "live entry summary");
}

#[test]
fn a_batch_whose_siblings_are_all_live_is_not_rewritten() {
    // Written as the model writes it, keys unsorted.
    let original = compress_call(
        "mc",
        "call-batch",
        r#"{"content":[{"startId":"m5","endId":"m6","summary":"S1"},{"startId":"m8","endId":"m9","summary":"S2"}]}"#,
    );
    let hidden = hide_consumed_compress_calls(&batch_state(true), std::slice::from_ref(&original));
    assert_eq!(hidden.hidden, 0);
    assert_eq!(hidden.messages[0].text, original.text, "byte-identical");
    assert_eq!(content(&hidden.messages[0].text).len(), 2);
}

#[test]
fn a_batch_whose_siblings_are_all_consumed_is_removed() {
    let state = state_with(vec![
        ranged("b5", "call-batch", false, "m5", "m6"),
        ranged("b8", "call-batch", false, "m8", "m9"),
    ]);
    let messages = [batch_call(json!([
        {"startId": "m5", "endId": "m6", "summary": "S1"},
        {"startId": "m8", "endId": "m9", "summary": "S2"},
    ]))];
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.hidden, 1);
    assert!(hidden.messages.is_empty());
}

#[test]
fn a_legacy_live_block_without_refs_keeps_the_whole_batch() {
    let state = state_with(vec![
        ranged("b5", "call-batch", true, "m5", "m6"),
        block("b8", "call-batch", true),
    ]);
    let messages = [batch_call(json!([
        {"startId": "m5", "endId": "m6", "summary": "S1"},
        {"startId": "m8", "endId": "m9", "summary": "S2"},
    ]))];
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.hidden, 0);
    assert_eq!(content(&hidden.messages[0].text).len(), 2);
}

#[test]
fn a_message_mode_entry_matches_through_its_message_id() {
    let state = state_with(vec![
        ranged("b1", "call-batch", true, "msg-3", "msg-3"),
        ranged("b2", "call-batch", false, "msg-7", "msg-7"),
    ]);
    let messages = [batch_call(json!([
        {"messageId": "msg-3", "summary": "live message-mode summary"},
        {"messageId": "msg-7", "summary": "consumed message-mode summary"},
    ]))];
    let hidden = hide_consumed_compress_calls(&state, &messages);
    assert_eq!(hidden.hidden, 0);
    let entries = content(&hidden.messages[0].text);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["messageId"], "msg-3");
    assert_eq!(entries[0]["summary"], "live message-mode summary");
}

#[test]
fn a_batch_matching_no_live_range_is_kept_verbatim() {
    let state = state_with(vec![ranged("b1", "call-batch", true, "m5", "m6")]);
    let original = batch_call(json!([
        {"startId": "mx", "endId": "my", "summary": "unmatched entry"},
        {"startId": "mz", "endId": "mw", "summary": "another unmatched"},
    ]));
    let hidden = hide_consumed_compress_calls(&state, std::slice::from_ref(&original));
    assert_eq!(hidden.hidden, 0);
    assert_eq!(hidden.messages[0].text, original.text);
}

#[test]
fn a_rewritten_batch_keeps_its_tool_result() {
    let mut outcome = result("mc-result", "call-batch", "compress", "{\"ok\":true}");
    outcome.tool_name = None;
    let messages = [
        batch_call(json!([
            {"startId": "m5", "endId": "m6", "summary": "S1"},
            {"startId": "m8", "endId": "m9", "summary": "S2"},
        ])),
        outcome,
    ];
    let hidden = hide_consumed_compress_calls(&batch_state(false), &messages);
    assert_eq!(hidden.hidden, 0);
    assert_eq!(ids(&hidden.messages), ["mc", "mc-result"]);
    assert_eq!(content(&hidden.messages[0].text).len(), 1);
}
