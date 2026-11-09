// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `prune-recommend-scale.test.ts`, and the functional pins of
// `issue3406-screen-scale.test.ts` and `issue5337-shared-screening.test.ts`
// (their wall-time caps, tuned to a JIT, are not ported; the #498 cases keep
// a loose ten-second bound, and the n = 20000 chain is not ported).

use std::collections::HashSet;
use std::time::{Duration, Instant};

use hanafubuki_agent_compression::{
    build_compressible_ranges, compute_protected_refs, merge_ranges_to_threshold, project_unit,
    prune, screen_recommended_units, summary_message_id, CompressibleRange, Config, Kind, Message,
    ProjectionContext, RenderStrategy, State, UnitRequest, SUMMARY_HEADER,
};

use super::{block, user};
use crate::fixtures::{call, reasoning, result, state_for};

const LOOSE_WALL: Duration = Duration::from_secs(10);

fn short_user(id: &str) -> Message {
    user(id, &format!("u-{id}"))
}

fn read_call(id: &str, call_id: &str) -> Message {
    call(id, call_id, "read", "{}")
}

fn read_result(id: &str, call_id: &str) -> Message {
    result(id, call_id, "read", &format!("result-{call_id}"))
}

fn ids(messages: &[Message]) -> Vec<&str> {
    messages.iter().map(|message| message.id.as_str()).collect()
}

fn pruned_with(messages: &[Message], effective: &[&str]) -> Vec<String> {
    let mut state = State::default();
    state.blocks.push(block(1, effective));
    prune(messages, &state)
        .into_iter()
        .map(|message| message.id)
        .collect()
}

// --- prune anchor pairing ---

#[test]
fn an_anchor_crossing_one_result_moves_past_a_farther_call() {
    let messages = [
        short_user("u0"),
        read_call("c1", "ta"),
        short_user("u2"),
        read_call("c3", "tb"),
        read_result("o4", "ta"),
        short_user("u5"),
        read_result("o6", "tb"),
        short_user("u7"),
    ];
    assert_eq!(
        pruned_with(&messages, &["u2"]),
        ["u0", "c1", "c3", "o4", "u5", "o6", "acp_summary_b1", "u7"]
    );
}

#[test]
fn the_anchor_uses_the_earliest_result_of_a_repeated_call_id() {
    let messages = [
        short_user("u0"),
        read_call("c1", "td"),
        read_result("o2", "td"),
        read_result("o3", "td"),
        short_user("u4"),
    ];
    assert_eq!(
        pruned_with(&messages, &["o2"]),
        ["u0", "c1", "acp_summary_b1", "o3", "u4"]
    );
}

#[test]
fn the_anchor_does_not_move_past_an_unanswered_call() {
    let messages = [short_user("u0"), read_call("c1", "tx"), short_user("u2")];
    assert_eq!(pruned_with(&messages, &["u2"]), ["u0", "acp_summary_b1"]);
}

#[test]
fn the_anchor_snaps_to_the_run_start_then_crosses_the_burst() {
    let messages = [
        short_user("u0"),
        reasoning("r1", "r-r1"),
        read_call("c2", "t1"),
        read_call("c3", "t2"),
        read_result("o4", "t1"),
        read_result("o5", "t2"),
    ];
    assert_eq!(
        pruned_with(&messages, &["r1"]),
        ["u0", "acp_summary_b1", "c2", "c3", "o4", "o5"]
    );
}

// --- large sessions ---

/// 7250 reasoning, call and result triples under 132 active tier-1 blocks.
fn large_session() -> (Vec<Message>, State) {
    let mut messages = vec![short_user("m0")];
    for index in 0..7250 {
        messages.push(reasoning(&format!("r{index}"), &format!("r-r{index}")));
        messages.push(read_call(&format!("c{index}"), &format!("t{index}")));
        messages.push(Message {
            text: "x".repeat(1200),
            ..read_result(&format!("o{index}"), &format!("t{index}"))
        });
    }
    let mut state = state_for(&messages);
    for index in 0..132 {
        let ids: Vec<&str> = messages[1 + index * 162..1 + (index + 1) * 162]
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        state.blocks.push(block(index as u32 + 1, &ids));
    }
    state.next_block_id = 133;
    state.next_run_id = 133;
    (messages, state)
}

/// Tool calls not answered before the next message that is not a tool
/// message or reasoning.
fn unanswered_tool_calls(messages: &[Message]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut calls: Vec<&str> = Vec::new();
    let mut answers: Vec<&str> = Vec::new();
    let mut flush = |calls: &mut Vec<&str>, answers: &mut Vec<&str>| {
        violations.extend(
            calls
                .iter()
                .filter(|call| !answers.contains(call))
                .map(|call| (*call).to_owned()),
        );
        calls.clear();
        answers.clear();
    };
    for message in messages {
        match (message.kind, message.tool_call_id.as_deref()) {
            (Kind::Reasoning, _) => {}
            (Kind::ToolCall, Some(call_id)) => calls.push(call_id),
            (Kind::ToolResult, Some(call_id)) => answers.push(call_id),
            _ => flush(&mut calls, &mut answers),
        }
    }
    flush(&mut calls, &mut answers);
    violations
}

fn turn_text_only(
    messages: &[Message],
    state: State,
    config: &Config,
) -> hanafubuki_agent_compression::TurnOutput {
    hanafubuki_agent_compression::process_turn(hanafubuki_agent_compression::TurnInput {
        messages,
        state,
        config,
        token_count: 148_265,
        token_count_source: None,
        render: RenderStrategy::TextOnly,
        clock: None,
    })
}

#[test]
fn a_turn_stays_correct_at_21k_messages_and_132_blocks() {
    let (messages, state) = large_session();
    let config = Config::new(212_992);
    let start = Instant::now();
    let out = turn_text_only(&messages, state, &config);
    let elapsed = start.elapsed();

    assert_eq!(out.messages.len(), 499);
    let out_ids = ids(&out.messages);
    for index in 1..=132 {
        let summary = summary_message_id(&format!("b{index}"));
        assert_eq!(
            out_ids.iter().filter(|id| **id == summary).count(),
            1,
            "{summary}"
        );
    }
    let first = out_ids
        .iter()
        .position(|id| *id == summary_message_id("b1"));
    let last = out_ids
        .iter()
        .rposition(|id| *id == summary_message_id("b132"));
    assert!(matches!((first, last), (Some(first), Some(last)) if last > first));
    assert!(unanswered_tool_calls(&out.messages).is_empty());
    assert!(elapsed < LOOSE_WALL, "{elapsed:?}");

    // Feeding the view back keeps its structure.
    let again = turn_text_only(&out.messages, out.state.clone(), &config);
    assert_eq!(ids(&again.messages), ids(&out.messages));
}

#[test]
fn recommendations_stay_correct_at_scale() {
    let (messages, state) = large_session();
    let config = Config::new(212_992);
    let start = Instant::now();
    let zone = compute_protected_refs(&messages, &state, &config);
    let ranges = build_compressible_ranges(&messages, &state, &config, Some(&zone), &messages);
    let elapsed = start.elapsed();

    let covered = state.covered_message_ids();
    for range in &ranges.compressible {
        for message in &messages[range.start_index.unwrap()..=range.end_index.unwrap()] {
            assert!(
                !covered.contains(message.id.as_str()),
                "{} leaked",
                message.id
            );
        }
    }
    for r#ref in &zone {
        assert!(
            r#ref.len() == 6
                && r#ref.starts_with('m')
                && r#ref[1..].bytes().all(|b| b.is_ascii_digit()),
            "{ref}"
        );
    }
    assert!(!ranges.compressible.is_empty());
    assert!(elapsed < LOOSE_WALL, "{elapsed:?}");
}

fn no_zone_config() -> Config {
    let mut config = Config::new(200_000);
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

fn listed_starts(messages: &[Message], state: &State, config: &Config) -> Vec<String> {
    let zone = compute_protected_refs(messages, state, config);
    build_compressible_ranges(messages, state, config, Some(&zone), messages)
        .compressible
        .into_iter()
        .map(|range| range.start_ref)
        .collect()
}

#[test]
fn only_active_blocks_exclude_messages() {
    let messages = [
        short_user("a"),
        short_user("b"),
        short_user("c"),
        short_user("d"),
    ];
    let mut state = state_for(&messages);
    state.blocks.push(block(1, &["a"]));
    let mut inactive = block(2, &["b"]);
    inactive.active = false;
    state.blocks.push(inactive);
    let listed = listed_starts(&messages, &state, &no_zone_config());
    assert!(!listed.contains(&"m00001".to_owned()));
    assert!(listed.contains(&"m00002".to_owned()));
}

#[test]
fn summary_text_is_pruned_whatever_the_coverage() {
    let messages = [
        short_user("a"),
        user("sum", &format!("{SUMMARY_HEADER} — leftover")),
        short_user("c"),
    ];
    let state = state_for(&messages);
    let listed = listed_starts(&messages, &state, &no_zone_config());
    assert!(!listed.contains(&"m00002".to_owned()));
}

// --- the screen at n = 2000 and more ---

/// A ten-message turn pattern (users, reasoning, call, result, assistant
/// fill) with 40 active tier-1 blocks over the first 800 messages.
fn screen_session(n: usize) -> (State, Vec<Message>, Config, HashSet<String>) {
    let messages: Vec<Message> = (0..n)
        .map(|index| match index % 10 {
            0 | 1 => user(&format!("u{index}"), &"u".repeat(800)),
            2 => reasoning(&format!("r{index}"), &"r".repeat(500)),
            3 => read_call(&format!("c{index}"), &format!("t{index}")),
            4 => Message {
                text: "x".repeat(1200),
                ..read_result(&format!("o{index}"), &format!("t{index}"))
            },
            _ => super::text(
                &format!("a{index}"),
                &"a".repeat(700),
                hanafubuki_agent_compression::Role::Assistant,
            ),
        })
        .collect();
    let mut state = state_for(&messages);
    for index in 0..40 {
        let ids: Vec<&str> = messages[index * 20..(index + 1) * 20]
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        state.blocks.push(block(index as u32 + 1, &ids));
    }
    let config = Config::new(212_992);
    let zone = compute_protected_refs(&messages, &state, &config);
    (state, messages, config, zone)
}

fn padded(index: usize) -> String {
    format!("m{:05}", index + 1)
}

/// Fresh units of ten over the uncovered tail, from raw message `from`.
fn tail_units(from: usize, count: usize) -> Vec<CompressibleRange> {
    (0..count)
        .map(|k| CompressibleRange {
            start_ref: padded(from + k * 10),
            end_ref: padded(from + k * 10 + 9),
            count: 10,
            tokens: 2000,
            ..CompressibleRange::default()
        })
        .collect()
}

#[test]
fn every_fresh_tail_unit_survives_the_screen() {
    let (state, messages, config, zone) = screen_session(2000);
    for count in [10, 40, 120] {
        let kept =
            screen_recommended_units(tail_units(800, count), &messages, &state, &config, &zone);
        assert_eq!(kept.len(), count);
    }
}

fn recommend_chain(state: &State, messages: &[Message], config: &Config) -> Vec<CompressibleRange> {
    let zone = compute_protected_refs(messages, state, config);
    let ranges = build_compressible_ranges(messages, state, config, Some(&zone), messages);
    let merged = merge_ranges_to_threshold(ranges.compressible, config.compress.min_compress_range);
    screen_recommended_units(merged, messages, state, config, &zone)
}

#[test]
fn a_large_session_still_offers_ranges() {
    let (state, messages, config, _) = screen_session(2000);
    assert!(!recommend_chain(&state, &messages, &config).is_empty());
}

#[test]
fn the_screen_keeps_exactly_the_units_the_projection_keeps() {
    let (state, messages, config, zone) = screen_session(1200);
    let units = tail_units(810, 12);
    let coverage: HashSet<String> = state
        .covered_message_ids()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let context = ProjectionContext::new(&messages, &state, &config);
    let expected: Vec<String> = units
        .iter()
        .filter(|unit| {
            let request = UnitRequest {
                start_ref: &unit.start_ref,
                end_ref: &unit.end_ref,
                pre_existing_coverage: &coverage,
                protected_refs: Some(&zone),
            };
            project_unit(&context, &request, &mut Vec::new()).is_ok_and(|projection| {
                projection.fatal.is_none()
                    && !(projection.filtered_ids.is_empty()
                        && projection.consumed_block_ids.is_empty())
            })
        })
        .map(|unit| unit.start_ref.clone())
        .collect();
    let kept: Vec<String> = screen_recommended_units(units, &messages, &state, &config, &zone)
        .into_iter()
        .map(|unit| unit.start_ref)
        .collect();
    assert_eq!(kept, expected);
}
