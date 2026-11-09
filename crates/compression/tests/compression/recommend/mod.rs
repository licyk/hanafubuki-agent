// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `recommend.test.ts` and `recommend-merge.test.ts`; the children port
// the rest of the recommend group.

mod advertise;
mod scale;
mod screen;

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    assign_refs, build_compressible_ranges, compute_protected_refs, default_count_tokens,
    merge_ranges_to_threshold, process_turn, Block, CompressibleRange, Config, ContextRanges,
    Generation, Message, RenderStrategy, Role, State, TurnInput, TurnOutput, BLOCKED_REF,
};

use crate::fixtures::{assign, config, result, state_for};

/// The recommend tests' configuration: every gate off, the kernel tests'
/// nudge settings.
pub(crate) fn recommend_config() -> Config {
    let mut config = config();
    config.tiers.tier2_trigger = 5;
    config.tiers.tier3_trigger = 10;
    let nudge = &mut config.nudge;
    nudge.max_context_limit_pct = 0.55;
    nudge.min_context_limit_pct = 0.45;
    nudge.growth_ratio = 0.05;
    nudge.growth_floor = 6000;
    nudge.growth_cap = 50_000;
    nudge.min_growth_floor = 5000;
    nudge.min_growth_ratio = 0.45;
    nudge.emergency_threshold_pct = 0.98;
    config
}

pub(crate) fn text(id: &str, body: &str, role: Role) -> Message {
    Message::text(id, role, body)
}

pub(crate) fn user(id: &str, body: &str) -> Message {
    text(id, body, Role::User)
}

/// A tool call with no tool-call id, as the kernel's recommend tests build
/// them.
pub(crate) fn bare_call(id: &str, tool: &str) -> Message {
    Message {
        tool_call_id: None,
        ..Message::tool_call(id, "", tool, format!("call {tool}"))
    }
}

/// A result with only its call id.
pub(crate) fn unnamed_result(id: &str, call_id: &str, output: &str) -> Message {
    Message {
        tool_name: None,
        ..result(id, call_id, "", output)
    }
}

/// Refs for `messages` from `m00001` (existing refs kept).
pub(crate) fn assign_all(messages: &[Message], state: &mut State) {
    assign(messages, state);
}

pub(crate) fn ranges_of(messages: &[Message], state: &State, config: &Config) -> ContextRanges {
    build_compressible_ranges(messages, state, config, None, messages)
}

pub(crate) fn spans(ranges: &[CompressibleRange]) -> Vec<(String, String, usize)> {
    ranges
        .iter()
        .map(|range| (range.start_ref.clone(), range.end_ref.clone(), range.count))
        .collect()
}

pub(crate) fn span(start: &str, end: &str, count: usize) -> (String, String, usize) {
    (start.to_owned(), end.to_owned(), count)
}

pub(crate) fn block(id: u32, effective: &[&str]) -> Block {
    let ids: Vec<String> = effective.iter().map(|id| (*id).to_owned()).collect();
    Block {
        block_id: format!("b{id}"),
        run_id: format!("r{id}"),
        tier: 1,
        summary: format!("Summary {id}"),
        direct_message_ids: ids.clone(),
        effective_message_ids: ids,
        compressed_tokens: 100,
        created_at: 0,
        survived_count: 1,
        generation: Generation::Old,
        active: true,
        ..Block::default()
    }
}

pub(crate) fn turn(messages: &[Message], state: State, config: &Config, tokens: u64) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config,
        token_count: tokens,
        token_count_source: None,
        render: RenderStrategy::TextAndToolResults,
    })
}

pub(crate) fn ref_of<'a>(state: &'a State, id: &str) -> &'a str {
    &state.message_refs.by_raw[id]
}

// --- computeProtectedRefs ---

fn zone(messages: &[Message], preserve_messages: usize, preserve_tokens: u64) -> HashSet<String> {
    let mut config = recommend_config();
    config.preserve_recent_messages = preserve_messages;
    config.preserve_recent_tokens = preserve_tokens;
    compute_protected_refs(messages, &state_for(messages), &config)
}

#[test]
fn the_last_user_message_is_protected_with_recent_protection_on() {
    let messages = [user("a", "x"), text("b", "y", Role::Assistant)];
    assert!(zone(&messages, 5, 0).contains("m00001"));
}

#[test]
fn the_last_user_message_is_not_protected_with_recent_protection_off() {
    let messages = [user("a", "x"), text("b", "y", Role::Assistant)];
    assert!(!zone(&messages, 0, 0).contains("m00001"));
}

#[test]
fn the_zone_keeps_the_last_messages_by_count() {
    let messages = [user("a", "x"), user("b", "y"), user("c", "z")];
    let refs = zone(&messages, 2, 0);
    assert!(refs.contains("m00002") && refs.contains("m00003"));
    assert!(!refs.contains("m00001"));
}

#[test]
fn the_zone_keeps_the_last_tokens_expanding_backward() {
    let messages = [
        user("a", &"x".repeat(1200)),
        user("b", &"y".repeat(1200)),
        user("c", &"z".repeat(1200)),
    ];
    let refs = zone(&messages, 0, 500);
    assert!(refs.contains("m00003") && refs.contains("m00002"));
    assert!(
        !refs.contains("m00001"),
        "a is outside the 500-token window"
    );
}

#[test]
fn the_token_zone_counts_cjk_characters_one_token_each() {
    // 100 CJK characters are 100 tokens, so a 150-token zone holds two.
    let messages = [
        user("a", &"文".repeat(100)),
        user("b", &"文".repeat(100)),
        user("c", &"文".repeat(100)),
    ];
    let refs = zone(&messages, 0, 150);
    assert!(refs.contains("m00003") && refs.contains("m00002"));
    assert!(!refs.contains("m00001"));
}

#[test]
fn the_count_and_token_rules_combine() {
    let messages = [
        user("a", &"x".repeat(1200)),
        user("b", &"y".repeat(1200)),
        user("c", &"z".repeat(1200)),
    ];
    let refs = zone(&messages, 1, 500);
    assert!(refs.contains("m00003") && refs.contains("m00002"));
    assert!(!refs.contains("m00001"));
}

#[test]
fn ascii_text_sizes_the_token_zone_at_four_characters_a_token() {
    let messages = [
        user("a", &"x".repeat(200)),
        user("b", &"x".repeat(200)),
        user("c", "y"),
    ];
    let refs = zone(&messages, 0, 100);
    assert!(
        refs.contains("m00001"),
        "a (50 tokens) fits the 100-token zone"
    );
    assert!(refs.contains("m00003"));
}

// --- buildCompressibleRanges ---

#[test]
fn contiguous_compressible_messages_form_one_range() {
    let messages = [
        user("a", &"x".repeat(2000)),
        user("b", &"y".repeat(2000)),
        user("c", &"z".repeat(2000)),
    ];
    let ranges = ranges_of(&messages, &state_for(&messages), &recommend_config());
    assert_eq!(spans(&ranges.compressible), [span("m00001", "m00003", 3)]);
    assert!(ranges.protected.is_empty());
}

#[test]
fn a_blocked_protected_tool_is_skipped_and_its_neighbours_join() {
    let messages = [
        user("a", &"x".repeat(2000)),
        bare_call("p", "skill"),
        user("c", &"z".repeat(2000)),
    ];
    let mut state = State::default();
    state.message_refs = assign_refs(&messages, &state.message_refs, 1, |message| {
        message.tool_name.as_deref() == Some("skill")
    })
    .map;
    assert_eq!(state.message_refs.by_raw["p"], BLOCKED_REF);
    let mut config = recommend_config();
    config.protected_tools = vec!["skill".to_owned()];
    let ranges = ranges_of(&messages, &state, &config);
    assert_eq!(spans(&ranges.compressible), [span("m00001", "m00002", 2)]);
}

#[test]
fn the_protected_zone_splits_compressible_groups() {
    let messages = [
        user("a", &"x".repeat(2000)),
        user("b", &"y".repeat(2000)),
        user("c", &"z".repeat(2000)),
        user("d", &"w".repeat(2000)),
    ];
    let state = state_for(&messages);
    let zone: HashSet<String> = ["m00003".to_owned(), "m00004".to_owned()].into();
    let ranges = build_compressible_ranges(
        &messages,
        &state,
        &recommend_config(),
        Some(&zone),
        &messages,
    );
    assert_eq!(spans(&ranges.compressible), [span("m00001", "m00002", 2)]);
}

#[test]
fn tool_and_text_shares_add_up_to_a_hundred() {
    let messages = [
        user("a", "text content here"),
        bare_call("t", "bash"),
        user("c", "more text content"),
    ];
    let ranges = ranges_of(&messages, &state_for(&messages), &recommend_config());
    let range = &ranges.compressible[0];
    assert!(range.tool_pct > 0 && range.text_pct > 0);
    assert_eq!(range.tool_pct + range.text_pct, 100);
}

#[test]
fn groups_split_at_a_user_message_once_three_long() {
    let mut messages = Vec::new();
    for turn in 0..4 {
        messages.push(user(
            &format!("u{turn}"),
            &format!("user turn {turn}{}", " content".repeat(50)),
        ));
        messages.push(bare_call(&format!("a{turn}"), "bash"));
        messages.push(Message {
            role: Role::Tool,
            ..unnamed_result(
                &format!("t{turn}"),
                &format!("c{turn}"),
                &"result".repeat(50),
            )
        });
    }
    let ranges = ranges_of(&messages, &state_for(&messages), &recommend_config());
    assert_eq!(ranges.compressible.len(), 4, "one block per user turn");
    assert_eq!(spans(&ranges.compressible)[0], span("m00001", "m00003", 3));
    assert_eq!(
        (
            ranges.compressible[3].start_ref.as_str(),
            ranges.compressible[3].end_ref.as_str()
        ),
        ("m00010", "m00012")
    );
}

#[test]
fn a_group_shorter_than_three_is_not_split_at_a_user_message() {
    let messages = [
        bare_call("a", "bash"),
        unnamed_result("t", "c", &"result".repeat(50)),
        user("u", "user message"),
    ];
    let ranges = ranges_of(&messages, &state_for(&messages), &recommend_config());
    assert_eq!(ranges.compressible.len(), 1);
    assert_eq!(ranges.compressible[0].count, 3);
}

#[test]
fn ref_map_holes_do_not_fragment_ranges() {
    let a = text("a", &"x".repeat(2000), Role::User);
    let e = text("e", &"v".repeat(2000), Role::User);
    let state = state_for(&[
        a.clone(),
        user("b", "y"),
        user("c", "z"),
        user("d", "w"),
        e.clone(),
    ]);
    let ranges = ranges_of(&[a, e], &state, &recommend_config());
    assert_eq!(spans(&ranges.compressible), [span("m00001", "m00005", 2)]);
}

#[test]
fn a_mid_array_summary_node_extends_the_range() {
    let a = text("a", &"x".repeat(2000), Role::Assistant);
    let d = text("d", &"w".repeat(2000), Role::Assistant);
    let e = text("e", &"v".repeat(2000), Role::Assistant);
    let summary = text(
        "s",
        "Summary of the compressed span: did the work.",
        Role::Assistant,
    );
    let mut state = state_for(&[
        a.clone(),
        user("b", "y"),
        user("c", "z"),
        d.clone(),
        e.clone(),
    ]);
    let view = [a, summary, d, e];
    assign_all(&view, &mut state);
    assert_eq!(state.message_refs.by_raw["s"], "m00006");
    let ranges = ranges_of(&view, &state, &recommend_config());
    assert_eq!(spans(&ranges.compressible), [span("m00001", "m00005", 4)]);
}

#[test]
fn a_synthetic_summary_node_still_splits_ranges() {
    let a = user("a", &"x".repeat(2000));
    let e = user("e", &"v".repeat(2000));
    let synthetic = text(
        "s",
        "[Compressed conversation section] earlier work summarized.",
        Role::Assistant,
    );
    let mut state = state_for(&[
        a.clone(),
        user("b", "y"),
        user("c", "z"),
        user("d", "w"),
        e.clone(),
    ]);
    let view = [a, synthetic, e];
    assign_all(&view, &mut state);
    let ranges = ranges_of(&view, &state, &recommend_config());
    assert_eq!(
        spans(&ranges.compressible),
        [span("m00001", "m00001", 1), span("m00005", "m00005", 1)]
    );
}

#[test]
fn protected_groups_segment_by_array_adjacency() {
    let mut config = recommend_config();
    config.protected_tools = vec!["skill".to_owned()];
    let (p1, p2) = (bare_call("p1", "skill"), bare_call("p2", "skill"));

    let dense_view = [p1.clone(), user("x", "y"), p2.clone()];
    let mut dense = State::default();
    for (id, r#ref) in [("p1", "m00001"), ("x", "m00002"), ("p2", "m00003")] {
        dense.message_refs.by_raw.insert(id.into(), r#ref.into());
        dense.message_refs.by_ref.insert(r#ref.into(), id.into());
    }
    let dense_ranges = ranges_of(&dense_view, &dense, &config);
    assert_eq!(
        dense_ranges.protected.len(),
        2,
        "a message between splits them"
    );

    let mut holed = State::default();
    for (id, r#ref) in [("p1", "m00001"), ("p2", "m00005")] {
        holed.message_refs.by_raw.insert(id.into(), r#ref.into());
        holed.message_refs.by_ref.insert(r#ref.into(), id.into());
    }
    let holed_ranges = ranges_of(&[p1, p2], &holed, &config);
    assert_eq!(
        holed_ranges.protected.len(),
        1,
        "a ref-map hole does not split"
    );
    let protected = &holed_ranges.protected[0];
    assert_eq!(protected.count, 2);
    assert_eq!(
        (protected.start_ref.as_str(), protected.end_ref.as_str()),
        ("m00001", "m00005")
    );
}

#[test]
fn tiny_ranges_are_not_nudged_on_the_first_turn() {
    let mut config = recommend_config();
    config.model_context_limit = 12_000;
    let messages = [user("a", "hello"), user("b", "world")];
    let out = turn(&messages, State::default(), &config, 5000);
    assert!(!out.nudge.should_inject, "turn 1: growth 0, no nudge");
    assert!(out.nudge.reason.contains("growth"), "{}", out.nudge.reason);
}

#[test]
fn cjk_text_counts_as_more_pending_than_four_characters_a_token() {
    let chinese = "这是一个中文测试消息用于验证token校准的效果。".repeat(20);
    let messages = [
        user("a", &chinese),
        bare_call("b", "bash"),
        text("c", "tail", Role::Tool),
    ];
    let ranges = ranges_of(&messages, &state_for(&messages), &recommend_config());
    let pending: u64 = ranges.compressible.iter().map(|range| range.tokens).sum();
    let quarter: u64 = messages
        .iter()
        .map(|message| (message.text.chars().count() as u64).div_ceil(4))
        .sum();
    assert!(pending >= quarter * 3, "{pending} vs {quarter}");
    assert_eq!(
        ranges.compressible[0].tokens,
        messages
            .iter()
            .map(|m| default_count_tokens(&m.text))
            .sum::<u64>()
    );
}

#[test]
fn ascii_range_tokens_are_four_characters_a_token() {
    let messages = [user("a", &"x".repeat(40))];
    let ranges = ranges_of(&messages, &state_for(&messages), &recommend_config());
    assert_eq!(ranges.compressible[0].tokens, 10);
}

// --- mergeRangesToThreshold ---

fn hand_range(start: &str, end: &str, tokens: u64, count: usize) -> CompressibleRange {
    CompressibleRange {
        start_ref: start.to_owned(),
        end_ref: end.to_owned(),
        count,
        tokens,
        text_pct: 100,
        ..CompressibleRange::default()
    }
}

fn with_pct(mut range: CompressibleRange, tool_pct: u32) -> CompressibleRange {
    range.tool_pct = tool_pct;
    range.text_pct = 100 - tool_pct;
    range
}

#[test]
fn merging_nothing_gives_nothing() {
    assert!(merge_ranges_to_threshold(Vec::new(), 5000).is_empty());
}

#[test]
fn a_zero_threshold_leaves_the_ranges_unchanged() {
    let ranges = vec![
        hand_range("m00001", "m00002", 100, 1),
        hand_range("m00003", "m00004", 200, 1),
    ];
    assert_eq!(merge_ranges_to_threshold(ranges.clone(), 0), ranges);
}

#[test]
fn a_range_already_over_the_threshold_is_one_batch() {
    let ranges = vec![with_pct(hand_range("m00001", "m00010", 1250, 4), 50)];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].tokens, out[0].count), (1250, 4));
    assert_eq!(
        (out[0].start_ref.as_str(), out[0].end_ref.as_str()),
        ("m00001", "m00010")
    );
}

#[test]
fn two_small_ranges_whose_sum_clears_the_threshold_merge() {
    let ranges = vec![
        with_pct(hand_range("m00001", "m00005", 600, 2), 100),
        with_pct(hand_range("m00010", "m00020", 700, 3), 0),
    ];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].count, out[0].tokens), (5, 1300));
    assert_eq!(
        (out[0].start_ref.as_str(), out[0].end_ref.as_str()),
        ("m00001", "m00020")
    );
}

#[test]
fn a_short_tail_folds_into_the_batch_before_it() {
    let ranges = vec![
        hand_range("m00001", "m00003", 600, 1),
        hand_range("m00004", "m00006", 700, 1),
        hand_range("m00010", "m00011", 50, 1),
    ];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].tokens, out[0].count), (1350, 3));
    assert_eq!(
        (out[0].start_ref.as_str(), out[0].end_ref.as_str()),
        ("m00001", "m00011")
    );
    assert!(out[0].chars.unwrap_or(out[0].tokens as usize * 4) >= 5000);
}

#[test]
fn the_tool_share_is_weighted_by_count() {
    let ranges = vec![
        with_pct(hand_range("m00001", "m00002", 600, 2), 100),
        with_pct(hand_range("m00003", "m00004", 700, 3), 0),
    ];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!((out[0].tool_pct, out[0].text_pct), (40, 60));
}

#[test]
fn ranges_merge_across_ref_gaps() {
    let ranges = vec![
        hand_range("m00001", "m00002", 600, 1),
        hand_range("m00050", "m00060", 700, 1),
    ];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!(out.len(), 1);
    assert_eq!(
        (out[0].start_ref.as_str(), out[0].end_ref.as_str()),
        ("m00001", "m00060")
    );
}

#[test]
fn content_wholly_under_the_threshold_is_not_offered() {
    let ranges = vec![
        hand_range("m00001", "m00002", 100, 1),
        hand_range("m00003", "m00004", 100, 1),
        hand_range("m00005", "m00006", 100, 1),
    ];
    assert!(merge_ranges_to_threshold(ranges, 5000).is_empty());
}

#[test]
fn every_emitted_batch_clears_the_threshold_alone() {
    let cjk = |n: u64| CompressibleRange {
        chars: Some(n as usize),
        ..hand_range(
            "m00001",
            "m00005",
            n,
            ((n as f64 / 50.0).round() as usize).max(1),
        )
    };
    for ranges in [
        vec![cjk(600), cjk(700), cjk(50)],
        vec![cjk(3000), cjk(3000)],
        vec![cjk(6000), cjk(2000), cjk(2000)],
        vec![cjk(100), cjk(100)],
    ] {
        for batch in merge_ranges_to_threshold(ranges, 5000) {
            assert!(batch.chars.unwrap() >= 5000, "{batch:?}");
        }
    }
}

fn indexed(
    chars: usize,
    start: usize,
    end: usize,
    start_ref: &str,
    end_ref: &str,
) -> CompressibleRange {
    CompressibleRange {
        chars: Some(chars),
        start_index: Some(start),
        end_index: Some(end),
        ..hand_range(
            start_ref,
            end_ref,
            (chars as f64 / 4.0).round() as u64,
            end - start + 1,
        )
    }
}

fn positions(range: &CompressibleRange) -> (Option<usize>, Option<usize>) {
    (range.start_index, range.end_index)
}

#[test]
fn an_array_gap_is_never_bridged_even_when_the_sum_clears() {
    let ranges = vec![
        indexed(2000, 0, 1, "m00001", "m00002"),
        indexed(2000, 2, 3, "m00003", "m00004"),
        indexed(3000, 10, 12, "m00011", "m00013"),
        indexed(3000, 13, 15, "m00014", "m00016"),
    ];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!(out.len(), 1, "only the post-gap region qualifies");
    assert_eq!(
        (out[0].start_ref.as_str(), out[0].end_ref.as_str()),
        ("m00011", "m00016")
    );
    assert_eq!(positions(&out[0]), (Some(10), Some(15)));
}

#[test]
fn two_gapped_regions_each_clearing_stay_separate() {
    let ranges = vec![
        indexed(3000, 0, 1, "m00001", "m00002"),
        indexed(3000, 2, 3, "m00003", "m00004"),
        indexed(3000, 10, 11, "m00011", "m00012"),
        indexed(3000, 12, 13, "m00013", "m00014"),
    ];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!(out.len(), 2);
    assert_eq!(positions(&out[0]), (Some(0), Some(3)));
    assert_eq!(positions(&out[1]), (Some(10), Some(13)));
}

#[test]
fn a_short_tail_across_a_gap_is_dropped() {
    let ranges = vec![
        indexed(3000, 0, 1, "m00001", "m00002"),
        indexed(3000, 2, 3, "m00003", "m00004"),
        indexed(2000, 10, 11, "m00011", "m00012"),
    ];
    let out = merge_ranges_to_threshold(ranges, 5000);
    assert_eq!(out.len(), 1);
    assert_eq!(positions(&out[0]), (Some(0), Some(3)));
}
