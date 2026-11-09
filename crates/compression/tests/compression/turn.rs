// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the `processTurn` cases of `compress.test.ts` and `pipeline.test.ts`,
// the status-line case of `issue4521-retry-livelock.test.ts` and
// `validateConfig` of `config.ts`; adds the dead-ref lifting of
// `processTurn`.

use hanafubuki_agent_compression::{
    process_turn, Block, Config, Generation, Message, RenderStrategy, Role, State, TurnInput,
    TurnOutput,
};

use crate::fixtures::{assistant, call, result, state_for, user};

fn turn(messages: &[Message], state: State, config: &Config, token_count: u64) -> TurnOutput {
    turn_rendered(
        messages,
        state,
        config,
        token_count,
        RenderStrategy::TextAndToolResults,
    )
}

fn turn_rendered(
    messages: &[Message],
    state: State,
    config: &Config,
    token_count: u64,
    render: RenderStrategy,
) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config,
        token_count,
        token_count_source: None,
        render,
        clock: None,
    })
}

/// `compress.test.ts`'s configuration.
fn compress_config() -> Config {
    let mut config = crate::fixtures::config();
    config.tiers.tier2_trigger = 5;
    config.tiers.tier3_trigger = 10;
    config.nudge.max_context_limit_pct = 0.55;
    config.nudge.growth_floor = 6000;
    config.nudge.min_growth_floor = 5000;
    config.nudge.emergency_threshold_pct = 0.98;
    config.truncate.threshold = 1.0;
    config
}

#[test]
fn a_turn_assigns_refs_prunes_and_decides_the_nudge() {
    let messages = [user("a", "hello world"), user("b", "second message")];
    let output = turn(&messages, State::default(), &compress_config(), 50_000);
    assert_eq!(output.state.message_refs.by_raw["a"], "m00001");
    assert_eq!(output.state.message_refs.by_raw["b"], "m00002");
    assert_eq!(output.messages.len(), 2);
    assert!(output.nudge.context_usage > 0.0);
}

#[test]
fn blocks_are_never_deactivated_for_age() {
    let messages = [user("old1", "content from old turn")];
    let mut state = State::default();
    state.blocks.push(Block {
        block_id: "b1".to_owned(),
        run_id: "r1".to_owned(),
        summary: "old but still active".to_owned(),
        direct_message_ids: vec!["old1".to_owned()],
        effective_message_ids: vec!["old1".to_owned()],
        survived_count: 999,
        generation: Generation::Old,
        ..Block::default()
    });
    let output = turn(&messages, state, &compress_config(), 95_000);
    assert!(output.state.blocks[0].active);
}

fn is_tagged(message: &Message, r#ref: &str) -> bool {
    let prefix = "<acp tokens=\"";
    let Some(rest) = message.text.strip_prefix(prefix) else {
        return false;
    };
    let tokens: String = rest.chars().take_while(char::is_ascii_digit).collect();
    !tokens.is_empty()
        && rest[tokens.len()..].starts_with(&format!("\" type=\"text\">{}</acp>\n", r#ref))
}

#[test]
fn a_turn_tags_every_mapped_text_with_its_ref() {
    let messages = [user("a", "alpha"), user("b", "beta")];
    let output = turn(&messages, State::default(), &Config::new(100_000), 100);
    assert!(is_tagged(&output.messages[0], "m00001"));
    assert!(output.messages[0].text.ends_with(">m00001</acp>\nalpha"));
    assert!(is_tagged(&output.messages[1], "m00002"));
    assert!(output.messages[1].text.ends_with(">m00002</acp>\nbeta"));
}

fn tool_exchange() -> Vec<Message> {
    vec![
        user("u1", "run echo"),
        call("a1", "tc1", "bash", r#"{"command":"echo hello"}"#),
        result("t1", "tc1", "bash", "hello"),
        assistant("a2", "Done."),
    ]
}

fn refs_in_order(output: &TurnOutput) {
    let refs = &output.state.message_refs.by_raw;
    assert_eq!(refs["u1"], "m00001");
    assert_eq!(refs["a1"], "m00002");
    assert_eq!(refs["t1"], "m00003");
    assert_eq!(refs["a2"], "m00004");
}

// The kernel's default tags tool-call arguments too; here the arguments are
// JSON and never tagged, and tool results are.
#[test]
fn text_and_results_are_tagged_and_call_arguments_never() {
    let messages = tool_exchange();
    let output = turn(&messages, State::default(), &Config::new(100_000), 50);
    assert!(output.messages[0].text.contains("m00001</acp>"));
    assert_eq!(output.messages[1].text, r#"{"command":"echo hello"}"#);
    assert!(output.messages[2].text.contains("m00003</acp>"));
    assert!(output.messages[2].text.starts_with("<acp tokens=\""));
    assert!(output.messages[2]
        .text
        .contains("type=\"bash\">m00003</acp>\nhello"));
    assert!(output.messages[3].text.contains("m00004</acp>"));
    refs_in_order(&output);
}

#[test]
fn text_only_leaves_tool_messages_pristine_and_still_maps_them() {
    let messages = tool_exchange();
    let output = turn_rendered(
        &messages,
        State::default(),
        &Config::new(100_000),
        50,
        RenderStrategy::TextOnly,
    );
    assert!(output.messages[0].text.contains("m00001</acp>"));
    assert!(output.messages[3].text.contains("m00004</acp>"));
    assert_eq!(output.messages[1].text, r#"{"command":"echo hello"}"#);
    assert_eq!(output.messages[2].text, "hello");
    refs_in_order(&output);
}

#[test]
fn no_tags_maps_every_message_and_leaves_all_text_untouched() {
    let messages = tool_exchange();
    let output = turn_rendered(
        &messages,
        State::default(),
        &Config::new(100_000),
        50,
        RenderStrategy::None,
    );
    let texts: Vec<&str> = output
        .messages
        .iter()
        .map(|message| message.text.as_str())
        .collect();
    assert_eq!(
        texts,
        ["run echo", r#"{"command":"echo hello"}"#, "hello", "Done."]
    );
    assert!(output.state.token_snapshot.is_empty());
    refs_in_order(&output);
}

/// `issue4521-retry-livelock.test.ts`'s configuration.
fn livelock_config() -> Config {
    let mut config = Config::new(872_000);
    config.tiers.tier2_trigger = 5;
    config.tiers.tier3_trigger = 10;
    config.nudge.max_context_limit_pct = 0.75;
    config.nudge.min_context_limit_pct = 0.05;
    config.nudge.growth_ratio = 0.06;
    config.nudge.min_growth_floor = 22_500;
    config.nudge.emergency_threshold_pct = 0.98;
    config.truncate.threshold = 1.0;
    config.compress.min_compress_range = 100;
    config.compress.min_summary_length = 1;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

#[test]
fn an_armed_but_silent_round_shows_the_highest_ref_and_the_offered_ranges() {
    let config = livelock_config();
    let messages: Vec<Message> = (0..40)
        .map(|index| {
            let role = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            Message::text(format!("m{index}"), role, "x".repeat(24_000))
        })
        .collect();

    // First sight of a 240K backlog nudges; the nudge carries the refs.
    let warm = turn(&messages, State::default(), &config, 600_000);
    assert!(warm.nudge.should_inject, "{}", warm.nudge.reason);
    assert_eq!(warm.status_line, None);

    // Growth 10K is under the 22.5K floor and usage 70% under the band: the
    // round stays armed but its nudge is withheld.
    let idle = turn(&messages, warm.state, &config, 610_000);
    assert!(!idle.nudge.should_inject, "{}", idle.nudge.reason);
    assert!(!idle.nudge.compressible_ranges.is_empty());
    assert_eq!(
        warm.nudge.reason,
        "T1 effective 240000 >= 50000, growth 0, usage 69% [first-sight mass]"
    );
    assert_eq!(
        idle.nudge.reason,
        "growth 10000 < floor 22500, ready: T1 240000, blocked: T1 (cadence)"
    );
    assert_eq!(
        idle.status_line.as_deref(),
        Some("[ACP status: highest allocated ref m00040 — refs above it are not minted yet, never guess them; compressible now: m00001–m00004 (24.0K), m00005–m00008 (24.0K), m00009–m00012 (24.0K), +7 more; the newest messages stay protected. Use these refs directly; acp_status only if you need the full list.]")
    );
}

#[test]
fn a_fresh_session_below_every_threshold_stays_silent() {
    let config = livelock_config();
    let messages: Vec<Message> = (0..4)
        .map(|index| Message::text(format!("m{index}"), Role::User, "x".repeat(24_000)))
        .collect();
    let output = turn(&messages, State::default(), &config, 30_000);
    assert!(!output.nudge.should_inject);
    assert_eq!(output.status_line, None);
}

#[test]
fn a_dead_ref_whose_message_came_back_is_alive_again() {
    let messages = [user("a", "alpha"), user("b", "beta"), user("c", "gamma")];
    let mut state = state_for(&messages);
    state.dead_refs = vec!["m00002".to_owned(), "m00009".to_owned()];
    let output = turn(&messages, state, &Config::new(100_000), 100);
    assert_eq!(output.state.dead_refs, ["m00009"]);
}

#[test]
fn the_default_configuration_is_valid() {
    assert!(Config::new(100_000).validate().is_empty());
}

/// A change that makes the default configuration unsound.
type Unsound = fn(&mut Config);

#[test]
fn validation_reports_each_unsound_setting() {
    let cases: [(Unsound, &str); 8] = [
        (
            |config| config.model_context_limit = 0,
            "modelContextLimit must be a positive number",
        ),
        (
            |config| config.nudge.min_context_limit_pct = 0.8,
            "nudge.minContextLimitPct must not exceed nudge.maxContextLimitPct",
        ),
        (
            |config| config.nudge.emergency_threshold_pct = 0.7,
            "nudge.maxContextLimitPct must not exceed nudge.emergencyThresholdPct",
        ),
        (
            |config| config.promotion_threshold = 0,
            "promotionThreshold must be >= 1",
        ),
        (
            |config| config.truncate.threshold = 0.0,
            "truncate.threshold must be in (0, 1]",
        ),
        (
            |config| config.truncate.threshold = 1.5,
            "truncate.threshold must be in (0, 1]",
        ),
        (
            |config| config.tiers.tier3_trigger = config.tiers.tier2_trigger,
            "tiers.tier3Trigger must be greater than tiers.tier2Trigger",
        ),
        (
            |config| config.nudge.tier_growth_tokens.t2 = Some(0),
            "nudge.tierGrowthTokens.t2 must be a positive number",
        ),
    ];
    for (unsound, message) in cases {
        let mut config = Config::new(100_000);
        unsound(&mut config);
        assert_eq!(config.validate(), [message]);
    }
    let mut config = Config::new(100_000);
    config.tiers.tier2_trigger = 0;
    assert_eq!(config.validate(), ["tier triggers must be >= 1"]);
}
