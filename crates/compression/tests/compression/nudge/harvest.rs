// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `issue5698-t1-harvest-stall.test.ts`: a tier-1 nudge answered by a
// fold taking less than half of its offer lets the distillation tiers go
// first.

use hanafubuki_agent_compression::{ApplyOutcome, Block, Message, State};

use super::{make_messages, nudge_config, t1_blocks, turn, warmed};
use crate::fixtures::{apply, range};

/// The share of its offer a tier-1 fold must take not to stall.
const STALL_HARVEST_RATIO: f64 = 0.5;

fn ref_of(state: &State, id: &str) -> String {
    state.message_refs.by_raw[id].clone()
}

/// A baseline at 50K, then a tier-1 nudge at 60K that banks its offer.
fn fired(messages: &[Message]) -> (State, u64) {
    let config = nudge_config();
    let state = warmed(messages, &config, 50_000);
    let fired = turn(messages, state, &config, 60_000);
    assert!(fired.nudge.should_inject, "{}", fired.nudge.reason);
    assert_eq!(fired.nudge.tier, Some(1));
    let offered = fired.state.nudge.last_t1_fire_pending.unwrap_or(0);
    assert!(offered > 0);
    assert_eq!(offered, fired.nudge.breakdown.pending_t1);
    (fired.state, offered)
}

/// Folds `m0` alone with a summary of `chars` characters.
fn partial_fold(state: &State, messages: &[Message], chars: usize) -> ApplyOutcome {
    let m0 = ref_of(state, "m0");
    apply(
        &[range(&m0, &m0, &"s".repeat(chars))],
        messages,
        state,
        &nudge_config(),
    )
}

#[test]
fn a_partial_fold_stalls_tier_one_and_a_distillation_clears_it() {
    let config = nudge_config();
    let messages = make_messages(10);
    let (state, offered) = fired(&messages);

    let folded = partial_fold(&state, &messages, 48_000);
    assert!(folded.result.errors.is_empty(), "{:?}", folded.result);
    assert!((folded.result.tokens_compressed as f64) < STALL_HARVEST_RATIO * offered as f64);
    let state = folded.state;
    assert!(state.nudge.t1_harvest_stall);
    assert_eq!(state.nudge.last_t1_fire_pending, None, "consumed once");

    let escalated = turn(&messages, state, &config, 60_000);
    assert!(escalated.nudge.should_inject, "{}", escalated.nudge.reason);
    assert_eq!(escalated.nudge.tier, Some(2), "{}", escalated.nudge.reason);
    assert!(escalated
        .nudge
        .reason
        .contains("T2 distill ready: 1 tier-1 blocks"));
    assert!(escalated.nudge.reason.contains("[T1 harvest stall]"));

    let state = escalated.state;
    let first = state
        .active_blocks()
        .find(|block| block.tier == 1)
        .unwrap()
        .block_id
        .clone();
    let distilled = apply(
        &[range(&first, &first, "distilled")],
        &messages,
        &state,
        &config,
    );
    assert!(distilled.result.errors.is_empty(), "{:?}", distilled.result);
    assert!(
        !distilled.state.nudge.t1_harvest_stall,
        "a distillation clears it"
    );

    let next = turn(&messages, distilled.state, &config, 60_000);
    assert!(
        !next.nudge.reason.contains("[T1 harvest stall]"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn an_eager_fold_never_stalls() {
    let config = nudge_config();
    let messages = make_messages(10);
    let (state, offered) = fired(&messages);
    let (m0, m9) = (ref_of(&state, "m0"), ref_of(&state, "m9"));
    let folded = apply(
        &[range(&m0, &m9, &"s".repeat(48_000))],
        &messages,
        &state,
        &config,
    );
    assert!(folded.result.errors.is_empty(), "{:?}", folded.result);
    assert!(folded.result.tokens_compressed as f64 >= STALL_HARVEST_RATIO * offered as f64);
    assert!(!folded.state.nudge.t1_harvest_stall);
    let next = turn(&messages, folded.state, &config, 60_000);
    assert!(
        !next.nudge.reason.contains("[T1 harvest stall]"),
        "{}",
        next.nudge.reason
    );
}

fn stalled_with(blocks: Vec<Block>) -> State {
    let mut state = State {
        blocks,
        ..State::default()
    };
    state.nudge.t1_harvest_stall = true;
    state
}

#[test]
fn the_pressure_band_routes_to_distillation_while_tier_one_is_stalled() {
    let config = nudge_config();
    let messages = make_messages(30);
    // T1 ~137K raw against T2 5 × 2500.
    let stalled = stalled_with(t1_blocks(
        &[&["m0"], &["m1"], &["m2"], &["m3"], &["m4"]],
        10_000,
    ));
    let next = turn(&messages, stalled.clone(), &config, 92_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, Some(2), "{}", next.nudge.reason);
    assert!(next.nudge.reason.contains("OVER-LIMIT T2 distill"));
    assert!(next.nudge.reason.contains("[T1 harvest stall]"));

    let mut unstalled = stalled;
    unstalled.nudge.t1_harvest_stall = false;
    let next = turn(&messages, unstalled, &config, 92_000);
    assert_eq!(next.nudge.tier, Some(1), "{}", next.nudge.reason);
    assert!(!next.nudge.reason.contains("[T1 harvest stall]"));
}

#[test]
fn with_tiers_off_the_stall_is_never_consulted() {
    let mut config = nudge_config();
    config.tiers.enabled = false;
    let messages = make_messages(30);
    let stalled = stalled_with(t1_blocks(&[&["m0"], &["m1"]], 10_000));
    let next = turn(&messages, stalled, &config, 92_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, Some(1), "{}", next.nudge.reason);
}

#[test]
fn only_the_first_fold_after_a_nudge_is_judged() {
    let config = nudge_config();
    let messages = make_messages(10);
    let (state, _) = fired(&messages);
    let mut state = partial_fold(&state, &messages, 5000).state;
    assert!(state.nudge.t1_harvest_stall);

    state.nudge.t1_harvest_stall = false;
    state.nudge.last_t1_fire_pending = None;
    let m1 = ref_of(&state, "m1");
    let second = apply(
        &[range(&m1, &m1, &"s".repeat(5000))],
        &messages,
        &state,
        &config,
    );
    assert!(!second.state.nudge.t1_harvest_stall);
}

#[test]
fn a_stall_without_enough_distillation_mass_does_not_escalate() {
    let config = nudge_config();
    let messages = make_messages(10);
    let (state, _) = fired(&messages);
    let state = partial_fold(&state, &messages, 1000).state;
    assert!(state.nudge.t1_harvest_stall);
    let next = turn(&messages, state, &config, 60_000);
    assert_eq!(next.nudge.tier, Some(1), "{}", next.nudge.reason);
    assert!(!next.nudge.reason.contains("[T1 harvest stall]"));
}

#[test]
fn a_stall_escalates_tier_three_ahead_of_tiers_one_and_two() {
    let config = nudge_config();
    let messages = make_messages(10);
    let (state, _) = fired(&messages);
    let mut state = partial_fold(&state, &messages, 1000).state;
    assert!(state.nudge.t1_harvest_stall);

    // A tier-2 block of 10K summary tokens, consuming the tier-1 block.
    let t1 = state
        .active_blocks()
        .find(|block| block.tier == 1)
        .unwrap()
        .clone();
    state.blocks.push(Block {
        block_id: "bT3".to_owned(),
        tier: 2,
        summary: "y".repeat(40_000),
        direct_message_ids: Vec::new(),
        direct_block_ids: vec![t1.block_id.clone()],
        ..t1
    });

    let escalated = turn(&messages, state.clone(), &config, 60_000);
    assert_eq!(escalated.nudge.tier, Some(3), "{}", escalated.nudge.reason);
    assert!(escalated
        .nudge
        .reason
        .contains("T3 condense ready: 1 tier-2 blocks"));
    assert!(escalated.nudge.reason.contains("[T1 harvest stall]"));

    state.nudge.t1_harvest_stall = false;
    let unstalled = turn(&messages, state, &config, 60_000);
    assert_eq!(unstalled.nudge.tier, Some(1), "{}", unstalled.nudge.reason);
    assert!(!unstalled.nudge.reason.contains("[T1 harvest stall]"));
}
