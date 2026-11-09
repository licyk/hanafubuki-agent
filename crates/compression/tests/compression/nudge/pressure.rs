// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `nudge-pressure-benefit.test.ts`,
// `nudge-pressure-no-sample-hold.test.ts` and
// `nudge-pressure-estimate-hold-backstop.test.ts`.

use hanafubuki_agent_compression::{Block, Config, Message, State, TokenCountSource, TurnOutput};

use super::{block, make_messages, nudge_config, t2_blocks, turn, turn_with, warmed};
use crate::fixtures::ids;

// The minimum benefit: the pressure band nudges only for at least
// max(5000, 1% of the window) pending tokens, so a micro rewrite cannot
// reset the baselines and re-arm the band every request.

/// A tier-1 block over every message: tier-1 effective pending 0, its
/// summary the only (tier-2) pending mass.
fn cover_all(messages: &[Message], summary_chars: usize) -> Block {
    block("b1", 1, summary_chars, &ids(messages))
}

fn covered(config: &Config, warm_tokens: u64, summary_chars: usize) -> (Vec<Message>, State) {
    let messages = make_messages(10);
    let mut state = warmed(&messages, config, warm_tokens);
    state.blocks = vec![cover_all(&messages, summary_chars)];
    (messages, state)
}

#[test]
fn micro_tier_two_pending_in_the_emergency_band_is_held_back() {
    let config = nudge_config();
    // 928 characters: 232 tokens, as in production.
    let (messages, state) = covered(&config, 10_000, 928);
    let held = turn(&messages, state, &config, 99_000);
    assert!(held.nudge.breakdown.emergency_override);
    assert!(!held.nudge.should_inject);
    assert_eq!(held.nudge.breakdown.min_pressure_benefit, 5000);
    let reason = &held.nudge.reason;
    assert!(reason.contains("EMERGENCY"), "{reason}");
    assert!(reason.contains("232"), "{reason}");
    assert!(reason.contains("suppressed"), "{reason}");

    let again = turn(&messages, held.state, &config, 99_500);
    assert!(!again.nudge.should_inject, "no re-arm loop");
    assert_eq!(again.state.nudge.last_nudge_shown_tokens, 0);
}

#[test]
fn micro_tier_three_pending_in_the_emergency_band_is_held_back() {
    let mut config = nudge_config();
    config.preserve_recent_messages = 10;
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 10_000);
    // 480 characters: 120 tokens.
    state.blocks = t2_blocks(1, 480, &ids(&messages));
    let next = turn(&messages, state, &config, 99_000);
    assert!(next.nudge.breakdown.emergency_override);
    assert!(!next.nudge.should_inject);
    assert!(
        next.nudge.reason.contains("EMERGENCY"),
        "{}",
        next.nudge.reason
    );
    assert!(next.nudge.reason.contains("120"), "{}", next.nudge.reason);
}

#[test]
fn pending_exactly_at_the_benefit_floor_still_nudges() {
    let config = nudge_config();
    let (messages, state) = covered(&config, 10_000, 20_000);
    let next = turn(&messages, state, &config, 99_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(2));
    assert!(
        next.nudge
            .reason
            .contains("EMERGENCY T2 distill: max pending 5000"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn the_benefit_floor_scales_with_the_window() {
    let mut config = nudge_config();
    config.model_context_limit = 1_000_000;
    // 6000 tokens < 1% of a 1M window.
    let (messages, state) = covered(&config, 100_000, 24_000);
    let held = turn(&messages, state, &config, 990_000);
    assert!(!held.nudge.should_inject);
    assert_eq!(held.nudge.breakdown.min_pressure_benefit, 10_000);

    let mut state = held.state;
    state.blocks = vec![cover_all(&messages, 40_000)];
    let next = turn(&messages, state, &config, 990_000);
    assert!(next.nudge.should_inject, "10000 >= 10000");
    assert_eq!(next.nudge.tier, Some(2));
}

#[test]
fn an_explicit_minimum_benefit_lowers_the_gate() {
    let mut config = nudge_config();
    config.nudge.min_pressure_benefit_tokens = Some(100);
    let (messages, state) = covered(&config, 10_000, 928);
    let next = turn(&messages, state, &config, 99_000);
    assert_eq!(next.nudge.breakdown.min_pressure_benefit, 100);
    assert!(next.nudge.should_inject, "232 >= 100");
    assert_eq!(next.nudge.tier, Some(2));
}

#[test]
fn a_zero_minimum_benefit_nudges_for_any_pending() {
    let mut config = nudge_config();
    config.nudge.min_pressure_benefit_tokens = Some(0);
    let (messages, state) = covered(&config, 10_000, 928);
    let next = turn(&messages, state, &config, 99_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(2));
}

// The phantom-pressure guard: an estimated count holds the pressure band
// back for at most `max_estimate_hold_rounds` consecutive requests.

/// One request on a fresh state or on `previous`, about 55K tier-1 pending.
fn pressure_turn(
    previous: Option<State>,
    tokens: u64,
    source: Option<TokenCountSource>,
    config: &Config,
) -> TurnOutput {
    turn_with(
        &make_messages(10),
        previous.unwrap_or_default(),
        config,
        tokens,
        source,
    )
}

const ESTIMATE: Option<TokenCountSource> = Some(TokenCountSource::Estimate);
const USAGE: Option<TokenCountSource> = Some(TokenCountSource::Usage);

#[test]
fn an_estimate_holds_the_emergency_band_back() {
    let next = pressure_turn(None, 99_000, ESTIMATE, &nudge_config());
    assert!(!next.nudge.should_inject);
    let reason = &next.nudge.reason;
    assert!(reason.contains("estimate-grade"), "{reason}");
    assert!(reason.contains("estimate round 1 of 2"), "{reason}");
    assert!(reason.starts_with("EMERGENCY:"), "{reason}");
    assert!(next.nudge.breakdown.pressure_deferred_no_sample);
    assert!(next.nudge.breakdown.emergency_override);
}

#[test]
fn an_estimate_holds_the_over_limit_band_back_too() {
    let next = pressure_turn(None, 95_000, ESTIMATE, &nudge_config());
    assert!(!next.nudge.should_inject);
    assert!(
        next.nudge.reason.starts_with("OVER-LIMIT:"),
        "{}",
        next.nudge.reason
    );
    assert!(next.nudge.reason.contains("estimate round 1 of 2"));
    assert!(next.nudge.breakdown.pressure_deferred_no_sample);
}

#[test]
fn a_measured_count_nudges_under_pressure() {
    let next = pressure_turn(None, 99_000, USAGE, &nudge_config());
    assert!(next.nudge.should_inject);
    assert!(
        next.nudge.reason.starts_with("EMERGENCY"),
        "{}",
        next.nudge.reason
    );
    assert!(!next.nudge.breakdown.pressure_deferred_no_sample);
}

#[test]
fn an_unknown_source_nudges_under_pressure() {
    let next = pressure_turn(None, 99_000, None, &nudge_config());
    assert!(next.nudge.should_inject);
    assert!(!next.nudge.breakdown.pressure_deferred_no_sample);
}

#[test]
fn without_the_guard_an_estimate_nudges_under_pressure() {
    let mut config = nudge_config();
    config.nudge.require_usage_sample_for_pressure = false;
    let next = pressure_turn(None, 99_000, ESTIMATE, &config);
    assert!(next.nudge.should_inject);
    assert!(
        next.nudge.reason.starts_with("EMERGENCY"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn outside_the_pressure_band_the_source_changes_nothing() {
    let config = nudge_config();
    let estimated = pressure_turn(None, 52_000, ESTIMATE, &config);
    let measured = pressure_turn(None, 52_000, USAGE, &config);
    assert_eq!(
        (
            estimated.nudge.should_inject,
            estimated.nudge.tier,
            &estimated.nudge.reason
        ),
        (
            measured.nudge.should_inject,
            measured.nudge.tier,
            &measured.nudge.reason
        )
    );
    assert!(!estimated.nudge.breakdown.emergency_override);
    assert!(!estimated.nudge.breakdown.pressure_deferred_no_sample);
}

#[test]
fn a_backend_that_never_reports_is_nudged_on_the_third_estimate() {
    let config = nudge_config();
    let first = pressure_turn(None, 99_000, ESTIMATE, &config);
    assert!(!first.nudge.should_inject);
    assert!(first.nudge.breakdown.pressure_deferred_no_sample);
    assert!(first.nudge.reason.contains("estimate round 1 of 2"));
    let second = pressure_turn(Some(first.state), 99_000, ESTIMATE, &config);
    assert!(!second.nudge.should_inject);
    assert!(second.nudge.breakdown.pressure_deferred_no_sample);
    assert!(second.nudge.reason.contains("estimate round 2 of 2"));
    let third = pressure_turn(Some(second.state), 99_000, ESTIMATE, &config);
    assert!(third.nudge.should_inject);
    assert!(
        third.nudge.reason.starts_with("EMERGENCY"),
        "{}",
        third.nudge.reason
    );
    assert!(!third.nudge.breakdown.pressure_deferred_no_sample);
}

#[test]
fn a_measured_count_resets_the_estimate_streak() {
    let config = nudge_config();
    let first = pressure_turn(None, 99_000, ESTIMATE, &config);
    assert_eq!(first.state.nudge.estimate_hold_streak, 1);
    let measured = pressure_turn(Some(first.state), 99_000, USAGE, &config);
    assert!(measured.nudge.should_inject);
    assert_eq!(measured.state.nudge.estimate_hold_streak, 0);
    let again = pressure_turn(Some(measured.state), 99_000, ESTIMATE, &config);
    assert!(!again.nudge.should_inject);
    assert_eq!(again.state.nudge.estimate_hold_streak, 1);
    assert!(again.nudge.reason.contains("estimate round 1 of 2"));
}

#[test]
fn zero_hold_rounds_nudge_on_the_first_estimate() {
    let mut config = nudge_config();
    config.nudge.max_estimate_hold_rounds = 0;
    let next = pressure_turn(None, 99_000, ESTIMATE, &config);
    assert!(next.nudge.should_inject);
    assert!(
        next.nudge.reason.starts_with("EMERGENCY"),
        "{}",
        next.nudge.reason
    );
    assert!(!next.nudge.breakdown.pressure_deferred_no_sample);
}

#[test]
fn more_hold_rounds_widen_the_window() {
    let mut config = nudge_config();
    config.nudge.max_estimate_hold_rounds = 4;
    let mut previous = None;
    for round in 1..=4 {
        let held = pressure_turn(previous, 99_000, ESTIMATE, &config);
        assert!(!held.nudge.should_inject, "round {round} stays held");
        assert!(held.nudge.breakdown.pressure_deferred_no_sample);
        previous = Some(held.state);
    }
    let fifth = pressure_turn(previous, 99_000, ESTIMATE, &config);
    assert!(fifth.nudge.should_inject);
    assert!(!fifth.nudge.breakdown.pressure_deferred_no_sample);
}

#[test]
fn an_unknown_source_never_touches_the_streak() {
    let config = nudge_config();
    let first = pressure_turn(None, 99_000, None, &config);
    assert!(first.nudge.should_inject);
    assert!(!first.nudge.breakdown.pressure_deferred_no_sample);
    assert_eq!(first.state.nudge.estimate_hold_streak, 0);
    let second = pressure_turn(Some(first.state), 99_000, None, &config);
    assert!(second.nudge.should_inject);
    assert_eq!(second.state.nudge.estimate_hold_streak, 0);
}
