// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports state-prune, orphan-fixes (prune), prune-orphan-mcp (bare name),
// issue4311-summary-dedup and the syncBlocks cases of token-snapshot.

use std::collections::BTreeMap;

use hanafubuki_agent_compression::{
    prune, sync_blocks, Block, Generation, Kind, Message, Role, State, SUMMARY_HEADER,
};

use crate::fixtures::{
    apply, assistant, call, config, ids, range, reasoning, result, state_for, user, with_call,
};

fn block(block_id: &str, effective: &[&str]) -> Block {
    Block {
        block_id: block_id.to_owned(),
        run_id: "r1".to_owned(),
        summary: "summary".to_owned(),
        effective_message_ids: effective.iter().map(|id| (*id).to_owned()).collect(),
        ..Block::default()
    }
}

fn state_with(blocks: Vec<Block>) -> State {
    State {
        blocks,
        ..State::default()
    }
}

fn text(id: &str) -> Message {
    user(id, id)
}

#[test]
fn initial_state_is_empty_with_id_counters_at_one() {
    let state = State::default();
    assert!(state.blocks.is_empty());
    assert_eq!(state.next_block_id, 1);
    assert_eq!(state.next_run_id, 1);
    assert_eq!(state.stats.tokens_compressed, 0);
}

#[test]
fn block_by_id_finds_a_block() {
    let state = state_with(vec![block("b3", &[])]);
    assert_eq!(
        state.block("b3").map(|block| block.block_id.as_str()),
        Some("b3")
    );
    assert!(state.block("b9").is_none());
}

#[test]
fn active_blocks_filters_inactive() {
    let mut inactive = block("b2", &[]);
    inactive.active = false;
    let state = state_with(vec![block("b1", &[]), inactive]);
    assert_eq!(state.active_blocks().count(), 1);
}

#[test]
fn covered_message_ids_unions_active_blocks() {
    let mut inactive = block("b2", &["m3"]);
    inactive.active = false;
    let state = state_with(vec![
        block("b1", &["m1", "m2"]),
        inactive,
        block("b3", &["m2", "m4"]),
    ]);
    let mut covered: Vec<&str> = state.covered_message_ids().into_iter().collect();
    covered.sort();
    assert_eq!(covered, ["m1", "m2", "m4"]);
}

#[test]
fn highest_active_tier_is_the_max_among_active_blocks() {
    let mut tier2 = block("b2", &[]);
    tier2.tier = 2;
    tier2.active = false;
    let mut tier3 = block("b3", &[]);
    tier3.tier = 3;
    let state = state_with(vec![block("b1", &[]), tier2, tier3]);
    assert_eq!(state.highest_active_tier(), 3);
}

#[test]
fn advance_survival_ages_active_blocks_and_promotes_past_the_threshold() {
    let mut first = block("b1", &[]);
    first.survived_count = 4;
    let mut inactive = block("b2", &[]);
    inactive.survived_count = 1;
    inactive.active = false;
    let mut state = state_with(vec![first, inactive]);
    state.advance_survival(5);
    assert_eq!(state.blocks[0].survived_count, 5);
    assert_eq!(state.blocks[0].generation, Generation::Old);
    assert_eq!(state.blocks[1].survived_count, 1);
}

#[test]
fn prune_returns_messages_unchanged_when_nothing_is_covered() {
    let messages = vec![text("a"), text("b")];
    assert_eq!(prune(&messages, &State::default()), messages);
}

#[test]
fn prune_removes_covered_messages_and_injects_the_summary_at_the_anchor() {
    let mut covering = block("b1", &["m2", "m3"]);
    covering.summary = "the summary".to_owned();
    let state = state_with(vec![covering]);
    let messages = vec![text("m1"), text("m2"), text("m3"), text("m4")];
    let pruned = prune(&messages, &state);
    assert_eq!(ids(&pruned), ["m1", "acp_summary_b1", "m4"]);
    assert!(pruned[1].text.starts_with(SUMMARY_HEADER));
    assert!(pruned[1].text.contains("the summary"));
    assert_eq!((pruned[1].role, pruned[1].kind), (Role::System, Kind::Text));
}

// The kernel test ran with summary injection off, which the port does not
// keep; the pin holds the same way with the summary rendered after it.
#[test]
fn prune_preserves_the_first_user_message_even_when_covered() {
    let state = state_with(vec![block("b1", &["m1", "m2"])]);
    let messages = vec![text("m1"), assistant("m2", "m2"), assistant("m3", "m3")];
    let pruned = prune(&messages, &state);
    assert_eq!(ids(&pruned), ["m1", "acp_summary_b1", "m3"]);
}

#[test]
fn prune_orders_multiple_summaries_by_their_anchor_position() {
    let mut later = block("b1", &["x"]);
    later.summary = "later".to_owned();
    let mut earlier = block("b2", &["w"]);
    earlier.summary = "earlier".to_owned();
    let state = state_with(vec![later, earlier]);
    let messages = vec![text("u"), text("w"), text("x"), text("z")];
    let pruned = prune(&messages, &state);
    assert_eq!(ids(&pruned), ["u", "acp_summary_b2", "acp_summary_b1", "z"]);
    assert!(pruned[1].text.contains("earlier"));
    assert!(pruned[2].text.contains("later"));
}

#[test]
fn an_unresolved_anchor_parks_before_the_newest_user_message() {
    let mut orphaned = block("b1", &["gone1", "gone2"]);
    orphaned.summary = "orphaned history summary".to_owned();
    let state = state_with(vec![orphaned]);
    let messages = vec![
        assistant("keep1", "keep1"),
        assistant("keep2", "keep2"),
        user("q", "q"),
    ];
    let pruned = prune(&messages, &state);
    let anchor = pruned
        .iter()
        .position(|m| m.id == "acp_summary_b1")
        .expect("summary renders");
    let question = pruned
        .iter()
        .position(|m| m.id == "q")
        .expect("question stays");
    assert!(
        question > anchor,
        "summary {anchor} must precede the newest user turn {question}"
    );
}

#[test]
fn without_a_user_turn_an_unresolved_anchor_takes_the_tail() {
    let state = state_with(vec![block("b1", &["gone1"])]);
    let messages = vec![assistant("a1", "a1"), assistant("a2", "a2")];
    let pruned = prune(&messages, &state);
    assert_eq!(pruned.last().map(|m| m.id.as_str()), Some("acp_summary_b1"));
}

#[test]
fn prune_strips_a_tool_result_whose_call_folded() {
    let messages = vec![
        user("u0", "start"),
        assistant("a0", "ok"),
        user("u1", "read file"),
        call("a1", "call-1", "read", "{\"path\":\"foo.ts\"}"),
        result("a1r", "call-1", "read", "file contents here"),
        user("u2", "next task"),
    ];
    let mut covering = block("b0", &["u1", "a1"]);
    covering.direct_message_ids = covering.effective_message_ids.clone();
    let pruned = prune(&messages, &state_with(vec![covering]));
    let kept = ids(&pruned);
    assert!(!kept.contains(&"a1"), "the folded call is pruned");
    assert!(!kept.contains(&"a1r"), "its orphaned result is stripped");
    assert!(kept.contains(&"u0") && kept.contains(&"u2"));
}

#[test]
fn prune_keeps_a_tool_result_whose_call_is_outside_the_fold() {
    let messages = vec![
        user("u0", "start"),
        assistant("a0", "ok"),
        call("a1", "call-1", "read", "{\"path\":\"foo.ts\"}"),
        result("a1r", "call-1", "read", "file contents"),
    ];
    let mut covering = block("b0", &["u0"]);
    covering.direct_message_ids = vec!["u0".to_owned()];
    let pruned = prune(&messages, &state_with(vec![covering]));
    let kept = ids(&pruned);
    assert!(kept.contains(&"a1") && kept.contains(&"a1r"));
}

#[test]
fn a_covered_first_user_message_survives_once_beside_its_summary() {
    let messages = vec![
        user("u0", "critical first prompt"),
        assistant("a0", "reply"),
        user("u1", "second"),
        assistant("a1", "reply2"),
    ];
    let mut covering = block("b0", &["u0", "a0", "u1"]);
    covering.direct_message_ids = covering.effective_message_ids.clone();
    let pruned = prune(&messages, &state_with(vec![covering]));
    let kept = ids(&pruned);
    assert_eq!(kept.iter().filter(|id| **id == "u0").count(), 1);
    assert!(!kept.contains(&"a0") && !kept.contains(&"u1"));
    assert!(kept.iter().any(|id| id.starts_with("acp_summary_")));
}

fn pending_call_sequence(tool: &str) -> Vec<Message> {
    vec![
        user("raw1", "a"),
        assistant("raw2", "b"),
        user("later", "c"),
        call("pend", "c1", tool, "{}"),
    ]
}

#[test]
fn a_pending_compress_call_survives_the_orphan_strip() {
    let state = state_with(vec![block("b1", &["raw1", "raw2"])]);
    let pruned = prune(&pending_call_sequence("compress"), &state);
    assert!(ids(&pruned).contains(&"pend"));
}

#[test]
fn an_unanswered_call_of_any_other_tool_is_stripped() {
    let state = state_with(vec![block("b1", &["raw1", "raw2"])]);
    for tool in ["bash", "mcp__bili__compress"] {
        let pruned = prune(&pending_call_sequence(tool), &state);
        assert!(!ids(&pruned).contains(&"pend"), "{tool} orphan stripped");
    }
}

#[test]
fn prune_drops_a_reasoning_run_that_lost_its_assistant_act() {
    let messages = vec![
        user("u", "question"),
        reasoning("r1", "thinking"),
        assistant("t1", "answer"),
        user("u2", "next"),
        reasoning("r2", "thinking more"),
        assistant("t2", "second answer"),
    ];
    let state = state_with(vec![block("b1", &["t2"])]);
    let pruned = prune(&messages, &state);
    assert_eq!(ids(&pruned), ["u", "r1", "t1", "u2", "acp_summary_b1"]);
}

const LONG: &str = "this message is long enough to fold into a block for the dedup tests";
const VALID_SUMMARY: &str = "A meaningful summary that captures the key information of the compressed range including file paths and decisions.";

/// `u1` folded by the call `call_id`, or by no call.
fn folded_u1(call_id: Option<&str>) -> State {
    let messages = vec![user("u1", LONG), user("u2", LONG)];
    let state = state_for(&messages);
    let spec = range("m00001", "m00001", VALID_SUMMARY);
    let spec = match call_id {
        Some(call_id) => with_call(spec, call_id),
        None => spec,
    };
    let outcome = apply(&[spec], &messages, &state, &config());
    assert_eq!(outcome.result.blocks_created, 1, "{:?}", outcome.result);
    outcome.state
}

fn summary_renders(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|m| m.id.starts_with("acp_summary_"))
        .count()
}

fn compress_exchange(receipt: &str) -> Vec<Message> {
    let arguments = r#"{"content":[{"startId":"m00001","endId":"m00001","summary":"s"}]}"#;
    vec![
        call("sc1", "call-ok", "compress", arguments),
        result("sr1", "call-ok", "compress", receipt),
    ]
}

#[test]
fn a_visible_originating_call_suppresses_the_summary_render() {
    let state = folded_u1(Some("call-ok"));
    assert_eq!(state.blocks[0].compress_call_id.as_deref(), Some("call-ok"));
    let mut view = vec![user("u1", LONG)];
    view.extend(compress_exchange(
        "[Compressed m00001–m00001 → 1 block(s), ~1234 tokens saved.]",
    ));
    view.push(user("u2", LONG));
    let pruned = prune(&view, &state);
    assert_eq!(summary_renders(&pruned), 0);
    assert!(ids(&pruned).contains(&"sc1") && ids(&pruned).contains(&"sr1"));
}

#[test]
fn the_render_returns_once_the_call_is_gone() {
    let state = folded_u1(Some("call-ok"));
    let pruned = prune(&[user("u1", LONG), user("u2", LONG)], &state);
    assert_eq!(summary_renders(&pruned), 1);
}

#[test]
fn a_block_without_a_compress_call_always_renders() {
    let state = folded_u1(None);
    let mut view = vec![user("u1", LONG)];
    view.extend(compress_exchange(
        "[Compressed m00003–m00004 → 1 block(s), ~1234 tokens saved.]",
    ));
    view.push(user("u2", LONG));
    assert_eq!(summary_renders(&prune(&view, &state)), 1);
}

#[test]
fn a_stale_render_is_dropped_and_not_reinjected_while_the_call_is_visible() {
    let state = folded_u1(Some("call-ok"));
    let stale = Message::text(
        format!("acp_summary_{}", state.blocks[0].block_id),
        Role::System,
        "[Compressed conversation section] — stale copy",
    );
    let mut view = vec![user("u1", LONG), stale];
    view.extend(compress_exchange(
        "[Compressed m00001–m00001 → 1 block(s), ~1234 tokens saved.]",
    ));
    view.push(user("u2", LONG));
    assert_eq!(summary_renders(&prune(&view, &state)), 0);
}

#[test]
fn sync_blocks_keeps_the_snapshot_of_present_refs() {
    let messages = vec![user("a", "hello world this is a longer message for tokens")];
    let mut state = state_for(&messages);
    state.token_snapshot = BTreeMap::from([("m00001".to_owned(), 24)]);
    sync_blocks(&messages, &mut state);
    assert_eq!(
        state.token_snapshot,
        BTreeMap::from([("m00001".to_owned(), 24)])
    );
}

#[test]
fn sync_blocks_prunes_the_snapshot_of_absent_messages() {
    let messages = vec![
        user("a", "first message here"),
        user("b", "second message here"),
    ];
    let mut state = state_for(&messages);
    state.token_snapshot = BTreeMap::from([("m00001".to_owned(), 9), ("m00002".to_owned(), 10)]);
    sync_blocks(&[user("b", "second message here")], &mut state);
    assert!(!state.token_snapshot.contains_key("m00001"));
    assert!(state.token_snapshot.contains_key("m00002"));
}

#[test]
fn sync_blocks_deactivates_consumed_expanded_and_vanished_blocks_and_revives_returning_ones() {
    let mut consumer = block("b3", &["a", "b"]);
    consumer.direct_block_ids = vec!["b1".to_owned()];
    let mut expanded = block("b2", &["c"]);
    expanded.expanded = true;
    let mut returning = block("b4", &["d"]);
    returning.active = false;
    let gone = block("b5", &["gone"]);
    let mut state = state_with(vec![
        block("b1", &["a"]),
        expanded,
        consumer,
        returning,
        gone,
    ]);
    let messages = vec![text("a"), text("b"), text("c"), text("d")];
    let deactivated = sync_blocks(&messages, &mut state);
    let active: Vec<(&str, bool)> = state
        .blocks
        .iter()
        .map(|b| (b.block_id.as_str(), b.active))
        .collect();
    assert_eq!(
        active,
        [
            ("b1", false),
            ("b2", false),
            ("b3", true),
            ("b4", true),
            ("b5", false)
        ]
    );
    assert_eq!(deactivated, ["b5"]);
}

#[test]
fn sync_blocks_keeps_a_block_active_through_its_rendered_summary() {
    let mut state = state_with(vec![block("b1", &["a"])]);
    let view = vec![Message::text(
        "acp_summary_b1",
        Role::System,
        "[Compressed conversation section]",
    )];
    assert!(sync_blocks(&view, &mut state).is_empty());
    assert!(state.blocks[0].active);
}
