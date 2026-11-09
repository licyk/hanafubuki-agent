// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the tier-arbitration cases of `nudge.test.ts`: tier 1 first, the
// mass and count paths of tiers 2 and 3, and the pressure band's argmax.

use hanafubuki_agent_compression::{Block, Config, Message, Role, State};

use super::{block, gated_config, make_messages, nudge_config, t1_blocks, t2_blocks, turn, warmed};
use crate::fixtures::ids;

#[test]
fn ready_tier_one_mass_is_nudged_as_tier_one() {
    let config = gated_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let next = turn(&messages, state, &config, 55_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(1), "T1 effective ~50K >= 6000");
}

#[test]
fn tier_two_over_its_threshold_and_tier_one_is_nudged_as_tier_two() {
    let mut config = gated_config();
    config.preserve_recent_messages = 9;
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 50_000);
    // Two tier-1 blocks of ~6K-token summaries: T2 ~12K >= 9000.
    state.blocks = t1_blocks(&[&["m1"], &["m2"]], 24_000);
    let next = turn(&messages, state, &config, 60_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(2));
}

#[test]
fn ready_tier_one_wins_over_a_larger_tier_two() {
    let config = gated_config();
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 10_000);
    state.blocks = t1_blocks(&[&["m0"], &["m1"]], 40_000);
    let next = turn(&messages, state, &config, 55_000);
    assert_eq!(next.nudge.tier, Some(1));
}

fn cover_all(messages: &[Message], summary_chars: usize) -> Block {
    Block {
        compressed_tokens: 50_000,
        ..block("b1", 1, summary_chars, &ids(messages))
    }
}

#[test]
fn the_emergency_band_picks_tier_two_when_it_pends_the_most() {
    let config = nudge_config();
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 10_000);
    // One block over everything: T1 effective 0, T2 ~10K.
    state.blocks = vec![cover_all(&messages, 40_000)];
    let next = turn(&messages, state, &config, 99_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(2));
    assert!(next.nudge.breakdown.emergency_override);
}

#[test]
fn the_emergency_band_picks_tier_one_when_it_pends_the_most() {
    let config = nudge_config();
    let messages = make_messages(10);
    let state = warmed(&messages, &config, 10_000);
    let next = turn(&messages, state, &config, 99_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(1));
    assert!(next.nudge.breakdown.emergency_override);
}

#[test]
fn tier_two_between_the_step_and_its_threshold_stays_quiet() {
    let mut config = gated_config();
    config.preserve_recent_messages = 10;
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 10_000);
    // T2 7500 ∈ [6000, 9000), T1 effective 0.
    state.blocks = t1_blocks(&[&["m0"]], 30_000);
    let next = turn(&messages, state, &config, 35_000);
    assert!(!next.nudge.should_inject, "{}", next.nudge.reason);
}

#[test]
fn tier_three_over_its_threshold_and_the_lower_tiers_is_nudged_as_tier_three() {
    let mut config = gated_config();
    config.preserve_recent_messages = 10;
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 10_000);
    // T2 1000 (one small tier-1 block); T3 2 × 5000 = 10000 >= 9000.
    state.blocks = t1_blocks(&[&["m0"]], 4000);
    state.blocks.extend(t2_blocks(2, 20_000, &["m0"]));
    let next = turn(&messages, state, &config, 35_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(3));
    assert!(
        next.nudge.reason.contains("T3 condense"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn the_emergency_argmax_picks_tier_three_when_it_pends_the_most() {
    let config = gated_config();
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 10_000);
    // T1 effective 0; T2 5000; T3 3 × 5000 = 15000.
    let all = ids(&messages);
    state.blocks = vec![Block {
        block_id: "cover".to_owned(),
        ..cover_all(&messages, 20_000)
    }];
    state.blocks.extend(t2_blocks(3, 20_000, &all));
    let next = turn(&messages, state, &config, 99_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(3));
    assert!(next.nudge.breakdown.emergency_override);
}

fn pressure_config() -> Config {
    let mut config = gated_config();
    config.nudge.max_context_limit_pct = 0.75;
    config
}

#[test]
fn over_the_limit_with_compressible_content_nudges() {
    let config = pressure_config();
    let messages = make_messages(10);
    let next = turn(&messages, State::default(), &config, 80_000);
    assert!(next.nudge.should_inject);
    assert!(next.nudge.breakdown.over_limit);
    assert!(
        next.nudge.reason.contains("OVER-LIMIT"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn over_the_limit_with_nothing_compressible_does_not_nudge() {
    let mut config = pressure_config();
    config.preserve_recent_messages = 5;
    let messages = vec![
        Message::text("a", Role::User, "hello"),
        Message::text("b", Role::Assistant, "world"),
    ];
    let next = turn(&messages, State::default(), &config, 80_000);
    assert!(!next.nudge.should_inject);
    assert!(next.nudge.breakdown.over_limit);
    assert!(
        next.nudge.reason.contains("OVER-LIMIT"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn the_emergency_band_holds_back_ranges_below_the_minimum_range() {
    let mut config = pressure_config();
    config.nudge.emergency_threshold_pct = 0.95;
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
    assert!(!next.nudge.should_inject);
    assert!(next.nudge.breakdown.emergency_override);
    assert!(
        next.nudge.reason.contains("EMERGENCY"),
        "{}",
        next.nudge.reason
    );
    assert!(
        next.nudge.reason.contains("minCompressRange"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn the_emergency_band_with_effective_pending_routes_to_the_largest_tier() {
    let mut config = pressure_config();
    config.nudge.emergency_threshold_pct = 0.95;
    let messages = make_messages(10);
    let next = turn(&messages, State::default(), &config, 96_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(1));
    assert!(
        next.nudge.reason.contains("EMERGENCY"),
        "{}",
        next.nudge.reason
    );
}

// The count path: tier 2 or 3 on the number of active lower-tier blocks.

fn preserved_config() -> Config {
    let mut config = nudge_config();
    config.preserve_recent_messages = 30;
    config
}

fn five_blocks() -> Vec<Block> {
    t1_blocks(&[&["m1"], &["m2"], &["m3"], &["m4"], &["m5"]], 400)
}

#[test]
fn five_summary_blocks_stay_silent_under_the_default_triggers() {
    let mut config = preserved_config();
    config.tiers.tier2_trigger = 1000;
    config.tiers.tier3_trigger = 2000;
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = five_blocks();
    let next = turn(&messages, state, &config, 40_000);
    assert!(!next.nudge.should_inject, "{}", next.nudge.reason);
    assert!(!next.nudge.reason.contains("T2 distill ready"));
    assert!(
        !next.nudge.reason.contains("blocks (count)"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn a_configured_count_trigger_fires_tier_three_below_the_usage_band() {
    let mut config = preserved_config();
    config.tiers.tier2_trigger = 2;
    config.tiers.tier3_trigger = 3;
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 25_000);
    state.blocks = five_blocks()
        .into_iter()
        .map(|block| Block { tier: 2, ..block })
        .collect();
    let next = turn(&messages, state, &config, 40_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, Some(3));
    assert!(
        next.nudge
            .reason
            .contains("T3 condense ready: 5 tier-2 blocks >= tier3Trigger 3"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn tier_two_fires_on_the_block_count_with_small_summaries() {
    let config = preserved_config();
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = five_blocks();
    let next = turn(&messages, state, &config, 60_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, Some(2));
    assert!(
        next.nudge
            .reason
            .contains("5 tier-1 blocks >= tier2Trigger 5"),
        "{}",
        next.nudge.reason
    );
    assert_eq!(next.nudge.tier_target_blocks.len(), 5);
}

#[test]
fn below_the_count_and_the_mass_tier_two_stays_quiet() {
    let config = preserved_config();
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = t1_blocks(&[&["m1"], &["m2"], &["m3"], &["m4"]], 400);
    let next = turn(&messages, state, &config, 60_000);
    assert!(!next.nudge.should_inject, "{}", next.nudge.reason);
    assert!(
        next.nudge
            .reason
            .contains("max compressible 400 < threshold 6000"),
        "{}",
        next.nudge.reason
    );
    assert!(!next.nudge.reason.contains("tier2Trigger"));
}

#[test]
fn a_count_ready_tier_two_does_not_preempt_ready_tier_one() {
    let config = gated_config();
    let messages = make_messages(10);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = five_blocks();
    let next = turn(&messages, state, &config, 60_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, Some(1));
    assert!(
        next.nudge.reason.contains("T1 effective"),
        "{}",
        next.nudge.reason
    );
}

#[test]
fn a_count_ready_tier_two_keeps_its_cadence() {
    let config = preserved_config();
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = five_blocks();
    let first = turn(&messages, state, &config, 60_000);
    assert_eq!(first.nudge.tier, Some(2), "{}", first.nudge.reason);
    let second = turn(&messages, first.state, &config, 61_000);
    assert!(!second.nudge.should_inject, "{}", second.nudge.reason);
    assert!(
        second.nudge.reason.contains("T2 (cadence)"),
        "{}",
        second.nudge.reason
    );
}

#[test]
fn tier_three_fires_on_the_block_count_with_small_summaries() {
    let config = preserved_config();
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = t1_blocks(&[&["m1"], &["m2"]], 400);
    state.blocks.extend(t2_blocks(10, 400, &["m3"]));
    let next = turn(&messages, state, &config, 60_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, Some(3));
    assert!(
        next.nudge
            .reason
            .contains("10 tier-2 blocks >= tier3Trigger 10"),
        "{}",
        next.nudge.reason
    );
    assert_eq!(next.nudge.tier_target_blocks.len(), 10);
}

#[test]
fn with_tiers_off_the_count_trigger_cannot_fire() {
    let mut config = preserved_config();
    config.tiers.enabled = false;
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    state.blocks = t1_blocks(
        &[
            &["m1"],
            &["m2"],
            &["m3"],
            &["m4"],
            &["m5"],
            &["m6"],
            &["m7"],
        ],
        400,
    );
    let next = turn(&messages, state, &config, 60_000);
    assert!(!next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, None);
    assert!(!next.nudge.reason.contains("tier2Trigger"));
}

#[test]
fn tier_two_fires_once_growth_clears_the_floor() {
    let mut config = preserved_config();
    config.tiers.tier2_trigger = 3;
    let messages = make_messages(30);
    let mut state = warmed(&messages, &config, 50_000);
    let substantial = "tier-1 summary block {} with substantial content ";
    state.blocks = (0..3)
        .map(|index| Block {
            summary: substantial.replace("{}", &index.to_string()).repeat(1200),
            compressed_tokens: 5000,
            ..block(&format!("b{}", index + 1), 1, 0, &[&format!("m{index}")])
        })
        .collect();
    assert_eq!(
        state
            .active_blocks()
            .filter(|block| block.tier == 1)
            .count(),
        3
    );

    let quiet = turn(&messages, state.clone(), &config, 50_000);
    assert!(
        !quiet.nudge.should_inject,
        "no growth: {}",
        quiet.nudge.reason
    );
    let next = turn(&messages, state, &config, 60_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(2));
    assert!(!next.nudge.tier_target_blocks.is_empty());
}
