// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the growth-gate, baseline and first-sight cases of `nudge.test.ts`
// and `issue3674-nudge-low-water.test.ts`.

use std::collections::BTreeMap;

use hanafubuki_agent_compression::{ref_to_index, Config, Message, Role, State};

use super::{gated_config, make_messages, nudge_config, turn, warmed};
use crate::fixtures::{apply, range};

// nudge_config: growth step 6000, growth floor max(5000, 0.45 × 6000) = 5000,
// effective threshold 6000 (3000 with a nudge pending).

#[test]
fn the_first_request_stamps_the_baseline_and_does_not_nudge() {
    let config = nudge_config();
    let messages = make_messages(10);
    let first = turn(&messages, State::default(), &config, 10_000);
    assert!(!first.nudge.should_inject);
    assert_eq!(first.state.nudge.last_per_message_nudge_tokens, 10_000);
    assert_eq!(first.state.nudge.last_nudge_shown_tokens, 0);
}

#[test]
fn growth_over_the_floor_in_the_usage_band_nudges() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let second = turn(&messages, state, &config, 55_000);
    assert!(second.nudge.should_inject, "growth 45000, usage 55%");
    assert_eq!(second.state.nudge.last_nudge_shown_tokens, 55_000);
}

#[test]
fn growth_below_the_floor_after_a_nudge_stays_quiet() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let state = turn(&messages, state, &config, 55_000).state;
    let third = turn(&messages, state, &config, 58_000);
    assert!(!third.nudge.should_inject, "growth 3000 < floor 5000");
}

#[test]
fn a_pending_nudge_renudges_once_growth_clears_the_floor() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let state = turn(&messages, state, &config, 55_000).state;
    assert_eq!(state.nudge.last_nudge_shown_tokens, 55_000);
    let third = turn(&messages, state, &config, 60_500);
    assert!(third.nudge.should_inject, "growth 5500 >= floor 5000");
}

#[test]
fn the_growth_floor_holds_back_growth_over_the_step() {
    let mut config = nudge_config();
    config.nudge.min_growth_floor = 10_000;
    config.nudge.min_growth_ratio = 0.9;
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let second = turn(&messages, state, &config, 17_000);
    assert!(
        !second.nudge.should_inject,
        "growth 7000 >= step 6000 but < floor 10000"
    );
}

#[test]
fn the_emergency_band_nudges_whatever_the_growth() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let second = turn(&messages, state, &config, 99_000);
    assert!(second.nudge.should_inject);
    assert!(
        second.nudge.reason.contains("EMERGENCY"),
        "{}",
        second.nudge.reason
    );
}

#[test]
fn the_baseline_and_the_shown_reference_follow_any_drop() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let state = turn(&messages, state, &config, 55_000).state;
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 10_000);
    let third = turn(&messages, state, &config, 3000);
    assert_eq!(third.state.nudge.last_per_message_nudge_tokens, 3000);
    assert_eq!(third.state.nudge.last_nudge_shown_tokens, 3000);
}

#[test]
fn the_baseline_follows_a_drop_shorter_than_one_step() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 9210);
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 9210);
    let state = turn(&messages, state, &config, 4000).state;
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 4000);
    let climbing = turn(&messages, state, &config, 6300);
    assert_eq!(climbing.state.nudge.last_per_message_nudge_tokens, 4000);
}

#[test]
fn a_fold_clears_the_baseline_so_no_feedback_loop_follows() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let state = turn(&messages, state, &config, 55_000).state;
    let state = apply(
        &[range("m00001", "m00003", "compressed early msgs")],
        &messages,
        &state,
        &config,
    )
    .state;
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 0);
    assert_eq!(state.nudge.last_nudge_shown_tokens, 0);

    let next = turn(&messages, state, &config, 30_000);
    assert!(!next.nudge.should_inject, "30% < 45%");
    assert_eq!(next.state.nudge.last_per_message_nudge_tokens, 30_000);
}

#[test]
fn a_full_cycle_nudges_folds_restamps_and_nudges_again() {
    let config = nudge_config();
    let messages = make_messages(20);
    let state = warmed(&messages, &config, 10_000);
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 10_000);
    let nudged = turn(&messages, state, &config, 55_000);
    assert!(nudged.nudge.should_inject);
    let state = apply(
        &[range("m00001", "m00005", "compressed batch")],
        &messages,
        &nudged.state,
        &config,
    )
    .state;
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 0);

    let quiet = turn(&messages, state, &config, 30_000);
    assert!(!quiet.nudge.should_inject);
    assert_eq!(quiet.state.nudge.last_per_message_nudge_tokens, 30_000);
    let again = turn(&messages, quiet.state, &config, 55_000);
    assert!(again.nudge.should_inject, "growth 25000, usage 55%");
}

#[test]
fn the_recent_zone_is_not_offered() {
    let mut config = nudge_config();
    config.preserve_recent_messages = 5;
    let messages = make_messages(15);
    let state = warmed(&messages, &config, 10_000);
    let next = turn(&messages, state, &config, 55_000);
    assert!(next.nudge.should_inject);
    assert!(!next.nudge.compressible_ranges.is_empty());
    for offered in &next.nudge.compressible_ranges {
        assert!(!offered.start_ref.starts_with("m0001"), "{offered:?}");
    }
}

#[test]
fn covered_messages_are_not_offered() {
    let mut config = nudge_config();
    config.preserve_recent_messages = 2;
    let messages = make_messages(12);
    let state = warmed(&messages, &config, 10_000);
    let state = apply(
        &[range("m00001", "m00005", "covered")],
        &messages,
        &state,
        &config,
    )
    .state;
    let next = turn(&messages, state, &config, 55_000);
    assert!(!next.nudge.compressible_ranges.is_empty());
    for offered in &next.nudge.compressible_ranges {
        assert!(ref_to_index(&offered.end_ref).unwrap() <= messages.len() as u32);
        assert!(ref_to_index(&offered.start_ref).unwrap() > 5, "{offered:?}");
    }
}

#[test]
fn a_scale_drop_lowers_the_shown_reference_and_the_tier_stamps() {
    let config = nudge_config();
    let messages = make_messages(10);
    let mut state = State::default();
    state.nudge.last_per_message_nudge_tokens = 366_000;
    state.nudge.last_nudge_shown_tokens = 366_000;
    state.nudge.last_shown_by_tier = BTreeMap::from([(1, 366_000), (2, 366_000)]);

    let dropped = turn(&messages, state, &config, 40_000);
    let stamped = &dropped.state.nudge;
    assert_eq!(stamped.last_per_message_nudge_tokens, 40_000);
    assert_eq!(stamped.last_nudge_shown_tokens, 40_000);
    assert_eq!(
        stamped.last_shown_by_tier,
        BTreeMap::from([(1, 40_000), (2, 40_000)])
    );

    // One floor above the lowered stamps the tier gates open again.
    let climb = turn(&messages, dropped.state, &config, 45_000);
    assert!(climb.nudge.should_inject, "{}", climb.nudge.reason);
    assert_eq!(climb.state.nudge.last_shown_by_tier[&1], 45_000);
}

#[test]
fn a_settle_below_the_shown_reference_rearms_the_cadence() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let state = turn(&messages, state, &config, 55_000).state;
    assert_eq!(state.nudge.last_nudge_shown_tokens, 55_000);

    // Settled far below the shown reference but above baseline − step.
    let settled = turn(&messages, state, &config, 9000);
    assert_eq!(settled.state.nudge.last_per_message_nudge_tokens, 9000);
    assert_eq!(settled.state.nudge.last_nudge_shown_tokens, 9000);
    assert_eq!(settled.state.nudge.last_shown_by_tier[&1], 9000);

    let regrown = turn(&messages, settled.state, &config, 15_000);
    assert!(regrown.nudge.should_inject, "{}", regrown.nudge.reason);
}

// First-sight mass: a session arriving with a ready backlog must not wait a
// whole floor of new tokens for its first nudge.

#[test]
fn first_sight_mass_nudges_on_a_big_ingest_without_growth() {
    let config = nudge_config();
    let messages = make_messages(10);
    let first = turn(&messages, State::default(), &config, 47_000);
    assert!(first.nudge.should_inject);
    assert!(first.nudge.reason.contains("[first-sight mass]"));
}

#[test]
fn first_sight_mass_waits_below_the_usage_band() {
    let config = nudge_config();
    let messages = make_messages(10);
    let first = turn(&messages, State::default(), &config, 44_000);
    assert!(!first.nudge.should_inject, "44% < 45%");
}

fn high_floor_config() -> Config {
    let mut config = nudge_config();
    config.nudge.min_growth_floor = 10_000;
    config.nudge.min_growth_ratio = 0.9;
    config
}

#[test]
fn first_sight_mass_paces_normally_after_its_nudge() {
    let config = high_floor_config();
    let messages = make_messages(10);
    let first = turn(&messages, State::default(), &config, 46_000);
    assert!(first.nudge.should_inject);
    assert!(first.nudge.reason.contains("[first-sight mass]"));
    let second = turn(&messages, first.state, &config, 50_000);
    assert!(!second.nudge.should_inject, "growth 4000 < floor 10000");
}

#[test]
fn first_sight_mass_rearms_after_a_fold_while_in_the_band() {
    let config = high_floor_config();
    let messages = make_messages(20);
    let first = turn(&messages, State::default(), &config, 47_000);
    assert!(first.nudge.should_inject);
    assert!(first.nudge.reason.contains("[first-sight mass]"));
    let state = apply(
        &[range("m00001", "m00005", "drained a slice of the backlog")],
        &messages,
        &first.state,
        &config,
    )
    .state;
    assert_eq!(state.nudge.last_nudge_shown_tokens, 0);
    let second = turn(&messages, state, &config, 48_000);
    assert!(second.nudge.should_inject, "still in band with ready mass");
    assert!(second.nudge.reason.contains("[first-sight mass]"));
}

#[test]
fn first_sight_mass_falls_silent_below_the_band() {
    let config = nudge_config();
    let messages = make_messages(20);
    let first = turn(&messages, State::default(), &config, 55_000);
    assert!(first.nudge.should_inject);
    let state = apply(
        &[range("m00001", "m00005", "drained most of the backlog")],
        &messages,
        &first.state,
        &config,
    )
    .state;
    let second = turn(&messages, state, &config, 20_000);
    assert!(!second.nudge.should_inject, "20% < 45%");
}

#[test]
fn a_tail_below_the_minimum_range_joins_the_batch_before_it() {
    let config = gated_config();
    // Ten 800-character messages: [m0..m7] clears 5000 characters, the
    // 1600-character tail joins it, so one range m0..m9 is offered.
    let messages: Vec<_> = (0..10)
        .map(|index| {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            Message::text(format!("m{index}"), role, "x".repeat(800))
        })
        .collect();
    let state = warmed(&messages, &config, 10_000);
    let next = turn(&messages, state, &config, 35_000);
    assert_eq!(next.nudge.breakdown.pending_t1, 2000);
    assert_eq!(next.nudge.compressible_ranges.len(), 1);
    assert_eq!(next.nudge.compressible_ranges[0].tokens, 2000);
}

// issue3674-nudge-low-water.test.ts: the reporter's numbers (872K window,
// flat 50K step, floor 22500).

fn low_water_config() -> Config {
    let mut config = nudge_config();
    config.model_context_limit = 872_000;
    config.nudge.max_context_limit_pct = 0.75;
    config.nudge.min_context_limit_pct = 0.05;
    config.nudge.growth_ratio = 0.06;
    config.nudge.growth_floor = 50_000;
    config.nudge.growth_cap = 50_000;
    config.nudge.min_growth_floor = 22_500;
    config
}

fn low_water_messages(count: usize) -> Vec<Message> {
    let mut messages = make_messages(count);
    for (index, message) in messages.iter_mut().enumerate() {
        message.text = format!("message {index} ").repeat(4000);
    }
    messages
}

#[test]
fn the_reference_follows_the_settle_and_growth_rearms_from_the_valley() {
    let config = low_water_config();
    let messages = low_water_messages(40);
    let state = warmed(&messages, &config, 650_092);
    let state = apply(
        &[range("m00001", "m00003", "folded")],
        &messages,
        &state,
        &config,
    )
    .state;
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 0);

    let state = turn(&messages, state, &config, 92_106).state;
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 92_106);
    let state = turn(&messages, state, &config, 44_038).state;
    assert_eq!(state.nudge.last_per_message_nudge_tokens, 44_038);

    let regrown = turn(&messages, state, &config, 67_967);
    assert!(regrown.nudge.should_inject, "{}", regrown.nudge.reason);
    assert_eq!(regrown.nudge.tier, Some(1));
}

#[test]
fn a_drop_short_of_one_step_no_longer_dead_zones_the_cadence() {
    let config = low_water_config();
    let messages = low_water_messages(24);
    let state = warmed(&messages, &config, 60_000);
    let state = turn(&messages, state, &config, 115_000).state;
    assert!(state.nudge.last_nudge_shown_tokens > 0);
    let phantom = state.nudge.last_nudge_shown_tokens;

    let state = turn(&messages, state, &config, phantom - 49_000).state;
    assert_eq!(state.nudge.last_nudge_shown_tokens, phantom - 49_000);
    let regrown = turn(&messages, state, &config, phantom - 49_000 + 23_000);
    assert!(regrown.nudge.should_inject, "{}", regrown.nudge.reason);
}
