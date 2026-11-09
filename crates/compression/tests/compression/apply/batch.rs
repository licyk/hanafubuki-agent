// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).

// Ports `validation.test.ts`, `issue3003-all-or-nothing-length.test.ts`,
// `issue4521-retry-livelock.test.ts` and `dead-refs.test.ts`.

use hanafubuki_agent_compression::{Config, Message, RangeSpec, RefMap, State};

use super::{block, defaults, gated, long, VALID_SUMMARY};
use crate::fixtures::{
    apply, assistant, call, config, range, result, sorted, state_for, user, with_topic,
};

// validation.test.ts

fn two_long() -> Vec<Message> {
    vec![long("a"), long("b")]
}

fn rejects(summary: &str, config: &Config, expected: &str) {
    let messages = two_long();
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00001", "m00002", summary)],
        &messages,
        &state,
        config,
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(
        outcome.result.errors[0].to_lowercase().contains(expected),
        "{:?}",
        outcome.result.errors
    );
}

#[test]
fn validation_rejects_an_empty_or_whitespace_only_summary() {
    rejects("", &defaults(), "empty");
    rejects("   \n\t  ", &defaults(), "empty");
}

#[test]
fn validation_rejects_a_summary_outside_its_length_limits() {
    let mut short = defaults();
    short.compress.min_compress_range = 0;
    short.compress.max_summary_length = 0;
    short.compress.min_summary_length = 100;
    rejects("too short", &short, "too short");
    let mut long_limit = defaults();
    long_limit.compress.min_compress_range = 0;
    long_limit.compress.max_summary_length = 50;
    long_limit.compress.min_summary_length = 0;
    rejects(VALID_SUMMARY, &long_limit, "too long");
}

#[test]
fn validation_rejects_a_range_below_min_compress_range() {
    let messages = vec![user("a", "short"), user("b", "text")];
    let state = state_for(&messages);
    let mut config = defaults();
    config.compress.max_summary_length = 0;
    config.compress.min_summary_length = 0;
    let outcome = apply(
        &[range("m00001", "m00002", VALID_SUMMARY)],
        &messages,
        &state,
        &config,
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.errors[0]
        .to_lowercase()
        .contains("too small"));
}

#[test]
fn validation_counts_the_batch_aggregate_against_min_compress_range() {
    let chunk = "x".repeat(1200);
    let messages: Vec<Message> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|id| user(id, &chunk))
        .collect();
    let state = state_for(&messages);
    let mut config = defaults();
    config.compress.max_summary_length = 0;
    config.compress.min_summary_length = 0;
    let ranges: Vec<RangeSpec> = (1..=5)
        .map(|index| {
            let r#ref = format!("m{index:05}");
            range(&r#ref, &r#ref, VALID_SUMMARY)
        })
        .collect();
    let outcome = apply(&ranges, &messages, &state, &config);
    assert_eq!(outcome.result.blocks_created, 5);
    assert!(outcome.result.errors.is_empty());
}

#[test]
fn validation_a_range_with_zero_compressible_messages_is_a_covered_no_op() {
    let messages = vec![long("a"), long("b"), long("c"), long("d")];
    let state = state_for(&messages);
    let first = apply(
        &[range("m00001", "m00003", VALID_SUMMARY)],
        &messages,
        &state,
        &defaults(),
    );
    let second = apply(
        &[range("m00002", "m00003", VALID_SUMMARY)],
        &messages,
        &first.state,
        &defaults(),
    );
    assert_eq!(second.result.blocks_created, 0);
    assert!(
        second.result.errors.is_empty(),
        "{:?}",
        second.result.errors
    );
    assert!(second.result.noop);
    let warnings = second.result.warnings.join(" ");
    assert!(
        warnings.contains("already covered by active block(s)")
            || warnings.contains("protected or filtered"),
        "{warnings}"
    );
}

#[test]
fn validation_block_boundary_compress_with_only_consumed_blocks_is_allowed() {
    let messages = vec![long("a"), long("b"), long("c")];
    let state = state_for(&messages);
    let t1 = apply(
        &[range("m00001", "m00002", VALID_SUMMARY)],
        &messages,
        &state,
        &defaults(),
    );
    let t2 = apply(
        &[range("b1", "b1", VALID_SUMMARY)],
        &messages,
        &t1.state,
        &defaults(),
    );
    assert_eq!(t2.result.blocks_created, 1);
    assert!(t2.result.errors.is_empty());
    assert_eq!(t2.state.blocks[1].tier, 2);
}

#[test]
fn validation_all_checks_disabled_allows_any_input() {
    let messages = vec![user("a", "x"), user("b", "y")];
    let state = state_for(&messages);
    let mut config = defaults();
    config.compress.min_compress_range = 0;
    config.compress.max_summary_length = 0;
    config.compress.min_summary_length = 0;
    let outcome = apply(
        &[range("m00001", "m00002", "ok")],
        &messages,
        &state,
        &config,
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(outcome.result.errors.is_empty());
}

#[test]
fn validation_an_empty_summary_range_rejects_the_whole_batch() {
    let messages = vec![long("a"), long("b"), long("c"), long("d")];
    let state = state_for(&messages);
    let outcome = apply(
        &[
            range("m00001", "m00002", ""),
            range("m00003", "m00004", VALID_SUMMARY),
        ],
        &messages,
        &state,
        &defaults(),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.state.blocks.is_empty());
    assert_eq!(outcome.result.errors.len(), 1);
    assert!(outcome.result.errors[0].contains("Batch rejected before folding any range"));
    assert!(outcome.result.errors[0].to_lowercase().contains("empty"));
}

// issue3003-all-or-nothing-length.test.ts

fn four() -> Vec<Message> {
    [
        ("a", "alpha"),
        ("b", "beta"),
        ("c", "gamma"),
        ("d", "delta"),
    ]
    .iter()
    .map(|(id, text)| user(id, text))
    .collect()
}

fn capped(max_summary_length: usize, min_summary_length: usize) -> Config {
    let mut config = config();
    config.compress.max_summary_length = max_summary_length;
    config.compress.min_summary_length = min_summary_length;
    config
}

#[test]
fn a_too_long_summary_in_a_multi_range_batch_rejects_the_whole_batch() {
    let messages = four();
    let state = state_for(&messages);
    let outcome = apply(
        &[
            with_topic(range("m00001", "m00002", &"x".repeat(40)), "too long"),
            with_topic(range("m00003", "m00004", "ok"), "fine"),
        ],
        &messages,
        &state,
        &capped(20, 0),
    );
    assert_eq!(outcome.result.blocks_created, 0, "nothing may commit");
    assert!(outcome.state.blocks.is_empty());
    assert_eq!(outcome.result.errors.len(), 1);
    let error = &outcome.result.errors[0];
    assert!(error.contains("Batch rejected before folding any range"));
    assert!(error.contains("range m00001..m00002: Summary too long (40 chars, max 20)"));
    assert!(error.contains("Nothing was folded, so no prefix was rewritten"));
    assert!(error.contains("shorten or split"));
    assert!(error.contains("drop the offending range(s) and resend only the rest"));
}

#[test]
fn a_too_short_summary_in_a_multi_range_batch_rejects_the_whole_batch() {
    let messages = four();
    let state = state_for(&messages);
    let outcome = apply(
        &[
            with_topic(range("m00001", "m00002", &"y".repeat(60)), "fine"),
            with_topic(range("m00003", "m00004", &"x".repeat(5)), "too short"),
        ],
        &messages,
        &state,
        &capped(0, 50),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.state.blocks.is_empty());
    assert!(outcome.result.errors[0].contains("Batch rejected before folding any range"));
    assert!(outcome.result.errors[0]
        .contains("range m00003..m00004: Summary too short (5 chars, min 50)"));
}

#[test]
fn a_multi_range_batch_whose_summaries_all_validate_commits_every_range() {
    let messages = four();
    let state = state_for(&messages);
    let outcome = apply(
        &[
            with_topic(range("m00001", "m00002", "ok-a"), "a"),
            with_topic(range("m00003", "m00004", "ok-b"), "b"),
        ],
        &messages,
        &state,
        &capped(20, 0),
    );
    assert_eq!(outcome.result.blocks_created, 2);
    assert_eq!(outcome.state.blocks.len(), 2);
    assert!(outcome.result.errors.is_empty());
}

#[test]
fn a_non_candidate_range_with_an_oversized_summary_does_not_reject_a_valid_fold() {
    let messages = vec![user("a", "alpha"), user("b", "beta")];
    let state = state_for(&messages);
    let outcome = apply(
        &[
            range("m00099", "m00100", &"z".repeat(500)),
            with_topic(range("m00001", "m00002", "ok"), "fine"),
        ],
        &messages,
        &state,
        &capped(20, 0),
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert_eq!(outcome.state.blocks.len(), 1);
    let errors = outcome.result.errors.join("\n");
    assert!(!errors.contains("Batch rejected before folding any range"));
    assert!(errors.contains("m00099"));
}

#[test]
fn a_too_long_refold_summary_rejects_the_whole_batch_with_its_fresh_sibling() {
    let messages = four();
    let state = state_for(&messages);
    let first = apply(
        &[with_topic(range("m00001", "m00002", "first fold"), "t")],
        &messages,
        &state,
        &config(),
    );
    assert_eq!(first.result.blocks_created, 1);
    let mut marked = first.state;
    marked.blocks[0].restored_inline = true;
    let second = apply(
        &[
            range("m00001", "m00002", &"z".repeat(400)),
            with_topic(range("m00003", "m00004", "fresh ok"), "sib"),
        ],
        &messages,
        &marked,
        &capped(20, 0),
    );
    assert_eq!(second.result.blocks_created, 0);
    assert_eq!(
        second.state.blocks.len(),
        1,
        "only the pre-existing block remains"
    );
    assert!(second.result.errors[0].contains("Batch rejected before folding any range"));
    assert!(second.result.errors[0]
        .contains("range m00001..m00002: Summary too long (400 chars, max 20)"));
    assert_eq!(second.state.blocks[0].summary, "first fold");
}

// issue4521-retry-livelock.test.ts

fn livelock_config() -> Config {
    let mut config = Config::new(872_000);
    config.compress.min_compress_range = 100;
    config.compress.max_summary_length = 20_000;
    config.compress.min_summary_length = 1;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

fn tail_session(count: usize) -> Vec<Message> {
    let filler = "x".repeat(3000);
    let mut messages = vec![user("u0", &filler)];
    for index in 1..count {
        messages.push(assistant(&format!("a{index}"), &filler));
    }
    messages
}

#[test]
fn an_end_above_the_highest_allocated_ref_clamps_down_instead_of_refusing() {
    let messages = tail_session(4);
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00002", "m00099", "work recap")],
        &messages,
        &state,
        &livelock_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|w| w.contains("clamped to it") && w.contains("highest allocated ref (m00004)")),
        "{:?}",
        outcome.result.warnings
    );
    let block = outcome
        .state
        .blocks
        .iter()
        .find(|block| block.active)
        .expect("a block was created");
    assert_eq!(sorted(block.direct_message_ids.clone()), ["a1", "a2", "a3"]);
}

#[test]
fn a_start_above_the_highest_allocated_ref_keeps_the_hard_refusal() {
    let messages = tail_session(3);
    let state = state_for(&messages);
    let outcome = apply(
        &[range("m00099", "m00104", "phantom work")],
        &messages,
        &state,
        &livelock_config(),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    let errors = outcome.result.errors.join("; ");
    assert!(
        errors.contains("above this session's highest allocated ref"),
        "{errors}"
    );
    assert!(!errors.contains("clamped"));
    assert!(!outcome.result.noop);
}

#[test]
fn a_range_entirely_inside_the_protected_zone_is_a_no_op() {
    let messages = tail_session(4);
    let state = state_for(&messages);
    let mut config = livelock_config();
    config.preserve_recent_messages = 4;
    let outcome = apply(
        &[range("m00001", "m00004", "newest work")],
        &messages,
        &state,
        &config,
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.noop);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|w| w.contains("protected zone") && w.contains("do not resend this range")),
        "{:?}",
        outcome.result.warnings
    );
    assert!(outcome.state.blocks.is_empty());
}

#[test]
fn a_range_of_pure_protected_tool_pairs_is_the_same_no_op_class() {
    let output = "x".repeat(3000);
    let messages = vec![
        call("c1", "call1", "bash", "args-call1"),
        result("r1", "call1", "bash", &output),
        call("c2", "call2", "bash", "args-call2"),
        result("r2", "call2", "bash", &output),
    ];
    let state = state_for(&messages);
    let mut config = livelock_config();
    config.protected_tools = vec!["bash".to_owned()];
    let outcome = apply(
        &[range("m00001", "m00004", "tool work")],
        &messages,
        &state,
        &config,
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(outcome.result.noop);
    assert!(
        outcome
            .result
            .warnings
            .iter()
            .any(|w| w.contains("protected or filtered") && w.contains("do not resend this range")),
        "{:?}",
        outcome.result.warnings
    );
}

// dead-refs.test.ts

fn dead_config() -> Config {
    gated(10)
}

/// Refs m00001..m00008 are known, but the resent view lacks m00005 and
/// m00006 and no block covers them.
fn vanished_scenario() -> (State, Vec<Message>) {
    let visible: Vec<Message> = [
        ("v1", "one "),
        ("v2", "two "),
        ("v3", "three "),
        ("v4", "four "),
        ("v7", "seven "),
        ("v8", "eight "),
    ]
    .iter()
    .map(|(id, word)| assistant(id, &word.repeat(40)))
    .collect();
    let mut refs = RefMap::default();
    for (index, id) in ["v1", "v2", "v3", "v4", "gone5", "gone6", "v7", "v8"]
        .iter()
        .enumerate()
    {
        let r#ref = format!("m{:05}", index + 1);
        refs.by_raw.insert((*id).to_owned(), r#ref.clone());
        refs.by_ref.insert(r#ref, (*id).to_owned());
    }
    let state = State {
        message_refs: refs,
        ..State::default()
    };
    (state, visible)
}

#[test]
fn dangling_endpoints_are_recorded_dead_and_the_receipt_forbids_retrying() {
    let (state, visible) = vanished_scenario();
    let outcome = apply(
        &[range("m00005", "m00006", "the vanished pair")],
        &visible,
        &state,
        &dead_config(),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    let error = &outcome.result.errors[0];
    assert!(error.contains("cannot be anchored"));
    assert!(error.contains("no active block covers them"));
    assert!(error.contains("recorded as DEAD"));
    assert!(error.contains("Do not retry this range in any form"));
    assert!(error.contains("refs m00005, m00006 are known"));
    assert_eq!(outcome.state.dead_refs, ["m00005", "m00006"]);
}

#[test]
fn retrying_the_same_dead_range_gives_the_same_verdict_and_grows_nothing() {
    let (state, visible) = vanished_scenario();
    let ranges = [range("m00005", "m00006", "the vanished pair")];
    let first = apply(&ranges, &visible, &state, &dead_config());
    let second = apply(&ranges, &visible, &first.state, &dead_config());
    assert_eq!(
        second.state.dead_refs,
        ["m00005", "m00006"],
        "the dead set is stable"
    );
    assert!(second.result.errors[0].contains("recorded as DEAD"));
    assert_eq!(second.result.blocks_created, 0);
}

/// The kernel lifts tombstones in `processTurn`; `applyCompression` lifts
/// them the same way, here through an empty batch.
#[test]
fn dead_tombstones_lift_individually_once_their_messages_reappear() {
    let (mut state, visible) = vanished_scenario();
    state.dead_refs = vec!["m00005".to_owned(), "m00006".to_owned()];
    let mut half_back = visible.clone();
    half_back.insert(4, assistant("gone5", &"five ".repeat(40)));
    let partly = apply(&[], &half_back, &state, &dead_config());
    assert_eq!(partly.state.dead_refs, ["m00006"]);

    let mut revived = half_back;
    revived.insert(5, assistant("gone6", &"six ".repeat(40)));
    let all = apply(&[], &revived, &state, &dead_config());
    assert!(all.state.dead_refs.is_empty(), "both tombstones lifted");
}

#[test]
fn mixed_batch_live_range_compresses_and_dead_range_warns_dead() {
    let (state, visible) = vanished_scenario();
    let outcome = apply(
        &[
            range("m00001", "m00004", "live span recap"),
            range("m00005", "m00006", "vanished pair"),
        ],
        &visible,
        &state,
        &dead_config(),
    );
    assert_eq!(
        outcome.result.blocks_created, 1,
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.errors.is_empty());
    let warning = outcome
        .result
        .warnings
        .iter()
        .find(|w| w.starts_with("Skipped range (m00005..m00006)"))
        .expect("a skip warning for the dead range");
    assert!(warning.contains("its refs m00005, m00006 are DEAD"));
    assert!(!warning.contains("already compressed"));
    assert_eq!(outcome.state.dead_refs, ["m00005", "m00006"]);
}

#[test]
fn covered_endpoints_keep_the_already_compressed_verdict_and_mark_nothing_dead() {
    let visible = vec![
        assistant("v3", &"three ".repeat(40)),
        assistant("v4", &"four ".repeat(40)),
    ];
    let mut refs = RefMap::default();
    for (index, id) in ["a1", "a2", "v3", "v4"].iter().enumerate() {
        let r#ref = format!("m{:05}", index + 1);
        refs.by_raw.insert((*id).to_owned(), r#ref.clone());
        refs.by_ref.insert(r#ref, (*id).to_owned());
    }
    let mut state = State {
        message_refs: refs,
        ..State::default()
    };
    state
        .blocks
        .push(block("b1", 1, &["a1", "a2"], &["a1", "a2"], &[], true));
    let outcome = apply(
        &[range("m00001", "m00002", "refold attempt")],
        &visible,
        &state,
        &dead_config(),
    );
    assert_eq!(outcome.result.blocks_created, 0);
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.noop);
    let warnings = outcome.result.warnings.join(" ");
    assert!(warnings.contains("already compressed"));
    assert!(warnings.contains("b1"));
    assert!(warnings.contains("do not resend"));
    assert!(outcome.state.dead_refs.is_empty());
}
