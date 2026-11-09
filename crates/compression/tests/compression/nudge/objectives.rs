// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `open-objectives.test.ts`: the guidance at every hop mandates the
// `Open objectives:` carry-forward, and the block lineage keeps the request.
// The full decompression of the distilled block is checked with the
// decompress module.

use hanafubuki_agent_compression::{render_nudge_text, Config, Message, Prompts, Role, State};

use super::{nudge_config, turn, warmed};
use crate::fixtures::{apply, range};

const OBJECTIVE: &str = "refactor the research runner into eight independent arms";

fn objectives_config() -> Config {
    let mut config = nudge_config();
    config.tiers.tier2_trigger = 1;
    config.tiers.tier3_trigger = 2;
    config
}

/// A hundred messages; `m9` (ref `m00010`) is the user's request.
fn session() -> Vec<Message> {
    (0..100)
        .map(|index| {
            let role = if index == 9 || index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = if index == 9 {
                format!("Please {OBJECTIVE}.")
            } else {
                format!("message {index} ").repeat(30)
            };
            Message::text(format!("m{index}"), role, text)
        })
        .collect()
}

#[test]
fn an_open_objective_survives_a_fold_and_a_distillation() {
    let config = objectives_config();
    let messages = session();
    let state = warmed(&messages, &config, 50_000);
    let state = apply(
        &[range(
            "m00001",
            "m00100",
            "T1 fold.\nOpen objectives: refactor research runner into eight arms (m00010).",
        )],
        &messages,
        &state,
        &config,
    )
    .state;
    let b1 = &state.blocks[0];
    assert_eq!(b1.block_id, "b1");
    assert_eq!(b1.tier, 1);
    assert!(b1.effective_message_ids.contains(&"m9".to_owned()));

    // The fold cleared the baselines: the next request restamps them, the
    // one after nudges the distillation.
    let rearm = turn(&messages, state, &config, 60_000);
    assert!(!rearm.nudge.should_inject, "{}", rearm.nudge.reason);
    let next = turn(&messages, rearm.state, &config, 70_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    assert_eq!(next.nudge.tier, Some(2), "{}", next.nudge.reason);
    let targets: Vec<&str> = next
        .nudge
        .tier_target_blocks
        .iter()
        .map(|block| block.block_id.as_str())
        .collect();
    assert_eq!(targets, ["b1"]);
    let rendered = render_nudge_text(&next.nudge, &Prompts::default(), false);
    assert!(rendered.text.contains("TIER 2 COMPRESSION"));
    assert!(rendered.text.contains("Open objectives"));

    let state: State = apply(
        &[range(
            "b1",
            "b1",
            "Source: b1 (fold m00001-m00100).\nObjective: refactor runner into eight arms (m00010) — open.",
        )],
        &messages,
        &next.state,
        &config,
    )
    .state;
    let b2 = &state.blocks[1];
    assert_eq!(b2.block_id, "b2");
    assert_eq!(b2.tier, 2);
    assert_eq!(b2.direct_block_ids, ["b1"]);
    assert!(b2.effective_message_ids.contains(&"m9".to_owned()));
}

#[test]
fn the_gentle_tier_one_nudge_carries_the_open_objectives_rule() {
    let mut config = objectives_config();
    config.tiers.enabled = false;
    config.tiers.tier2_trigger = 1000;
    config.tiers.tier3_trigger = 2000;
    let messages = session();
    let state = warmed(&messages, &config, 10_000);
    let next = turn(&messages, state, &config, 60_000);
    assert!(next.nudge.should_inject, "{}", next.nudge.reason);
    let text = render_nudge_text(&next.nudge, &Prompts::default(), false).text;
    assert!(text.contains("Open objectives:"));
    assert!(text
        .to_lowercase()
        .contains("open-objective status is current"));
}
