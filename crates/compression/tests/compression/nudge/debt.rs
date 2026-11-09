// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the nudge side of `issue5415-absorb-debt-nudge.test.ts`: growth
// skipped by aborted turns, banked by the host in `absorbed_debt` and
// counted in `absorb_count`, pays the growth floor from the second abort
// on, and a repeatedly disturbed cadence drains a ready backlog.

use hanafubuki_agent_compression::{Message, NudgeDecision, Role, State};

use super::{nudge_config, turn};
use crate::fixtures::{apply, assign, range};

/// About 5500 tokens each: ten put tier-1 pending at 55K, over twice the
/// tier-1 threshold.
fn big_messages() -> Vec<Message> {
    (0..10)
        .map(|index| {
            Message::text(
                format!("m{index}"),
                Role::User,
                format!("message {index} ").repeat(2000),
            )
        })
        .collect()
}

/// Mid-session: the growth reference at 46K, no nudge outstanding.
fn seeded(debt: u64, absorb_count: u32) -> State {
    let mut state = State::default();
    state.nudge.last_per_message_nudge_tokens = 46_000;
    state.nudge.absorbed_debt = debt;
    state.nudge.absorb_count = absorb_count;
    state
}

/// Growth 500 under the floor of 5000, usage 46.5%.
fn decide(state: State) -> NudgeDecision {
    turn(&big_messages(), state, &nudge_config(), 46_500).nudge
}

#[test]
fn debt_pays_the_growth_floor() {
    let nudge = decide(seeded(4500, 2));
    assert!(nudge.should_inject, "{}", nudge.reason);
    assert_eq!(nudge.tier, Some(1));
}

#[test]
fn a_single_abort_stays_quiet() {
    let nudge = decide(seeded(9000, 1));
    assert!(!nudge.should_inject, "{}", nudge.reason);
    assert!(
        nudge
            .reason
            .contains("growth 500 + absorbed 9000 held (single abort, #5293) < floor 5000"),
        "{}",
        nudge.reason
    );
}

#[test]
fn without_debt_the_same_shape_stays_idle() {
    let nudge = decide(seeded(0, 2));
    assert!(!nudge.should_inject);
    assert!(
        nudge.reason.contains("growth 500 < floor 5000"),
        "{}",
        nudge.reason
    );
}

#[test]
fn the_debt_shows_in_the_idle_reason() {
    let messages = vec![Message::text("s0", Role::User, "small 0 ".repeat(100))];
    let nudge = turn(&messages, seeded(4100, 2), &nudge_config(), 46_500).nudge;
    assert!(!nudge.should_inject);
    assert!(
        nudge
            .reason
            .contains("growth 500 + absorbed 4100 across 2 aborts < floor 5000"),
        "{}",
        nudge.reason
    );
}

#[test]
fn repeated_aborts_with_a_ready_backlog_drain_it_now() {
    let nudge = decide(seeded(10, 2));
    assert!(nudge.should_inject, "{}", nudge.reason);
    assert_eq!(nudge.tier, Some(1));
}

#[test]
fn an_outstanding_nudge_keeps_the_drain_off() {
    let mut state = seeded(10, 2);
    state.nudge.last_nudge_shown_tokens = 46_000;
    let nudge = decide(state);
    assert!(!nudge.should_inject, "{}", nudge.reason);
}

#[test]
fn a_session_without_debt_paces_as_before() {
    let nudge = decide(seeded(0, 2));
    assert!(!nudge.should_inject);
    assert!(
        nudge.reason.contains("growth 500 < floor 5000"),
        "{}",
        nudge.reason
    );
}

#[test]
fn a_fold_resets_the_debt_and_the_abort_count() {
    let config = nudge_config();
    let messages = big_messages();
    let mut state = seeded(4500, 2);
    assign(&messages, &mut state);
    let applied = apply(
        &[range("m00001", "m00005", "first five")],
        &messages,
        &state,
        &config,
    );
    assert!(
        applied.result.errors.is_empty(),
        "{:?}",
        applied.result.errors
    );
    assert_eq!(applied.state.nudge.absorbed_debt, 0);
    assert_eq!(applied.state.nudge.absorb_count, 0);
}
