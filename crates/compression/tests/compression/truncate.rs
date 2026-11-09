// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `terminal-escape.test.ts`, `issue3939-truncate-latch.test.ts`,
// `issue5101-truncate-blocked-latch.test.ts`, the truncation pins of
// `issue3939-fold-boundary-pins.test.ts`, `issue3935-truncation-shrink.test.ts`,
// the truncation cases of `truncate-keep-filter.test.ts` and the character
// boundary cases of `truncate-surrogates.test.ts`.
//
// The kernel's tests often count a token per character; the port's one
// estimate counts four characters a token, so texts and token counts are
// scaled to keep each scenario on the same side of every threshold.

use serde_json::{json, Value};

use hanafubuki_agent_compression::{
    build_recap, derive_topic_from_summary, hide_consumed_compress_calls, parse_compress_args,
    process_turn, topic_fallback, truncate_large_tool_outputs, Block, Config, Message,
    RenderStrategy, Role, State, TruncateLatchEntry, TruncateOptions, TurnInput, TurnOutput,
    BLOCKED_REF,
};

use crate::fixtures::{apply, assistant, call, range, result, state_for, user, with_topic};

const MARKER: &str = "[truncated for context space]";
const EMOJI: &str = "\u{1F980}";

fn turn(messages: &[Message], state: State, config: &Config, token_count: u64) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config,
        token_count,
        token_count_source: None,
        render: RenderStrategy::None,
        clock: None,
    })
}

fn text_of<'a>(messages: &'a [Message], id: &str) -> &'a str {
    &messages
        .iter()
        .find(|message| message.id == id)
        .unwrap_or_else(|| panic!("{id} in view"))
        .text
}

fn system(id: &str, text: &str) -> Message {
    Message::text(id, Role::System, text)
}

fn tool_output(id: &str, text: &str) -> Message {
    result(id, &format!("call-{id}"), "read", text)
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn with_threshold(threshold: f64) -> Config {
    let mut config = Config::new(100_000);
    config.truncate.threshold = threshold;
    config
}

fn options(min_output_tokens: u64, protect_recent_messages: usize) -> TruncateOptions {
    TruncateOptions {
        min_output_tokens,
        protect_recent_messages,
        ..TruncateOptions::default()
    }
}

// terminal-escape.test.ts

/// The irreducible floor (a system prompt) is over the limit; every other
/// message is tiny and in the recent zone: nothing to compress or clip.
fn floor_session() -> (Vec<Message>, u64) {
    (
        vec![
            system("sys", &"S".repeat(80_000)),
            user("u1", "small request"),
            assistant("a1", "ok"),
        ],
        96_000,
    )
}

#[test]
fn the_terminal_escape_fires_after_three_stuck_requests() {
    let config = Config::new(100_000);
    let (messages, tokens) = floor_session();
    let first = turn(&messages, State::default(), &config, tokens);
    assert!(first.terminal_escape.is_none());
    assert_eq!(first.state.terminal_streak, 1);
    assert!(first
        .truncation_skipped
        .as_deref()
        .expect("zero-candidate diagnostic")
        .contains("no truncatable content"));

    let second = turn(&messages, first.state, &config, tokens);
    assert!(second.terminal_escape.is_none());
    assert_eq!(second.state.terminal_streak, 2);

    let third = turn(&messages, second.state, &config, tokens);
    let escape = third
        .terminal_escape
        .expect("fires on the third stuck request");
    assert_eq!(escape.stuck_events, 3);
    assert!(escape
        .message
        .contains("compression cannot reduce this context below the limit"));
    assert!(escape
        .message
        .contains("Start a new session or use native compaction"));
    assert_eq!(
        escape.message,
        "Usage at 96% (96000/100000 tokens) persists with nothing compressible above the benefit floor and no truncatable content: compression cannot reduce this context below the limit. Start a new session or use native compaction."
    );
    assert!((escape.usage - 0.96).abs() < 0.01);
    assert_eq!(escape.model_context_limit, 100_000);
    assert_eq!(third.nudge.breakdown.max_pending, 0);
}

#[test]
fn terminal_escape_after_is_configurable_and_zero_disables_it() {
    let (messages, tokens) = floor_session();
    let mut fast = Config::new(100_000);
    fast.truncate.terminal_escape_after = 1;
    let out = turn(&messages, State::default(), &fast, tokens);
    assert_eq!(out.terminal_escape.expect("fires at once").stuck_events, 1);

    let mut off = Config::new(100_000);
    off.truncate.terminal_escape_after = 0;
    let mut state = State::default();
    for round in 1..=4 {
        let out = turn(&messages, state, &off, tokens);
        assert!(out.terminal_escape.is_none(), "request {round}: disabled");
        state = out.state;
    }
    assert_eq!(state.terminal_streak, 4, "the streak is still tracked");
}

#[test]
fn the_streak_resets_when_usage_drops_below_the_threshold() {
    let config = Config::new(100_000);
    let (messages, hot) = floor_session();
    let mut state = State::default();
    for _ in 0..3 {
        state = turn(&messages, state, &config, hot).state;
    }
    assert_eq!(state.terminal_streak, 3);
    let cool = turn(&messages, state, &config, 90_000);
    assert_eq!(cool.state.terminal_streak, 0);
    assert!(cool.terminal_escape.is_none());
}

#[test]
fn a_successful_fold_resets_the_streak() {
    let config = Config::new(100_000);
    let messages = vec![user("u1", &"x".repeat(20_000)), assistant("a1", "done")];
    let mut state = state_for(&messages);
    state.terminal_streak = 3;
    let out = hanafubuki_agent_compression::apply_compression(
        &hanafubuki_agent_compression::ApplyInput {
            ranges: &[range(
                "m00001",
                "m00002",
                "User made a long request; assistant completed it fully.",
            )],
            messages: &messages,
            state: &state,
            config: &config,
            protected_refs: Some(&Default::default()),
            now_ms: 0,
        },
    );
    assert_eq!(out.result.blocks_created, 1);
    assert_eq!(out.state.terminal_streak, 0);
}

#[test]
fn oversized_user_and_assistant_text_is_a_last_resort_candidate() {
    let config = with_threshold(0.5);
    let messages = vec![
        user("big-u", &"T".repeat(30_000)),
        assistant("s1", "ok"),
        user("s2", "next"),
        assistant("s3", "fine"),
        user("s4", "more"),
        assistant("s5", "last"),
    ];
    let legacy = truncate_large_tool_outputs(&messages, 90_000, &config, &options(1000, 3));
    assert_eq!(legacy.truncated_count, 0, "tool results only by default");
    assert_eq!(legacy.candidates_found, 0);

    let broad = truncate_large_tool_outputs(
        &messages,
        90_000,
        &config,
        &TruncateOptions {
            include_text_messages: true,
            ..options(1000, 3)
        },
    );
    assert_eq!(broad.truncated_count, 1);
    assert!(broad.saved_tokens > 0);
    assert!(broad.messages[0].text.contains(MARKER));
    assert!(!broad.messages[1].text.contains(MARKER));
}

#[test]
fn tool_results_are_clipped_before_text() {
    let config = with_threshold(0.5);
    let messages = vec![
        tool_output("tr1", &"L".repeat(30_000)),
        tool_output("tr2", &"L".repeat(30_000)),
        assistant("txt", &"T".repeat(30_000)),
        user("p1", "recent"),
        user("p2", "recent"),
        user("p3", "recent"),
    ];
    let both_options = TruncateOptions {
        include_text_messages: true,
        ..options(1000, 3)
    };
    // Each clip saves about 6.5K tokens; two bring 55K under the 45K target.
    let both = truncate_large_tool_outputs(&messages, 55_000, &config, &both_options);
    assert_eq!(both.truncated_count, 2, "both tool results close the gap");
    assert!(!both.messages[2].text.contains(MARKER));

    let single_history: Vec<Message> = [0, 2, 3, 4, 5]
        .iter()
        .map(|index| messages[*index].clone())
        .collect();
    let single = truncate_large_tool_outputs(&single_history, 55_000, &config, &both_options);
    assert_eq!(
        single.truncated_count, 2,
        "the text stage runs when tool results are not enough"
    );
    assert!(single.messages[1].text.contains(MARKER));
}

#[test]
fn rendered_summaries_and_header_prefixed_system_text_are_never_clipped() {
    let config = with_threshold(0.5);
    let messages = vec![
        system(
            "acp_summary_b1",
            &format!("[Compressed conversation section]\n{}", "Z".repeat(30_000)),
        ),
        system(
            "host-copy",
            &format!(
                "[Compressed conversation section] host copy {}",
                "Y".repeat(30_000)
            ),
        ),
        assistant("txt", &"T".repeat(30_000)),
        user("p1", "recent"),
        user("p2", "recent"),
        user("p3", "recent"),
    ];
    let out = truncate_large_tool_outputs(
        &messages,
        90_000,
        &config,
        &TruncateOptions {
            include_text_messages: true,
            ..options(1000, 3)
        },
    );
    assert_eq!(out.truncated_count, 1);
    assert!(!out.messages[0].text.contains(MARKER));
    assert!(!out.messages[1].text.contains(MARKER));
    assert!(out.messages[2].text.contains(MARKER));
}

#[test]
fn candidates_found_tells_none_from_unsavable() {
    let config = with_threshold(0.5);
    let none = truncate_large_tool_outputs(
        &[system("sys", &"S".repeat(80_000))],
        90_000,
        &config,
        &TruncateOptions {
            include_text_messages: true,
            ..options(1000, 3)
        },
    );
    assert_eq!(none.candidates_found, 0);
    assert_eq!(none.truncated_count, 0);

    let short = truncate_large_tool_outputs(
        &[
            tool_output("tr", &"L".repeat(4000)),
            user("p1", "recent"),
            user("p2", "recent"),
            user("p3", "recent"),
        ],
        90_000,
        &config,
        &options(1000, 3),
    );
    assert_eq!(short.candidates_found, 1);
    assert_eq!(
        short.truncated_count, 0,
        "the kept head and tail leave nothing to save"
    );
}

#[test]
fn process_turn_clips_oversized_assistant_text_over_the_threshold() {
    let config = Config::new(100_000);
    let messages = vec![
        system("sys", &"S".repeat(60_000)),
        assistant("big-a", &"A".repeat(40_000)),
        user("u1", "hi"),
        assistant("a2", "yo"),
        user("u2", "bye"),
        assistant("a3", "see ya"),
        user("u3", "again"),
        assistant("a4", "still here"),
    ];
    let out = turn(&messages, State::default(), &config, 96_000);
    assert!(text_of(&out.messages, "big-a").contains(MARKER));
    assert!(!text_of(&out.messages, "sys").contains(MARKER));
    assert!(out.terminal_escape.is_none(), "a clip saved tokens");
    assert_eq!(out.state.terminal_streak, 0);
}

// truncate-keep-filter.test.ts (its message-filter cases are in `opt_in/filter.rs`)

#[test]
fn truncation_is_a_no_op_below_the_threshold() {
    let config = with_threshold(0.8);
    let out = truncate_large_tool_outputs(
        &[tool_output("t1", &"x".repeat(5000))],
        1000,
        &config,
        &options(100, 3),
    );
    assert_eq!(out.truncated_count, 0);
}

#[test]
fn large_tool_outputs_are_clipped_over_the_threshold() {
    let config = with_threshold(0.5);
    let big = "L".repeat(10_000);
    let messages = vec![
        tool_output("a", &big),
        tool_output("b", &big),
        tool_output("c", "recent"),
        tool_output("d", "recent"),
        tool_output("e", "recent"),
    ];
    let out = truncate_large_tool_outputs(
        &messages,
        90_000,
        &config,
        &TruncateOptions {
            keep_prefix_chars: 100,
            keep_suffix_chars: 100,
            ..options(100, 3)
        },
    );
    assert!(out.truncated_count >= 1);
    assert!(out.saved_tokens > 0);
    assert!(out.messages[0].text.contains(MARKER));
}

#[test]
fn recent_messages_are_never_clipped() {
    let config = with_threshold(0.1);
    let out = truncate_large_tool_outputs(
        &[tool_output("only", &"L".repeat(10_000))],
        90_000,
        &config,
        &options(100, 3),
    );
    assert_eq!(out.truncated_count, 0);
}

/// Large tool results, each with its call (the kernel's fixture had bare
/// results, which the port's prune drops as orphans).
fn big_results_history() -> Vec<Message> {
    let big = "L".repeat(40_000);
    let mut messages = Vec::new();
    for id in ["a", "b", "c"] {
        messages.push(call(&format!("c-{id}"), id, "read", "{}"));
        messages.push(result(id, id, "read", &big));
    }
    for id in ["d", "e", "f", "g", "h"] {
        messages.push(user(id, "recent"));
    }
    messages
}

#[test]
fn the_emergency_step_clips_at_the_default_threshold() {
    let out = turn(
        &big_results_history(),
        State::default(),
        &Config::new(100_000),
        96_000,
    );
    let clipped = out
        .messages
        .iter()
        .filter(|message| message.text.contains(MARKER))
        .count();
    assert!(clipped >= 1, "at 96% usage, got {clipped}");
}

#[test]
fn the_emergency_step_is_a_no_op_below_the_default_threshold() {
    let out = turn(
        &big_results_history(),
        State::default(),
        &Config::new(100_000),
        80_000,
    );
    assert!(out
        .messages
        .iter()
        .all(|message| !message.text.contains(MARKER)));
}

// issue3939-fold-boundary-pins.test.ts (the truncation pins)

fn big_tool_history() -> Vec<Message> {
    vec![
        tool_output("t1", &"A".repeat(400_000)),
        user("p1", "q1"),
        user("p2", "q2"),
        user("p3", "q3"),
    ]
}

#[test]
fn a_clip_at_the_water_level_has_the_exact_marker_format() {
    let config = Config::new(100_000);
    let out =
        truncate_large_tool_outputs(&big_tool_history(), 96_000, &config, &Default::default());
    assert_eq!(out.truncated_count, 1);
    assert!(out.saved_tokens > 0);
    let text = &out.messages[0].text;
    assert!(text.contains("...[truncated for context space] — original ~100000 tokens]..."));
    assert_eq!(utf16_len(text), 4066);
}

#[test]
fn the_same_history_passes_untouched_below_the_water_level() {
    let config = Config::new(100_000);
    let out =
        truncate_large_tool_outputs(&big_tool_history(), 94_000, &config, &Default::default());
    assert_eq!(out.truncated_count, 0);
    assert_eq!(out.messages[0].text.len(), 400_000);
}

#[test]
fn already_clipped_text_is_skipped_on_the_next_pass() {
    let config = Config::new(100_000);
    let first =
        truncate_large_tool_outputs(&big_tool_history(), 96_000, &config, &Default::default());
    let second = truncate_large_tool_outputs(&first.messages, 96_000, &config, &Default::default());
    assert_eq!(second.truncated_count, 0);
}

// issue3935-truncation-shrink.test.ts

fn shrink_history(text: &str) -> Vec<Message> {
    vec![
        tool_output("t1", text),
        user("p1", "q1"),
        user("p2", "q2"),
        user("p3", "q3"),
    ]
}

fn clip_once(text: &str) -> hanafubuki_agent_compression::TruncateResult {
    truncate_large_tool_outputs(
        &shrink_history(text),
        96_000,
        &Config::new(100_000),
        &Default::default(),
    )
}

#[test]
fn a_just_over_window_output_clips_strictly_shorter() {
    let original = "A".repeat(4001);
    let out = clip_once(&original);
    assert_eq!(out.truncated_count, 1);
    let text = &out.messages[0].text;
    assert!(text.contains(MARKER));
    assert!(utf16_len(text) < original.len());
    assert!(out.saved_tokens > 0);
}

#[test]
fn every_clip_in_the_bloat_window_is_strictly_shorter() {
    for size in [4001, 4010, 4040, 4060, 4067, 4068, 4200, 6000] {
        let out = clip_once(&"A".repeat(size));
        assert_eq!(out.truncated_count, 1, "size {size}");
        assert!(utf16_len(&out.messages[0].text) < size, "size {size}");
    }
}

#[test]
fn large_outputs_keep_the_full_window() {
    let out = clip_once(&"A".repeat(400_000));
    assert_eq!(out.truncated_count, 1);
    assert_eq!(utf16_len(&out.messages[0].text), 4066);
}

#[test]
fn clipping_is_deterministic() {
    let original = "B".repeat(4037);
    assert_eq!(
        clip_once(&original).messages[0].text,
        clip_once(&original).messages[0].text
    );
}

#[test]
fn clipped_text_is_never_clipped_again() {
    let clipped = clip_once(&"C".repeat(5000)).messages[0].text.clone();
    let second = clip_once(&clipped);
    assert_eq!(second.truncated_count, 0);
    assert_eq!(second.messages[0].text, clipped);
}

#[test]
fn a_candidate_too_small_to_shrink_stays_verbatim() {
    let out = truncate_large_tool_outputs(
        &shrink_history("short payload"),
        96_000,
        &Config::new(100_000),
        &TruncateOptions {
            keep_prefix_chars: 0,
            keep_suffix_chars: 0,
            min_output_tokens: 1,
            ..TruncateOptions::default()
        },
    );
    assert_eq!(out.truncated_count, 0);
    assert_eq!(out.messages[0].text, "short payload");
}

// issue3939-truncate-latch.test.ts

const BIG_REPEAT: usize = 4000;

fn big() -> String {
    "alpha-beta ".repeat(BIG_REPEAT)
}

/// A user message with roomy filler, so a fold over the head clears the
/// minimum range even with the big text clipped, and a short answer.
fn tiny(tag: &str) -> [Message; 2] {
    [
        user(
            &format!("raw-{tag}"),
            &format!("hi {tag} filler {tag} {}", "x".repeat(800)),
        ),
        assistant(&format!("raw-{tag}-a"), &format!("ok {tag}")),
    ]
}

fn latch_history() -> Vec<Message> {
    let mut messages = tiny("warm").to_vec();
    messages.push(user("raw-big", &big()));
    for tag in ["tail1", "tail2", "tail3", "tail4"] {
        messages.extend(tiny(tag));
    }
    messages
}

/// A narrow recent zone so the head of a small history can fold.
fn narrow_zone() -> Config {
    let mut config = Config::new(100_000);
    config.preserve_recent_tokens = 100;
    config
}

#[test]
fn a_clip_is_replayed_byte_identical_under_the_water_level() {
    let config = Config::new(100_000);
    let first = turn(&latch_history(), State::default(), &config, 96_000);
    let clipped = text_of(&first.messages, "raw-big").to_owned();
    assert!(clipped.contains(MARKER), "the first request clips");
    assert!(first
        .state
        .emergency_truncate_latch
        .iter()
        .any(|entry| entry.text == clipped));

    let second = turn(&latch_history(), first.state, &config, 60_000);
    assert_eq!(text_of(&second.messages, "raw-big"), clipped);
    assert!(
        second
            .state
            .emergency_truncate_latch
            .iter()
            .any(|entry| entry.text == clipped),
        "the latch stays while its message is live"
    );
}

#[test]
fn a_successful_fold_retires_the_latch() {
    let config = narrow_zone();
    let first = turn(&latch_history(), State::default(), &config, 96_000);
    assert!(!first.state.emergency_truncate_latch.is_empty());
    let ref_of = |id: &str| first.state.message_refs.by_raw[id].clone();
    let fold = apply(
        &[with_topic(
            range(
                &ref_of("raw-warm"),
                &ref_of("raw-tail1-a"),
                "a long enough summary for the fold gate to accept this range, over fifty chars",
            ),
            "t",
        )],
        &first.messages,
        &first.state,
        &config,
    );
    assert_eq!(fold.result.blocks_created, 1, "{:?}", fold.result.errors);
    assert!(fold.state.emergency_truncate_latch.is_empty());

    let mut post: Vec<Message> = latch_history()
        .into_iter()
        .filter(|message| message.id != "raw-big")
        .collect();
    post.push(user("raw-big2", &big()));
    let third = turn(&post, fold.state, &config, 60_000);
    assert_eq!(
        text_of(&third.messages, "raw-big2"),
        big(),
        "no clip under the water level after the reset"
    );
    assert!(third.state.emergency_truncate_latch.is_empty());
}

#[test]
fn an_edited_message_is_not_replay_clipped_from_a_stale_entry() {
    let config = Config::new(100_000);
    let first = turn(&latch_history(), State::default(), &config, 96_000);
    assert!(!first.state.emergency_truncate_latch.is_empty());
    let edited: Vec<Message> = latch_history()
        .into_iter()
        .map(|message| {
            if message.id == "raw-big" {
                user("raw-big-v2", "completely rewritten, short")
            } else {
                message
            }
        })
        .collect();
    let second = turn(&edited, first.state, &config, 60_000);
    assert_eq!(
        text_of(&second.messages, "raw-big-v2"),
        "completely rewritten, short"
    );
    assert!(
        second.state.emergency_truncate_latch.is_empty(),
        "the stale entry retired with its message"
    );
}

// issue5101-truncate-blocked-latch.test.ts

const RULE_TOOL: &str = "mcp__bili__acp_rule";
const ORIGINAL_ARGS: &str = r#"{"action":"remove","id":"rule1"}"#;

fn long_result() -> String {
    format!("OLD_RECEIPT_BEGIN {}", "long result content ".repeat(1000))
}

fn rule_config() -> Config {
    let mut config = Config::new(10_000);
    config.preserve_recent_messages = 3;
    config.protected_tools = vec![RULE_TOOL.to_owned()];
    config
}

fn rule_history() -> Vec<Message> {
    let mut messages = vec![
        user("user1", "Start"),
        call("call1", "tc1", RULE_TOOL, r#"{"action":"list"}"#),
        result("result1", "tc1", RULE_TOOL, &long_result()),
        call("call2", "tc2", RULE_TOOL, ORIGINAL_ARGS),
        result("result2", "tc2", RULE_TOOL, r#"{"ok":true}"#),
    ];
    for index in 0..6 {
        let text = format!("Recent message {index}");
        messages.push(if index % 2 == 1 {
            assistant(&format!("tail{index}"), &text)
        } else {
            user(&format!("tail{index}"), &text)
        });
    }
    messages
}

fn assert_rule_calls_intact(messages: &[Message]) {
    assert_eq!(text_of(messages, "call2"), ORIGINAL_ARGS);
    assert_eq!(text_of(messages, "result2"), r#"{"ok":true}"#);
}

fn no_blocked_entry(latch: &[TruncateLatchEntry]) -> bool {
    latch.iter().all(|entry| entry.r#ref != BLOCKED_REF)
}

#[test]
fn clipping_a_protected_result_seeds_no_shared_latch() {
    let config = rule_config();
    let messages = rule_history();
    let first = turn(&messages, State::default(), &config, 9800);
    assert_eq!(first.state.message_refs.by_raw["result1"], BLOCKED_REF);
    assert!(text_of(&first.messages, "result1").contains(MARKER));
    assert!(no_blocked_entry(&first.state.emergency_truncate_latch));
    let second = turn(&messages, first.state, &config, 100);
    assert_rule_calls_intact(&second.messages);
}

#[test]
fn persisted_blocked_entries_are_discarded() {
    let config = rule_config();
    let messages = rule_history();
    for token_count in [100, 9800] {
        let mut persisted = turn(&messages, State::default(), &config, 100).state;
        persisted.emergency_truncate_latch = ["wrong receipt one", "wrong receipt two"]
            .iter()
            .map(|text| TruncateLatchEntry {
                r#ref: BLOCKED_REF.to_owned(),
                text: (*text).to_owned(),
            })
            .collect();
        let out = turn(&messages, persisted, &config, token_count);
        assert_rule_calls_intact(&out.messages);
        assert!(
            no_blocked_entry(&out.state.emergency_truncate_latch),
            "at {token_count}"
        );
    }
}

#[test]
fn old_blocked_entries_are_cleaned_and_a_valid_clip_kept() {
    let config = rule_config();
    let mut messages = rule_history();
    messages.insert(0, user("ordinary-large", &long_result()));
    let mut state = turn(&messages, State::default(), &config, 100).state;
    let r#ref = state.message_refs.by_raw["ordinary-large"].clone();
    assert_ne!(r#ref, BLOCKED_REF);
    state.emergency_truncate_latch = vec![
        TruncateLatchEntry {
            r#ref: r#ref.clone(),
            text: "valid unique clip".to_owned(),
        },
        TruncateLatchEntry {
            r#ref: BLOCKED_REF.to_owned(),
            text: "wrong shared clip".to_owned(),
        },
    ];
    let out = turn(&messages, state, &config, 100);
    assert_eq!(
        text_of(&out.messages, "ordinary-large"),
        "valid unique clip"
    );
    assert_eq!(text_of(&out.messages, "call2"), ORIGINAL_ARGS);
    assert_eq!(
        out.state.emergency_truncate_latch,
        [TruncateLatchEntry {
            r#ref,
            text: "valid unique clip".to_owned(),
        }]
    );
}

// truncate-surrogates.test.ts: Rust strings hold no lone surrogates, so the
// contract becomes "no cut splits a character": a cut through an astral
// character drops the whole character.

#[test]
fn a_clips_head_and_tail_never_split_an_astral_character() {
    // The head cut at 10 units and the tail cut 10 units from the end each
    // fall inside an emoji.
    let text = format!(
        "{}{EMOJI}{}{EMOJI}{}",
        "x".repeat(9),
        "m".repeat(300),
        "q".repeat(9)
    );
    let mut config = Config::new(1000);
    config.truncate.threshold = 0.95;
    let out = truncate_large_tool_outputs(
        &[tool_output("t1", &text), user("u1", "go")],
        1000,
        &config,
        &TruncateOptions {
            min_output_tokens: 1,
            keep_prefix_chars: 10,
            keep_suffix_chars: 10,
            protect_recent_messages: 0,
            include_text_messages: false,
        },
    );
    assert_eq!(out.truncated_count, 1);
    let clipped = &out.messages[0].text;
    assert!(clipped.contains(MARKER));
    assert!(clipped.starts_with(&format!("{}\n", "x".repeat(9))));
    assert!(clipped.ends_with(&format!("\n{}", "q".repeat(9))));
}

fn stubbed_summary(summary: &str) -> String {
    let state = State {
        blocks: vec![Block {
            block_id: "b1".to_owned(),
            run_id: "r0".to_owned(),
            summary: "s".to_owned(),
            created_at: 1000,
            compress_call_id: Some("call1".to_owned()),
            start_ref: Some("m00001".to_owned()),
            end_ref: Some("m00050".to_owned()),
            ..Block::default()
        }],
        ..State::default()
    };
    let arguments =
        json!({"content": [{"startId": "m00001", "endId": "m00050", "summary": summary}]})
            .to_string();
    let hidden =
        hide_consumed_compress_calls(&state, &[call("mc1", "call1", "compress", &arguments)]);
    let parsed: Value = serde_json::from_str(&hidden.messages[0].text).expect("JSON arguments");
    parsed["content"][0]["summary"]
        .as_str()
        .expect("summary")
        .to_owned()
}

#[test]
fn a_summary_stub_drops_an_emoji_straddling_its_cut() {
    let stub = stubbed_summary(&format!("{}{EMOJI}tail", "a".repeat(198)));
    assert!(utf16_len(&stub) <= 200);
    assert!(stub.ends_with('\u{2026}'));
    assert!(!stub.contains(EMOJI));
}

#[test]
fn an_emoji_inside_the_stub_survives_intact() {
    let stub = stubbed_summary(&format!("{}{EMOJI}{}", "a".repeat(197), "b".repeat(10)));
    assert_eq!(utf16_len(&stub), 200);
    assert!(stub.contains(EMOJI));
}

#[test]
fn a_recap_preview_never_splits_an_emoji() {
    let state = State {
        blocks: vec![Block {
            block_id: "b1".to_owned(),
            run_id: "r0".to_owned(),
            summary: format!("{}{EMOJI}tail", "a".repeat(198)),
            effective_message_ids: vec!["m1".to_owned()],
            created_at: 1000,
            ..Block::default()
        }],
        ..State::default()
    };
    let recap = build_recap(&state, None);
    assert!(recap.contains("..."), "a long summary gets an ellipsis");
    assert!(recap.contains(&format!("  {}{EMOJI}...", "a".repeat(198))));
}

#[test]
fn a_derived_topic_never_splits_an_emoji() {
    let topic = derive_topic_from_summary(&format!("# {}{EMOJI} rest\nbody", "h".repeat(59)))
        .expect("a topic");
    assert!(utf16_len(&topic) <= 60);
    assert!(!topic.contains(EMOJI));
}

#[test]
fn a_topic_fallback_never_splits_an_emoji() {
    let topic = topic_fallback(&format!("h{}{EMOJI}rest of the sentence", "h".repeat(28)));
    assert!(topic.ends_with('\u{2026}'));
    assert!(!topic.contains(EMOJI));
    assert_eq!(
        topic_fallback(&format!("ok {EMOJI}")),
        format!("ok {EMOJI}")
    );
}

#[test]
fn a_raw_prefix_diagnostic_never_splits_an_emoji() {
    let raw = format!("{}{EMOJI}{}", "j".repeat(799), "k".repeat(50));
    let parsed = parse_compress_args(&Value::String(raw), None);
    let prefix = parsed.diagnostics.raw_prefix.expect("a raw prefix");
    assert_eq!(utf16_len(&prefix), 799);
}
