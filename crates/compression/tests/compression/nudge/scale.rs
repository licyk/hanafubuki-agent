// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `tier-scale-separation.test.ts`: the default count triggers make
// tier distillation a late-session mechanism.

use hanafubuki_agent_compression::{Block, Config, Message, Role, State, TurnOutput};

use super::{block, nudge_config, turn, warmed};

fn scale_config() -> Config {
    let mut config = nudge_config();
    config.tiers.tier2_trigger = 1000;
    config.tiers.tier3_trigger = 2000;
    config.preserve_recent_messages = 30;
    config
}

fn short_messages(count: usize) -> Vec<Message> {
    (0..count)
        .map(|index| {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            Message::text(
                format!("m{index}"),
                role,
                format!("message {index} content"),
            )
        })
        .collect()
}

/// `count` blocks of a one-token summary each, so only the count can
/// trigger a tier.
fn micro_blocks(count: usize, tier: u8) -> Vec<Block> {
    (0..count)
        .map(|index| block(&format!("b{}", index + 1), tier, 4, &[&format!("m{index}")]))
        .collect()
}

/// The blocks cover messages outside the 30 preserved ones.
fn grown(blocks: Vec<Block>, message_count: usize) -> TurnOutput {
    let config = scale_config();
    let messages = short_messages(message_count);
    let mut state: State = warmed(&messages, &config, 50_000);
    state.blocks = blocks;
    turn(&messages, state, &config, 60_000)
}

#[test]
fn tier_two_fires_at_exactly_one_thousand_tier_one_blocks() {
    let quiet = grown(micro_blocks(999, 1), 1030);
    assert!(!quiet.nudge.should_inject, "{}", quiet.nudge.reason);
    assert!(!quiet.nudge.reason.contains("T2 distill ready"));

    let ready = grown(micro_blocks(1000, 1), 1031);
    assert!(ready.nudge.should_inject, "{}", ready.nudge.reason);
    assert_eq!(ready.nudge.tier, Some(2));
    assert!(
        ready
            .nudge
            .reason
            .contains("1000 tier-1 blocks >= tier2Trigger 1000"),
        "{}",
        ready.nudge.reason
    );
}

#[test]
fn tier_three_fires_at_exactly_two_thousand_tier_two_blocks() {
    let quiet = grown(micro_blocks(1999, 2), 2030);
    assert!(!quiet.nudge.should_inject, "{}", quiet.nudge.reason);
    assert!(!quiet.nudge.reason.contains("T3 condense ready"));

    let ready = grown(micro_blocks(2000, 2), 2031);
    assert!(ready.nudge.should_inject, "{}", ready.nudge.reason);
    assert_eq!(ready.nudge.tier, Some(3));
    assert!(
        ready
            .nudge
            .reason
            .contains("2000 tier-2 blocks >= tier3Trigger 2000"),
        "{}",
        ready.nudge.reason
    );
}

#[test]
fn the_default_triggers_encode_the_folded_mass_scale() {
    // The kernel's defaults, not the test configuration: a tier-1 block
    // folds at least the minimum range (5K) and typically one growth step
    // (50K); a tier-2 block distils at least 1.5 steps (75K).
    let config = Config::new(100_000);
    assert_eq!(config.tiers.tier2_trigger, 1000);
    assert_eq!(config.tiers.tier3_trigger, 2000);
    assert!(config.tiers.tier2_trigger as u64 * 5000 >= 5_000_000);
    assert!(config.tiers.tier2_trigger as u64 * 50_000 >= 50_000_000);
    assert!(config.tiers.tier3_trigger as u64 * 75_000 >= 150_000_000);
}
