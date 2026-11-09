// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// The full reason texts of scenarios from `nudge.test.ts`,
// `nudge-pressure-benefit.test.ts`, `nudge-pressure-no-sample-hold.test.ts`
// and `issue5698-t1-harvest-stall.test.ts`, as the kernel writes them (the
// kernel's tests match parts of them; these were taken from the kernel run
// on the same inputs).

use hanafubuki_agent_compression::{Message, Role, State, TokenCountSource};

use super::{block, gated_config, make_messages, nudge_config, t1_blocks, turn, turn_with, warmed};
use crate::fixtures::ids;

#[test]
fn a_count_ready_tier_two_and_its_cadence() {
    let mut config = nudge_config();
    config.preserve_recent_messages = 30;
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = t1_blocks(&[&["m1"], &["m2"], &["m3"], &["m4"], &["m5"]], 400);
    let first = turn(&messages, state, &config, 60_000);
    assert_eq!(
        first.nudge.reason,
        "T2 distill ready: 5 tier-1 blocks >= tier2Trigger 5 (500 tokens), usage 60%"
    );
    let second = turn(&messages, first.state, &config, 61_000);
    assert_eq!(
        second.nudge.reason,
        "max compressible 500 < threshold 6000; growth 1000 < floor 5000, ready: T2 5 blocks (count), blocked: T2 (cadence)"
    );
}

#[test]
fn tier_one_and_its_cadence() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let nudged = turn(&messages, state, &config, 55_000);
    assert_eq!(
        nudged.nudge.reason,
        "T1 effective 50000 >= 6000, growth 45000, usage 55%"
    );
    let held = turn(&messages, nudged.state, &config, 58_000);
    assert_eq!(
        held.nudge.reason,
        "growth 3000 < floor 5000, ready: T1 50000, blocked: T1 (cadence)"
    );
}

#[test]
fn an_emergency_with_nothing_effective() {
    let mut config = gated_config();
    config.nudge.emergency_threshold_pct = 0.95;
    config.nudge.max_context_limit_pct = 0.75;
    let messages: Vec<Message> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(index, word)| {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            Message::text(
                format!("m{index}"),
                role,
                format!("tiny {word} ").repeat(10),
            )
        })
        .collect();
    let next = turn(&messages, State::default(), &config, 96_000);
    assert_eq!(
        next.nudge.reason,
        "EMERGENCY: usage 96% but no tier has effective compressible content (T1 effective 0, T2 0, T3 0) — nudge suppressed to avoid offering ranges below minCompressRange"
    );
}

#[test]
fn an_estimate_held_over_the_limit() {
    let next = turn_with(
        &make_messages(10),
        State::default(),
        &nudge_config(),
        95_000,
        Some(TokenCountSource::Estimate),
    );
    assert_eq!(
        next.nudge.reason,
        "OVER-LIMIT: usage 95% but the meter reading is estimate-grade (no billing sample landed this cycle, max pending 50000) — pressure injection held (estimate round 1 of 2) until a usage-grade reading decides or the estimate-hold backstop elapses; hard-overflow backstop unaffected"
    );
}

#[test]
fn an_emergency_below_the_benefit_floor() {
    let config = nudge_config();
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 10_000);
    state.blocks = vec![block("b1", 1, 928, &ids(&messages))];
    let next = turn(&messages, state, &config, 99_000);
    assert_eq!(
        next.nudge.reason,
        "EMERGENCY: usage 99% but max pending 232 < min benefit 5000 tokens (T1 effective 0, T2 232, T3 0) — suppressed: rewriting below the benefit floor reclaims almost nothing while usage stays high; truncate.threshold remains the safety valve"
    );
}

#[test]
fn tier_two_by_mass() {
    let mut config = gated_config();
    config.preserve_recent_messages = 9;
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = t1_blocks(&[&["m1"], &["m2"]], 24_000);
    let next = turn(&messages, state, &config, 60_000);
    assert_eq!(
        next.nudge.reason,
        "T2 distill ready: 2 tier-1 blocks (12000 tokens) >= 9000 (1.5x) and > T1 effective 0, usage 60%"
    );
}

#[test]
fn first_sight_mass() {
    let next = turn(
        &make_messages(10),
        State::default(),
        &nudge_config(),
        47_000,
    );
    assert_eq!(
        next.nudge.reason,
        "T1 effective 50000 >= 6000, growth 0, usage 47% [first-sight mass]"
    );
}

#[test]
fn a_stalled_tier_one_under_pressure() {
    let mut state = State {
        blocks: t1_blocks(&[&["m0"], &["m1"], &["m2"], &["m3"], &["m4"]], 10_000),
        ..State::default()
    };
    state.nudge.t1_harvest_stall = true;
    let next = turn(&make_messages(30), state, &nudge_config(), 92_000);
    assert_eq!(
        next.nudge.reason,
        "OVER-LIMIT T2 distill: max pending 12500 (T1 effective 135000, T2 12500, T3 0), usage 92% [first-sight mass] [T1 harvest stall]"
    );
}
