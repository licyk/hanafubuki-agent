// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// The nudge decision's contracts; this module holds the helpers the nudge
// test files share (their `buildConfig`, `makeMessages`, `t1Blocks` and
// `t2Blocks`).

mod arbitration;
mod debt;
mod growth;
mod harvest;
mod objectives;
mod pressure;
mod reasons;
mod scale;
mod thresholds;

use hanafubuki_agent_compression::{
    process_turn, Block, Config, Message, RenderStrategy, Role, State, TokenCountSource, TurnInput,
    TurnOutput,
};

use crate::fixtures;

/// The kernel nudge tests' `buildConfig`: a 100K window whose growth step
/// is 6000 (the floor), with a growth floor of 5000 and tier thresholds of
/// 6000 (tier 1) and 9000 (tiers 2 and 3).
pub(crate) fn nudge_config() -> Config {
    let mut config = fixtures::config();
    config.tiers.tier2_trigger = 5;
    config.tiers.tier3_trigger = 10;
    let nudge = &mut config.nudge;
    nudge.max_context_limit_pct = 0.9;
    nudge.min_context_limit_pct = 0.45;
    nudge.growth_ratio = 0.05;
    nudge.growth_floor = 6000;
    nudge.growth_cap = 50_000;
    nudge.min_growth_floor = 5000;
    nudge.min_growth_ratio = 0.45;
    nudge.emergency_threshold_pct = 0.98;
    config.truncate.threshold = 1.0;
    config
}

/// `nudge_config` with the minimum range on, so the merged-range filter of
/// tier-1 pending engages.
pub(crate) fn gated_config() -> Config {
    let mut config = nudge_config();
    config.compress.min_compress_range = 5000;
    config
}

/// Alternating user and assistant texts `m0`, `m1`, … of about 5500 tokens
/// each.
pub(crate) fn make_messages(count: usize) -> Vec<Message> {
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
                format!("message {index} ").repeat(2000),
            )
        })
        .collect()
}

/// One request through the pipeline, with no count source and no tags.
pub(crate) fn turn(messages: &[Message], state: State, config: &Config, tokens: u64) -> TurnOutput {
    turn_with(messages, state, config, tokens, None)
}

pub(crate) fn turn_with(
    messages: &[Message],
    state: State,
    config: &Config,
    tokens: u64,
    source: Option<TokenCountSource>,
) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config,
        token_count: tokens,
        token_count_source: source,
        render: RenderStrategy::None,
    })
}

/// The state after one request at `tokens`.
pub(crate) fn warmed(messages: &[Message], config: &Config, tokens: u64) -> State {
    turn(messages, State::default(), config, tokens).state
}

pub(crate) fn block(block_id: &str, tier: u8, summary_chars: usize, ids: &[&str]) -> Block {
    let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
    Block {
        block_id: block_id.to_owned(),
        run_id: "r1".to_owned(),
        tier,
        summary: "x".repeat(summary_chars),
        direct_message_ids: ids.clone(),
        effective_message_ids: ids,
        compressed_tokens: summary_chars as u64,
        ..Block::default()
    }
}

/// Active tier-1 blocks `b1`, `b2`, … each covering its own ids.
pub(crate) fn t1_blocks(anchors: &[&[&str]], summary_chars: usize) -> Vec<Block> {
    anchors
        .iter()
        .enumerate()
        .map(|(index, ids)| block(&format!("b{}", index + 1), 1, summary_chars, ids))
        .collect()
}

/// Active tier-2 blocks `t2-1`, … carrying `ids` (so they stay active) and
/// consuming blocks that do not exist (so no tier-1 block is deactivated).
pub(crate) fn t2_blocks(count: usize, summary_chars: usize, ids: &[&str]) -> Vec<Block> {
    (1..=count)
        .map(|index| Block {
            run_id: "r2".to_owned(),
            direct_message_ids: Vec::new(),
            direct_block_ids: vec![format!("t2-src-{index}")],
            ..block(&format!("t2-{index}"), 2, summary_chars, ids)
        })
        .collect()
}
