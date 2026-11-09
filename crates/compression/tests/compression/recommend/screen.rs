// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `recommend-range-integrity.test.ts`, `effective-gate-chars.test.ts`,
// `issue3236-recommend-apply-consistency.test.ts`,
// `issue3371-consumed-unit-screen.test.ts`, the verdicts of
// `issue3406-screen-observability.test.ts` and the integrity-source case of
// `carrier-advertise-validate.test.ts`: every advertised range applies.

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    assign_refs, build_compressible_ranges, build_status_report, compute_protected_refs,
    merge_ranges_to_threshold, screen_recommended_units, Block, CompressibleRange, Config, Message,
    Role, State, StatusReportOptions, StatusScope, StatusView,
};

use super::{recommend_config, ref_of, turn, user};
use crate::fixtures::{apply, assistant, call, range, reasoning, result, state_for, with_topic};

fn unit(start: &str, end: &str, count: usize, tokens: u64) -> CompressibleRange {
    CompressibleRange {
        start_ref: start.to_owned(),
        end_ref: end.to_owned(),
        count,
        tokens,
        ..CompressibleRange::default()
    }
}

fn no_zone() -> HashSet<String> {
    HashSet::new()
}

/// Recommends as the kernel's integrity test does: zone, ranges, merge.
fn recommend_merged(
    messages: &[Message],
    state: &State,
    config: &Config,
) -> Vec<CompressibleRange> {
    let zone = compute_protected_refs(messages, state, config);
    let ranges = build_compressible_ranges(messages, state, config, Some(&zone), messages);
    merge_ranges_to_threshold(ranges.compressible, config.compress.min_compress_range)
}

fn one_turn() -> Vec<Message> {
    vec![
        user("u0", "task"),
        assistant("a0", "ack"),
        reasoning("r1", "thinking about the task"),
        call("c1", "call_1", "edit", "call edit"),
        result("t1", "call_1", "edit", "edit output"),
        assistant("a1", "summarized the read"),
        user("u1", "next"),
        assistant("a2", "reply"),
    ]
}

#[test]
fn a_turn_split_by_the_protected_zone_is_not_advertised() {
    let messages = one_turn();
    let mut config = recommend_config();
    config.preserve_recent_messages = 4;
    let head = apply(
        &[range("m00001", "m00002", "head recap")],
        &messages,
        &state_for(&messages),
        &config,
    );
    assert!(head.result.errors.is_empty(), "{:?}", head.result.errors);
    let recommended = recommend_merged(&messages, &head.state, &config);
    for advertised in &recommended {
        let outcome = apply(
            &[range(&advertised.start_ref, &advertised.end_ref, "fold it")],
            &messages,
            &head.state,
            &config,
        );
        assert!(
            outcome.result.errors.is_empty(),
            "{:?}",
            outcome.result.errors
        );
    }
    assert!(
        recommended.is_empty(),
        "residue the fold gate always withdraws must not be advertised"
    );
}

#[test]
fn a_wholly_compressible_turn_is_still_advertised() {
    let messages = one_turn();
    let mut config = recommend_config();
    config.preserve_recent_messages = 2;
    let state = state_for(&messages);
    let recommended = recommend_merged(&messages, &state, &config);
    assert!(
        recommended
            .iter()
            .any(|range| range.start_ref.as_str() <= "m00003" && range.end_ref.as_str() >= "m00006"),
        "{recommended:?}"
    );
    let outcome = apply(
        &[range("m00003", "m00006", "turn recap")],
        &messages,
        &state,
        &config,
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert_eq!(outcome.result.blocks_created, 1);
}

// --- effective-gate-chars: recommendations count characters as apply does ---

fn gate_config() -> Config {
    let mut config = recommend_config();
    config.nudge.max_context_limit_pct = 0.9;
    config.compress.min_compress_range = 5000;
    config.compress.max_summary_length = 3000;
    config.compress.min_summary_length = 100;
    config
}

fn cjk_messages(chars_per_message: usize, count: usize) -> Vec<Message> {
    (0..count)
        .map(|index| {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            Message::text(
                format!("raw-{index}"),
                role,
                "内容".repeat(chars_per_message / 2),
            )
        })
        .collect()
}

#[test]
fn a_hand_built_range_without_chars_counts_four_characters_a_token() {
    let legacy = CompressibleRange {
        text_pct: 100,
        ..unit("m00001", "m00002", 2, 1250)
    };
    assert_eq!(merge_ranges_to_threshold(vec![legacy], 5000).len(), 1);
}

#[test]
fn a_cjk_session_under_the_minimum_range_is_not_offered() {
    let config = gate_config();
    let messages = cjk_messages(500, 6);
    let first = turn(&messages, State::default(), &config, 10_000);
    let out = turn(&messages, first.state, &config, 95_000);
    assert!(!out.nudge.should_inject, "3000 chars < 5000");
    assert!(
        out.nudge
            .reason
            .contains("no tier has effective compressible content"),
        "{}",
        out.nudge.reason
    );
    let applied = apply(
        &[with_topic(range("m00001", "m00006", "s"), "t")],
        &messages,
        &out.state,
        &config,
    );
    assert_eq!(applied.result.blocks_created, 0);
    assert!(
        applied
            .result
            .errors
            .iter()
            .any(|error| error.contains("too small")),
        "{:?}",
        applied.result.errors
    );
}

#[test]
fn a_cjk_session_over_the_minimum_range_is_offered() {
    let config = gate_config();
    let messages = cjk_messages(1000, 6);
    let first = turn(&messages, State::default(), &config, 10_000);
    let out = turn(&messages, first.state, &config, 95_000);
    assert!(out.nudge.should_inject, "6000 chars >= 5000");
    assert!(out.nudge.reason.contains("T1"), "{}", out.nudge.reason);
    let applied = apply(
        &[with_topic(
            range("m00001", "m00006", &"总结".repeat(60)),
            "t",
        )],
        &messages,
        &out.state,
        &config,
    );
    assert_eq!(applied.result.blocks_created, 1);
    assert!(
        applied.result.errors.is_empty(),
        "{:?}",
        applied.result.errors
    );
}

// --- #3236: what is advertised is what applies ---

fn consistency_config() -> Config {
    let mut config = recommend_config();
    config.tiers.enabled = false;
    config.compress.min_compress_range = 5000;
    config.compress.max_summary_length = 20_000;
    config.compress.min_summary_length = 1;
    config
}

fn sized(message: Message, len: usize) -> Message {
    Message {
        text: "x".repeat(len),
        ..message
    }
}

fn text_of(id: &str, role: Role, len: usize) -> Message {
    sized(Message::text(id, role, ""), len)
}

fn reasoning_of(id: &str, len: usize) -> Message {
    sized(reasoning(id, ""), len)
}

fn call_of(id: &str, tool: &str, call_id: &str) -> Message {
    sized(call(id, call_id, tool, ""), 1500)
}

fn result_of(id: &str, tool: &str, call_id: &str, len: usize) -> Message {
    sized(result(id, call_id, tool, ""), len)
}

/// A folded head (`b1` over `m00001..m00002`).
fn head_fold(config: &Config) -> (State, Vec<Message>) {
    let all = vec![
        text_of("U0", Role::User, 3000),
        text_of("W1", Role::Assistant, 3000),
    ];
    let folded = apply(
        &[range("m00001", "m00002", "earlier work")],
        &all,
        &state_for(&all),
        config,
    );
    assert!(
        folded.result.errors.is_empty(),
        "{:?}",
        folded.result.errors
    );
    (folded.state, all)
}

fn assign_from(all: &[Message], state: &mut State, next: u32) {
    state.message_refs = assign_refs(all, &state.message_refs, next, |_| false).map;
}

fn filler_turn(all: &mut Vec<Message>, index: usize) {
    all.push(text_of(&format!("UF{index}"), Role::User, 200));
    all.push(reasoning_of(&format!("RF{index}"), 300));
    all.push(sized(
        call(&format!("CF{index}"), &format!("cf{index}"), "bash", ""),
        300,
    ));
}

fn filler_result(all: &mut Vec<Message>, index: usize) {
    all.push(result_of(
        &format!("RSF{index}"),
        "bash",
        &format!("cf{index}"),
        300,
    ));
}

/// A burst whose first result is immediate and whose other four arrive
/// after six filler turns (interleaved among them, or all at the end).
fn delayed_results(config: &Config, interleave: bool) -> (State, Vec<Message>) {
    let (mut state, mut all) = head_fold(config);
    all.extend([
        reasoning_of("R", 2000),
        call_of("C1", "bash", "c1"),
        call_of("C2", "read", "c2"),
        call_of("C3", "bash", "c3"),
        call_of("C4", "read", "c4"),
        call_of("C5", "bash", "c5"),
        result_of("RS1", "read", "c1", 300),
    ]);
    let mut late = vec![
        result_of("RS2", "read", "c2", 300),
        result_of("RS3", "bash", "c3", 300),
        result_of("RS4", "read", "c4", 300),
        result_of("RS5", "bash", "c5", 300),
    ]
    .into_iter();
    for index in 0..6 {
        filler_turn(&mut all, index);
        if interleave && index == 2 {
            all.extend(late.next());
        }
        filler_result(&mut all, index);
        if interleave && index == 4 {
            all.extend(late.next());
        }
    }
    all.extend(late);
    assign_from(&all, &mut state, 3);
    (state, all)
}

/// Two interleaved bursts whose late results come after six filler turns.
fn two_bursts(config: &Config) -> (State, Vec<Message>) {
    let (mut state, mut all) = head_fold(config);
    all.extend([
        reasoning_of("RA", 2000),
        call_of("CA1", "bash", "a1"),
        call_of("CA2", "read", "a2"),
        call_of("CA3", "bash", "a3"),
        result_of("RA_RS1", "bash", "a1", 300),
        reasoning_of("RB", 2000),
        call_of("CB1", "read", "b1"),
        call_of("CB2", "bash", "b2"),
        call_of("CB3", "read", "b3"),
        result_of("RB_RS1", "read", "b1", 300),
    ]);
    for index in 0..6 {
        filler_turn(&mut all, index);
        filler_result(&mut all, index);
    }
    all.extend([
        result_of("LA2", "read", "a2", 300),
        result_of("LA3", "bash", "a3", 300),
        result_of("LB2", "bash", "b2", 300),
        result_of("LB3", "read", "b3", 300),
    ]);
    assign_from(&all, &mut state, 3);
    (state, all)
}

/// Every range the nudge advertises applies cleanly against the state it
/// was advertised from.
fn assert_advertised_apply(config: &Config, state: &State, all: &[Message], label: &str) {
    let out = turn(all, state.clone(), config, 70_000);
    assert!(
        !out.nudge.compressible_ranges.is_empty(),
        "{label}: nothing advertised"
    );
    for advertised in &out.nudge.compressible_ranges {
        let applied = apply(
            &[range(
                &advertised.start_ref,
                &advertised.end_ref,
                "steady-state fold",
            )],
            all,
            state,
            config,
        );
        assert!(
            applied.result.errors.is_empty(),
            "{label}: advertised {}-{} failed: {:?}",
            advertised.start_ref,
            advertised.end_ref,
            applied.result.errors
        );
    }
}

#[test]
fn a_burst_with_delayed_results_folds_whole_and_every_advertised_unit_applies() {
    let config = consistency_config();
    for interleave in [false, true] {
        let (state, all) = delayed_results(&config, interleave);
        assert_advertised_apply(&config, &state, &all, if interleave { "A2" } else { "A" });
        let burst = apply(
            &[range(
                ref_of(&state, "R"),
                ref_of(&state, "RS1"),
                "manual burst span",
            )],
            &all,
            &state,
            &config,
        );
        assert!(burst.result.errors.is_empty(), "{:?}", burst.result.errors);
        assert_eq!(burst.result.blocks_created, 1);
    }
}

#[test]
fn two_interleaved_bursts_advertise_cleanly_and_fold_whole() {
    let config = consistency_config();
    let (state, all) = two_bursts(&config);
    assert_advertised_apply(&config, &state, &all, "B");
    let burst = apply(
        &[range(
            ref_of(&state, "RA"),
            ref_of(&state, "RB_RS1"),
            "manual double-burst span",
        )],
        &all,
        &state,
        &config,
    );
    assert!(burst.result.errors.is_empty(), "{:?}", burst.result.errors);
    assert_eq!(burst.result.blocks_created, 1);
}

#[test]
fn a_healthy_complete_turn_stays_advertised() {
    let config = consistency_config();
    let all = vec![
        text_of("U", Role::User, 1000),
        reasoning_of("R", 1000),
        call_of("C", "bash", "c1"),
        result_of("RS", "bash", "c1", 3000),
    ];
    let state = state_for(&all);
    assert_advertised_apply(&config, &state, &all, "complete-turn");
    let position = |r#ref: &str| {
        all.iter()
            .position(|message| ref_of(&state, &message.id) == r#ref)
    };
    // R and RS.
    let (low, high) = (1, 3);
    let out = turn(&all, state.clone(), &config, 70_000);
    assert!(
        out.nudge.compressible_ranges.iter().any(|advertised| {
            matches!(
                (position(&advertised.start_ref), position(&advertised.end_ref)),
                (Some(start), Some(end)) if start <= low && end >= high
            )
        }),
        "{:?}",
        out.nudge.compressible_ranges
    );
}

fn screened_report(state: &State, all: &[Message], config: &Config) -> String {
    build_status_report(
        state,
        all,
        &StatusReportOptions {
            scope: Some(StatusScope::Uncompressed),
            view: StatusView::Ranges,
            screen: Some((config, all)),
            ..StatusReportOptions::default()
        },
    )
}

#[test]
fn status_rows_stay_listed_and_flag_only_what_stays_doomed() {
    let config = consistency_config();
    let (state, all) = two_bursts(&config);
    let report = screened_report(&state, &all, &config);
    let row = |first: &str, last: &str| {
        report
            .lines()
            .find(|line| {
                line.contains(ref_of(&state, first)) && line.contains(ref_of(&state, last))
            })
            .map(str::to_owned)
    };
    let burst = row("RA", "RB_RS1").expect("burst row listed");
    assert!(!burst.contains('\u{26a0}'), "{burst}");
    let filler = row("UF0", "RSF0").expect("filler row listed");
    assert!(!filler.contains('\u{26a0}'), "{filler}");

    let legacy = build_status_report(
        &state,
        &all,
        &StatusReportOptions {
            scope: Some(StatusScope::Uncompressed),
            ..StatusReportOptions::default()
        },
    );
    assert!(!legacy.contains('\u{26a0}'), "no screen, no flags");
}

#[test]
fn the_status_doom_flag_still_fires_for_a_row_inside_the_zone() {
    let config = consistency_config();
    let (mut state, mut all) = head_fold(&config);
    all.extend([
        text_of("X1", Role::User, 200),
        text_of("X2", Role::Assistant, 200),
    ]);
    assign_from(&all, &mut state, 3);
    let mut zoned = config;
    zoned.preserve_recent_messages = 2;
    let report = screened_report(&state, &all, &zoned);
    for id in ["X1", "X2"] {
        let line = report
            .lines()
            .find(|line| line.contains(ref_of(&state, id)))
            .unwrap_or_else(|| panic!("{id} row listed: {report}"));
        assert!(line.contains("\u{26a0} not foldable"), "{line}");
    }
}

// --- #3371: a consumed unit is not advertised ---

fn consumed_config() -> Config {
    let mut config = recommend_config();
    config.tiers.enabled = false;
    config.compress.min_compress_range = 500;
    config.compress.max_summary_length = 20_000;
    config.compress.min_summary_length = 1;
    config
}

/// An active `b1` over `m00001..m00002` and two fresh messages after it.
fn consumed_session() -> (State, Vec<Message>) {
    let all = vec![
        text_of("U0", Role::User, 3000),
        text_of("W1", Role::Assistant, 3000),
        text_of("U3", Role::User, 3000),
        text_of("W3", Role::Assistant, 3000),
    ];
    let folded = apply(
        &[range("m00001", "m00002", "earlier work")],
        &all,
        &state_for(&all),
        &consumed_config(),
    );
    assert!(
        folded.result.errors.is_empty(),
        "{:?}",
        folded.result.errors
    );
    (folded.state, all)
}

#[test]
fn a_consumed_unit_is_screened_out_and_apply_agrees_with_a_no_op() {
    let (state, all) = consumed_session();
    let config = consumed_config();
    let screened = screen_recommended_units(
        vec![unit("m00001", "m00002", 2, 6000)],
        &all,
        &state,
        &config,
        &no_zone(),
    );
    assert!(screened.is_empty());
    let applied = apply(&[range("m00001", "m00002", "nope")], &all, &state, &config);
    assert!(
        applied.result.errors.is_empty(),
        "{:?}",
        applied.result.errors
    );
    assert_eq!(applied.result.blocks_created, 0);
    assert!(applied.result.noop);
    let warnings = applied.result.warnings.join("; ");
    assert!(
        warnings.contains("already covered by active block(s) b1"),
        "{warnings}"
    );
    assert!(warnings.contains("do not resend this range"), "{warnings}");
}

#[test]
fn a_genuine_refold_stays_advertised_and_applies_in_place() {
    let (mut state, all) = consumed_session();
    let config = consumed_config();
    let b1: &mut Block = state
        .blocks
        .iter_mut()
        .find(|block| block.block_id == "b1")
        .expect("b1");
    b1.restored_inline = true;
    let screened = screen_recommended_units(
        vec![unit("m00001", "m00002", 2, 6000)],
        &all,
        &state,
        &config,
        &no_zone(),
    );
    assert_eq!(screened.len(), 1);
    let applied = apply(
        &[range("m00001", "m00002", "refolded work")],
        &all,
        &state,
        &config,
    );
    assert!(
        applied.result.errors.is_empty(),
        "{:?}",
        applied.result.errors
    );
    let b1 = applied.state.block("b1").expect("refolded in place");
    assert_eq!(b1.summary, "refolded work");
    assert!(!b1.restored_inline);
    assert_eq!(applied.state.active_blocks().count(), 1);
}

#[test]
fn a_fresh_unit_beside_the_consumed_one_stays_advertised() {
    let (state, all) = consumed_session();
    let screened = screen_recommended_units(
        vec![unit("m00003", "m00004", 2, 6000)],
        &all,
        &state,
        &consumed_config(),
        &no_zone(),
    );
    assert_eq!(screened.len(), 1);
}

// --- #3406: units that cannot resolve are dropped (the kernel's warning
// on an unexpected error is not ported: the crate does not log) ---

fn plain_session() -> (State, Vec<Message>) {
    let all = vec![
        text_of("U0", Role::User, 3000),
        text_of("W1", Role::Assistant, 3000),
        text_of("U2", Role::User, 3000),
        text_of("W3", Role::Assistant, 3000),
    ];
    (state_for(&all), all)
}

#[test]
fn units_that_cannot_resolve_are_dropped() {
    let (state, all) = plain_session();
    let config = Config::new(100_000);
    for bad in [
        unit("zz-invalid", "m00002", 2, 6000),
        unit("m09999", "m01000", 2, 6000),
    ] {
        assert!(screen_recommended_units(vec![bad], &all, &state, &config, &no_zone()).is_empty());
    }
    let view = vec![
        text_of("U0", Role::User, 3000),
        text_of("W3", Role::Assistant, 3000),
    ];
    assert!(
        screen_recommended_units(
            vec![unit("m00002", "m00002", 1, 3000)],
            &view,
            &state,
            &config,
            &no_zone()
        )
        .is_empty(),
        "a ref whose message is not in the screened array"
    );
}

#[test]
fn a_healthy_unit_passes_screening() {
    let (state, all) = plain_session();
    let mut config = Config::new(100_000);
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    let screened = screen_recommended_units(
        vec![unit("m00001", "m00002", 2, 6000)],
        &all,
        &state,
        &config,
        &no_zone(),
    );
    assert_eq!(screened.len(), 1);
}

// --- #2663: integrity is judged on the array the apply step runs on ---

#[test]
fn integrity_withdrawal_is_judged_on_the_original_array() {
    let config = recommend_config();
    let original = vec![
        reasoning("R1", "thinking hard"),
        call("C", "c1", "read", "{}"),
        result("R", "c1", "read", "out"),
    ];
    let pruned_like = original[..2].to_vec();
    let mut state = state_for(&original);
    state.blocks.push(Block {
        block_id: "b1".to_owned(),
        run_id: "run-t4".to_owned(),
        summary: "covers the result only".to_owned(),
        direct_message_ids: vec!["R".to_owned()],
        effective_message_ids: vec!["R".to_owned()],
        compressed_tokens: 10,
        ..Block::default()
    });
    let call_index = 1;
    let spans_call = |range: &CompressibleRange| {
        range.start_index.unwrap() <= call_index && range.end_index.unwrap() >= call_index
    };

    let view_judgment =
        build_compressible_ranges(&pruned_like, &state, &config, None, &pruned_like);
    assert!(
        view_judgment.compressible.iter().any(spans_call),
        "the view without the result cannot see the split pair: {view_judgment:?}"
    );
    let original_judgment =
        build_compressible_ranges(&pruned_like, &state, &config, None, &original);
    assert!(
        !original_judgment.compressible.iter().any(spans_call),
        "{original_judgment:?}"
    );
}
