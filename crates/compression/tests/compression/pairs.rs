// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `tool-pairs.test.ts`, `tool-pair-integrity.test.ts` and
// `reasoning-pairs.test.ts`.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    adjust_boundaries_for_reasoning_pairs, adjust_boundaries_for_tool_pairs, apply_compression,
    prune, summary_message_id, ApplyInput, ApplyOutcome, Block, Config, Kind, Message, Role, State,
};

use crate::fixtures::{apply, range, state_for, with_topic};

fn text(id: &str) -> Message {
    Message::text(id, Role::User, format!("text-{id}"))
}

fn assistant_text(id: &str) -> Message {
    Message::text(id, Role::Assistant, format!("text-{id}"))
}

fn reasoning(id: &str) -> Message {
    Message::reasoning(id, format!("reasoning-{id}"))
}

fn tool_call(id: &str, call_id: &str, tool: &str) -> Message {
    Message::tool_call(id, call_id, tool, format!("call-{call_id}"))
}

/// Results carry the user role, as the kernel's tests build them.
fn tool_result(id: &str, call_id: &str, tool: &str) -> Message {
    Message {
        role: Role::User,
        ..Message::tool_result(id, call_id, tool, format!("result-{call_id}"))
    }
}

fn default_config() -> Config {
    let mut config = Config::new(100_000);
    config.compress.min_compress_range = 0;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

fn active_block(state: &State) -> &Block {
    state
        .blocks
        .iter()
        .find(|block| block.active)
        .expect("an active block")
}

fn covers(block: &Block, id: &str) -> bool {
    block
        .effective_message_ids
        .iter()
        .any(|covered| covered == id)
}

fn view_ids(messages: &[Message], state: &State) -> Vec<String> {
    prune(messages, state)
        .into_iter()
        .map(|message| message.id)
        .collect()
}

fn has(ids: &[String], id: &str) -> bool {
    ids.iter().any(|known| known == id)
}

// --- adjustBoundariesForToolPairs ---

#[test]
fn tool_pairs_no_op_without_tool_messages() {
    let messages = [text("a"), text("b"), text("c"), text("d")];
    assert_eq!(adjust_boundaries_for_tool_pairs(0, 2, &messages), (0, 2));
}

#[test]
fn tool_pairs_extend_forward_to_the_result() {
    let messages = [
        text("a"),
        tool_call("b", "call1", "read"),
        text("c"),
        tool_result("d", "call1", "read"),
        text("e"),
    ];
    assert_eq!(adjust_boundaries_for_tool_pairs(0, 2, &messages), (0, 3));
}

#[test]
fn tool_pairs_extend_backward_to_the_call() {
    let messages = [
        text("a"),
        tool_call("b", "call1", "read"),
        text("c"),
        tool_result("d", "call1", "read"),
        text("e"),
    ];
    assert_eq!(adjust_boundaries_for_tool_pairs(2, 3, &messages), (1, 3));
}

#[test]
fn tool_pairs_extend_both_directions_for_straddling_pairs() {
    let messages = [
        tool_call("a", "call1", "read"),
        tool_result("b", "call1", "read"),
        tool_call("c", "call2", "read"),
        text("d"),
        tool_result("e", "call2", "read"),
    ];
    assert_eq!(adjust_boundaries_for_tool_pairs(1, 2, &messages), (0, 4));
}

#[test]
fn tool_pairs_skip_the_compress_tool() {
    let messages = [
        text("a"),
        tool_call("b", "comp1", "compress"),
        text("c"),
        tool_result("d", "comp1", "compress"),
        text("e"),
    ];
    assert_eq!(adjust_boundaries_for_tool_pairs(0, 2, &messages), (0, 2));
}

#[test]
fn tool_pairs_widen_a_duplicated_call_id_to_its_extremes() {
    let messages = [
        text("a"),
        tool_call("b", "call1", "read"),
        tool_result("c", "call1", "read"),
        text("d"),
        tool_result("e", "call1", "read"),
    ];
    assert_eq!(adjust_boundaries_for_tool_pairs(0, 1, &messages), (0, 4));
}

#[test]
fn tool_pairs_pull_the_mate_in_regardless_of_distance() {
    let mut messages = vec![tool_call("a", "call1", "read")];
    messages.extend((0..25).map(|index| text(&format!("gap{index}"))));
    messages.push(tool_result("result", "call1", "read"));
    assert_eq!(adjust_boundaries_for_tool_pairs(0, 0, &messages), (0, 26));
}

#[test]
fn tool_pairs_handle_several_pairs_in_range() {
    let messages = [
        text("a"),
        tool_call("b", "call1", "read"),
        tool_result("c", "call1", "read"),
        tool_call("d", "call2", "read"),
        text("e"),
        tool_result("f", "call2", "read"),
    ];
    assert_eq!(adjust_boundaries_for_tool_pairs(1, 4, &messages), (1, 5));
}

// --- compression with tool-pair protection ---

fn straddled_pair() -> Vec<Message> {
    vec![
        text("m1"),
        tool_call("m2", "c1", "read"),
        text("m3"),
        tool_result("m4", "c1", "read"),
        text("m5"),
    ]
}

#[test]
fn compression_pulls_in_an_orphaned_tool_result() {
    let messages = straddled_pair();
    let state = state_for(&messages);
    let summary =
        "Compressed range with tool-call but not its result — extension should pull the result in.";
    let outcome = apply(
        &[range("m00001", "m00003", summary)],
        &messages,
        &state,
        &default_config(),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = active_block(&outcome.state);
    assert!(
        covers(block, "m4"),
        "tool-result m4 should be auto-included"
    );
    assert!(covers(block, "m2"), "tool-call m2 should be in block");
}

#[test]
fn compression_pulls_in_an_orphaned_tool_call() {
    let messages = straddled_pair();
    let state = state_for(&messages);
    let summary =
        "Compressed range with tool-result but not its call — extension pulls the call in.";
    let outcome = apply(
        &[range("m00003", "m00004", summary)],
        &messages,
        &state,
        &default_config(),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = active_block(&outcome.state);
    assert!(covers(block, "m2"), "tool-call m2 should be auto-included");
    assert!(covers(block, "m4"), "tool-result m4 should be in block");
}

#[test]
fn block_boundary_ranges_skip_tool_pair_adjustment() {
    let messages = [
        text("m1"),
        tool_call("m2", "c1", "read"),
        tool_result("m3", "c1", "read"),
        text("m4"),
        tool_call("m5", "c2", "read"),
        tool_result("m6", "c2", "read"),
        text("m7"),
    ];
    let config = default_config();
    let state = state_for(&messages);
    let first = apply(
        &[with_topic(
            range(
                "m00001",
                "m00004",
                "First tier-1 block covering initial text and a complete tool pair for reading configuration.",
            ),
            "Phase 1",
        )],
        &messages,
        &state,
        &config,
    );
    assert_eq!(first.result.blocks_created, 1);
    assert_eq!(first.state.block("b1").expect("b1").tier, 1);

    let second = apply(
        &[with_topic(
            range(
                "b1",
                "b1",
                "Tier-2 distillation of the first phase, condensing the initial setup and configuration reads.",
            ),
            "Distilled Phase 1",
        )],
        &messages,
        &first.state,
        &config,
    );
    assert_eq!(
        second.result.blocks_created, 1,
        "errors: {:?}",
        second.result.errors
    );
    let b2 = second.state.block("b2").expect("tier-2 block b2");
    assert_eq!(b2.tier, 2);
    assert!(
        !second.state.block("b1").expect("b1").active,
        "b1 consumed by b2"
    );
    assert_eq!(b2.direct_block_ids, ["b1"]);
}

#[test]
fn tool_pair_extension_rescans_nested_blocks_in_the_adjusted_range() {
    let messages = [
        tool_call("m1", "c1", "read"),
        tool_result("m2", "c1", "read"),
        text("m3"),
        tool_call("m4", "c2", "read"),
        tool_result("m5", "c2", "read"),
        text("m6"),
    ];
    let config = default_config();
    let state = state_for(&messages);
    let first = apply(
        &[range(
            "m00001",
            "m00002",
            "Block covering the first complete tool pair for initial file read operations.",
        )],
        &messages,
        &state,
        &config,
    );
    assert_eq!(first.result.blocks_created, 1);
    let second = apply(
        &[range(
            "m00004",
            "m00005",
            "Second complete tool pair covering the second file read and its result output.",
        )],
        &messages,
        &first.state,
        &config,
    );
    assert_eq!(second.result.blocks_created, 1);
    let b2 = second.state.block("b2").expect("b2");
    assert!(
        !b2.direct_block_ids.contains(&"b1".to_owned()),
        "b1 is not consumed"
    );
}

// --- prune stripOrphanedToolCalls ---

#[test]
fn a_complete_pair_survives_when_another_pair_folds() {
    let messages = [
        text("m1"),
        tool_call("m2", "c1", "read"),
        tool_result("m3", "c1", "read"),
        tool_call("m4", "c2", "read"),
        tool_result("m5", "c2", "read"),
        text("m6"),
    ];
    let state = state_for(&messages);
    let outcome = apply(
        &[range(
            "m00004",
            "m00005",
            "Second tool pair compressed. First pair should survive intact.",
        )],
        &messages,
        &state,
        &default_config(),
    );
    let ids = view_ids(&messages, &outcome.state);
    assert!(has(&ids, "m2") && has(&ids, "m3"));
    assert!(!has(&ids, "m4") && !has(&ids, "m5"));
}

#[test]
fn an_orphaned_result_is_stripped_when_its_call_sits_in_a_split_block() {
    let mut messages = vec![tool_call("m1", "c1", "read")];
    messages.extend((0..25).map(|index| text(&format!("gap{index}"))));
    messages.push(tool_result("result", "c1", "read"));
    let mut state = state_for(&messages);
    let call_ref = state.message_refs.by_raw["m1"].clone();
    state.blocks.push(Block {
        block_id: "b1".to_owned(),
        run_id: "run1".to_owned(),
        summary: "Legacy split block covering only the call (pre-#684 state).".to_owned(),
        direct_message_ids: vec!["m1".to_owned()],
        effective_message_ids: vec!["m1".to_owned()],
        compressed_tokens: 10,
        start_ref: Some(call_ref.clone()),
        end_ref: Some(call_ref),
        ..Block::default()
    });
    let ids = view_ids(&messages, &state);
    assert!(!has(&ids, "m1"), "blocked tool-call m1 hidden");
    assert!(!has(&ids, "result"), "orphaned tool-result stripped");
}

// --- tool-pair integrity ---

fn padded_user(id: &str) -> Message {
    Message::text(id, Role::User, format!("u-{id} {}", "x".repeat(200)))
}

fn padded_result(id: &str, call_id: &str, tool: &str) -> Message {
    Message {
        role: Role::User,
        ..Message::tool_result(
            id,
            call_id,
            tool,
            format!("result-{call_id} {}", "r".repeat(200)),
        )
    }
}

fn padded_reasoning(id: &str) -> Message {
    Message::reasoning(id, format!("think-{id} {}", "t".repeat(200)))
}

fn padded_assistant(id: &str) -> Message {
    Message::text(id, Role::Assistant, format!("say-{id} {}", "s".repeat(200)))
}

fn integrity_config() -> Config {
    let mut config = Config::new(150_000);
    config.compress.min_compress_range = 1;
    config.compress.min_summary_length = 1;
    config
}

/// Every tool call answered by results before any other message follows,
/// as a strict upstream validates; returns the unanswered call ids.
fn unanswered_tool_calls(view: &[Message]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut calls: Vec<String> = Vec::new();
    let mut answers: Vec<String> = Vec::new();
    for message in view {
        match (message.kind, &message.tool_call_id) {
            (Kind::Reasoning, _) => {}
            (Kind::ToolCall, Some(call_id)) => calls.push(call_id.clone()),
            (Kind::ToolResult, Some(call_id)) => answers.push(call_id.clone()),
            _ => {
                violations.extend(calls.drain(..).filter(|call| !answers.contains(call)));
                answers.clear();
            }
        }
    }
    violations.extend(calls.drain(..).filter(|call| !answers.contains(call)));
    violations
}

/// A summary between two assistant messages splits one wire message.
fn splits_assistant_run(view: &[Message]) -> bool {
    view.iter().enumerate().any(|(at, message)| {
        message.id.starts_with("acp_summary_")
            && at > 0
            && view[at - 1].role == Role::Assistant
            && view
                .get(at + 1)
                .is_some_and(|next| next.role == Role::Assistant)
    })
}

fn apply_with_zone(
    messages: &[Message],
    state: &State,
    zone: &HashSet<String>,
    start: &str,
    end: &str,
) -> ApplyOutcome {
    let ranges = [range(start, end, &"S".repeat(400))];
    apply_compression(&ApplyInput {
        ranges: &ranges,
        messages,
        state,
        config: &integrity_config(),
        protected_refs: Some(zone),
        now_ms: 0,
    })
}

fn zone_of(state: &State, ids: &[&str]) -> HashSet<String> {
    ids.iter()
        .map(|id| state.message_refs.by_raw[*id].clone())
        .collect()
}

fn position(ids: &[&str], id: &str) -> Option<usize> {
    ids.iter().position(|known| *known == id)
}

#[test]
fn the_summary_stays_out_of_a_parallel_burst() {
    let messages = [
        padded_user("m1"),
        tool_call("m2", "c-grep", "grep"),
        tool_call("m3", "c-read", "read"),
        padded_result("m4", "c-read", "read"),
        padded_result("m5", "c-grep", "grep"),
        padded_user("m6"),
    ];
    let state = state_for(&messages);
    let outcome = apply_with_zone(&messages, &state, &HashSet::new(), "m00003", "m00004");
    assert_eq!(
        outcome.result.blocks_created, 1,
        "errors={:?}",
        outcome.result.errors
    );
    let block = active_block(&outcome.state);
    let mut covered = block.effective_message_ids.clone();
    covered.sort();
    assert_eq!(covered, ["m3", "m4"], "the read pair folds");

    let view = prune(&messages, &outcome.state);
    let ids: Vec<&str> = view.iter().map(|message| message.id.as_str()).collect();
    let summary = position(&ids, &summary_message_id(&block.block_id));
    let grep_call = position(&ids, "m2").expect("grep call kept");
    let grep_result = position(&ids, "m5").expect("grep result kept");
    assert!(
        !summary.is_some_and(|at| at > grep_call && at < grep_result),
        "{ids:?}"
    );
    assert!(!splits_assistant_run(&view), "{ids:?}");
    assert!(unanswered_tool_calls(&view).is_empty());
}

#[test]
fn a_range_whose_only_foldable_half_is_the_call_is_refused() {
    let messages = [
        padded_user("m1"),
        tool_call("m2", "c-a", "read"),
        padded_result("m3", "c-a", "read"),
        padded_user("m4"),
        padded_user("m5"),
    ];
    let state = state_for(&messages);
    let zone = zone_of(&state, &["m3"]);
    let outcome = apply_with_zone(&messages, &state, &zone, "m00002", "m00003");
    assert_eq!(outcome.result.blocks_created, 0);
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(
        outcome.result.errors[0].contains("split 1 tool call/result pair"),
        "{:?}",
        outcome.result.errors
    );
}

#[test]
fn the_call_is_withdrawn_with_a_warning_when_the_range_carries_more() {
    let messages = [
        padded_user("m1"),
        tool_call("m2", "c-a", "read"),
        padded_result("m3", "c-a", "read"),
        padded_user("m4"),
        padded_user("m5"),
        padded_user("m6"),
    ];
    let state = state_for(&messages);
    let zone = zone_of(&state, &["m3"]);
    let outcome = apply_with_zone(&messages, &state, &zone, "m00002", "m00004");
    assert_eq!(
        outcome.result.blocks_created, 1,
        "errors={:?}",
        outcome.result.errors
    );
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|warning| warning.contains("tool call/result pair")),
        "{:?}",
        outcome.result.warnings
    );
    assert!(
        !covers(active_block(&outcome.state), "m2"),
        "the unanswerable call must not fold"
    );
    let view = view_ids(&messages, &outcome.state);
    assert!(has(&view, "m2") && has(&view, "m3"), "{view:?}");
    assert!(unanswered_tool_calls(&prune(&messages, &outcome.state)).is_empty());
}

#[test]
fn the_summary_stays_out_of_an_assistant_turns_reasoning_run() {
    let messages = [
        padded_user("m1"),
        padded_reasoning("m2"),
        padded_assistant("m3"),
        tool_call("m4", "c-r1", "read"),
        tool_call("m5", "c-g", "grep"),
        tool_call("m6", "c-r2", "read"),
        padded_result("m7", "c-r1", "read"),
        padded_result("m8", "c-r2", "read"),
        padded_result("m9", "c-g", "grep"),
        padded_user("m10"),
    ];
    let state = state_for(&messages);
    let zone = zone_of(&state, &["m5", "m9"]);
    let outcome = apply_with_zone(&messages, &state, &zone, "m00004", "m00009");
    assert_eq!(
        outcome.result.blocks_created, 1,
        "errors={:?}",
        outcome.result.errors
    );
    let block = active_block(&outcome.state);
    let mut covered = block.effective_message_ids.clone();
    covered.sort();
    assert_eq!(
        covered,
        ["m4", "m6", "m7", "m8"],
        "the read pairs fold, the protected grep pair stays"
    );

    let view = prune(&messages, &outcome.state);
    let ids: Vec<&str> = view.iter().map(|message| message.id.as_str()).collect();
    let summary = position(&ids, &summary_message_id(&block.block_id)).expect("summary rendered");
    assert!(summary < position(&ids, "m2").expect("m2"), "{ids:?}");
    assert!(!splits_assistant_run(&view), "{ids:?}");
    assert!(unanswered_tool_calls(&view).is_empty());
}

// --- adjustBoundariesForReasoningPairs ---

#[test]
fn reasoning_pairs_no_op_without_reasoning() {
    let messages = [text("a"), text("b"), text("c"), text("d")];
    assert_eq!(
        adjust_boundaries_for_reasoning_pairs(0, 2, &messages),
        (0, 2)
    );
}

#[test]
fn reasoning_pairs_extend_forward_to_the_companion() {
    let messages = [text("a"), reasoning("b"), assistant_text("c"), text("d")];
    assert_eq!(
        adjust_boundaries_for_reasoning_pairs(1, 1, &messages),
        (1, 2)
    );
}

#[test]
fn reasoning_pairs_extend_backward_to_the_reasoning() {
    let messages = [text("a"), reasoning("b"), assistant_text("c"), text("d")];
    assert_eq!(
        adjust_boundaries_for_reasoning_pairs(2, 2, &messages),
        (1, 2)
    );
}

#[test]
fn reasoning_pairs_extend_both_directions() {
    let messages = [
        reasoning("a"),
        assistant_text("b"),
        text("c"),
        reasoning("d"),
        assistant_text("e"),
    ];
    assert_eq!(
        adjust_boundaries_for_reasoning_pairs(1, 3, &messages),
        (0, 4)
    );
}

#[test]
fn reasoning_pairs_take_a_tool_call_as_the_companion() {
    let messages = [
        text("a"),
        reasoning("b"),
        tool_call("c", "call1", "read"),
        tool_result("d", "call1", "read"),
        text("e"),
    ];
    assert_eq!(
        adjust_boundaries_for_reasoning_pairs(1, 1, &messages),
        (1, 2)
    );
}

#[test]
fn reasoning_pairs_do_not_extend_without_a_companion() {
    let messages = [text("a"), reasoning("b"), text("c")];
    assert_eq!(
        adjust_boundaries_for_reasoning_pairs(1, 1, &messages),
        (1, 1)
    );
}

#[test]
fn reasoning_pairs_treat_a_reasoning_run_as_one_unit() {
    let messages = [
        text("a"),
        reasoning("r1"),
        reasoning("r2"),
        assistant_text("c"),
        text("d"),
    ];
    assert_eq!(
        adjust_boundaries_for_reasoning_pairs(1, 1, &messages),
        (1, 3)
    );
}

// --- compression with reasoning-pair protection ---

fn reasoning_turn() -> Vec<Message> {
    vec![
        text("m1"),
        reasoning("m2"),
        assistant_text("m3"),
        text("m4"),
    ]
}

fn fold(messages: &[Message], start: &str, end: &str, summary: &str) -> ApplyOutcome {
    let state = state_for(messages);
    apply(
        &[range(start, end, summary)],
        messages,
        &state,
        &default_config(),
    )
}

#[test]
fn compression_pulls_in_the_companion_after_reasoning() {
    let outcome = fold(
        &reasoning_turn(),
        "m00002",
        "m00002",
        "Compress just the reasoning; its companion text must be auto-included to keep the pair atomic.",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = active_block(&outcome.state);
    assert!(covers(block, "m2") && covers(block, "m3"));
}

#[test]
fn compression_pulls_in_the_reasoning_before_its_companion() {
    let outcome = fold(
        &reasoning_turn(),
        "m00003",
        "m00003",
        "Compress just the companion text; its preceding reasoning must be auto-included.",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = active_block(&outcome.state);
    assert!(covers(block, "m2") && covers(block, "m3"));
}

#[test]
fn reasoning_and_tool_call_also_pull_in_the_result() {
    let messages = [
        text("m1"),
        reasoning("m2"),
        tool_call("m3", "c1", "read"),
        tool_result("m4", "c1", "read"),
        text("m5"),
    ];
    let outcome = fold(
        &messages,
        "m00002",
        "m00002",
        "Compress just the reasoning; fixpoint pulls in the tool-call then its result.",
    );
    assert_eq!(outcome.result.blocks_created, 1);
    let block = active_block(&outcome.state);
    assert!(covers(block, "m2") && covers(block, "m3") && covers(block, "m4"));
}

#[test]
fn an_unfolded_reasoning_pair_survives_another_fold() {
    let messages = [
        text("m1"),
        reasoning("m2"),
        assistant_text("m3"),
        text("m4"),
        reasoning("m5"),
        assistant_text("m6"),
    ];
    let outcome = fold(
        &messages,
        "m00005",
        "m00006",
        "Compress the second reasoning pair. The first pair must survive intact.",
    );
    let ids = view_ids(&messages, &outcome.state);
    assert!(has(&ids, "m2") && has(&ids, "m3"));
    assert!(!has(&ids, "m5") && !has(&ids, "m6"));
}

// --- prune stripOrphanedReasoning ---

#[test]
fn prune_strips_a_reasoning_run_whose_companion_folded() {
    let messages = [
        Message::text("u0", Role::User, "ask"),
        Message::reasoning("r1", "thinking"),
        Message::text("a1", Role::Assistant, "answer"),
        Message::text("u1", Role::User, "next"),
    ];
    let state = State {
        blocks: vec![Block {
            block_id: "b0".to_owned(),
            run_id: "r0".to_owned(),
            summary: "covered the answer".to_owned(),
            direct_message_ids: vec!["a1".to_owned()],
            effective_message_ids: vec!["a1".to_owned()],
            compressed_tokens: 10,
            ..Block::default()
        }],
        ..State::default()
    };
    let ids = view_ids(&messages, &state);
    assert!(!has(&ids, "r1"), "dangling reasoning stripped");
    assert!(!has(&ids, "a1"), "covered companion pruned");
    assert!(has(&ids, "u0") && has(&ids, "u1"));
}

#[test]
fn prune_keeps_a_reasoning_pair_neither_half_of_which_is_covered() {
    let messages = [
        Message::text("u0", Role::User, "ask"),
        Message::reasoning("r1", "thinking"),
        Message::text("a1", Role::Assistant, "answer"),
    ];
    let ids = view_ids(&messages, &State::default());
    assert!(has(&ids, "r1") && has(&ids, "a1"));
}

// --- regression: a split compress does not orphan a reasoning pair (#133) ---

#[test]
fn folding_only_the_reasoning_folds_its_companion_too() {
    let messages = reasoning_turn();
    let outcome = fold(
        &messages,
        "m00002",
        "m00002",
        "Compress just the reasoning; its companion text must compress with it.",
    );
    let ids = view_ids(&messages, &outcome.state);
    assert!(!has(&ids, "m2") && !has(&ids, "m3"), "{ids:?}");
}

#[test]
fn folding_only_the_companion_folds_its_reasoning_too() {
    let messages = reasoning_turn();
    let outcome = fold(
        &messages,
        "m00003",
        "m00003",
        "Compress just the text; its preceding reasoning must compress with it.",
    );
    let ids = view_ids(&messages, &outcome.state);
    assert!(!has(&ids, "m2") && !has(&ids, "m3"), "{ids:?}");
}
