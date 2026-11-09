// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the per-tier threshold cases of `nudge.test.ts`
// (`nudge.tierGrowthTokens`) and their configuration check.

use hanafubuki_agent_compression::{Config, TierGrowthTokens};

use super::{gated_config, make_messages, nudge_config, t1_blocks, turn, warmed};

// Each message is about 5000 tokens; a block's summary mass is its
// characters over four. Derived thresholds: T1 6000, T2 and T3 9000.

fn preserved_gated() -> Config {
    let mut config = gated_config();
    // Only m0 (~5000 tokens) is T1-effective, under the T1 threshold.
    config.preserve_recent_messages = 9;
    config
}

#[test]
fn a_lowered_tier_two_threshold_fires_below_the_derived_one() {
    let messages = make_messages(10);
    let base = preserved_gated();
    // T2 mass 7000: above T1 effective 5000, below the derived 9000.
    let mut state = warmed(&messages, &base, 50_000);
    state.blocks = t1_blocks(&[&["m1"]], 28_000);
    let quiet = turn(&messages, state, &base, 60_000);
    assert!(!quiet.nudge.should_inject, "{}", quiet.nudge.reason);

    let mut lowered = base;
    lowered.nudge.tier_growth_tokens.t2 = Some(6000);
    let mut state = warmed(&messages, &lowered, 50_000);
    state.blocks = t1_blocks(&[&["m1"]], 28_000);
    let next = turn(&messages, state, &lowered, 60_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(2));
}

#[test]
fn a_raised_tier_one_threshold_holds_back_tier_one() {
    let messages = make_messages(10);
    let base = gated_config();
    let state = warmed(&messages, &base, 10_000);
    let default_turn = turn(&messages, state, &base, 55_000);
    assert!(default_turn.nudge.should_inject);
    assert_eq!(default_turn.nudge.tier, Some(1));

    let mut raised = base;
    raised.nudge.tier_growth_tokens.t1 = Some(100_000);
    let state = warmed(&messages, &raised, 10_000);
    let next = turn(&messages, state, &raised, 55_000);
    assert!(!next.nudge.should_inject, "{}", next.nudge.reason);
}

#[test]
fn pinning_one_tier_leaves_the_others_derived() {
    let messages = make_messages(10);
    let mut config = preserved_gated();
    config.nudge.tier_growth_tokens.t1 = Some(100_000);
    let mut state = warmed(&messages, &config, 50_000);
    // T2 mass 12000 >= the derived 9000.
    state.blocks = t1_blocks(&[&["m1"], &["m2"]], 24_000);
    let next = turn(&messages, state, &config, 60_000);
    assert!(next.nudge.should_inject);
    assert_eq!(next.nudge.tier, Some(2));
}

#[test]
fn tier_growth_tokens_must_be_positive() {
    let base = nudge_config();
    assert_eq!(base.validate(), Vec::<String>::new());
    let mut bad = base.clone();
    bad.nudge.tier_growth_tokens = TierGrowthTokens {
        t2: Some(0),
        ..TierGrowthTokens::default()
    };
    assert!(bad
        .validate()
        .iter()
        .any(|error| error.contains("nudge.tierGrowthTokens.t2")));
    let mut good = base;
    good.nudge.tier_growth_tokens.t1 = Some(1);
    assert!(!good
        .validate()
        .iter()
        .any(|error| error.contains("tierGrowthTokens")));
}
