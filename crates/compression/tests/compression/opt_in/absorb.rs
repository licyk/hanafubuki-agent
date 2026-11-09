// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `absorb.test.ts` and the absorb case of `ccr.test.ts`. Not ported:
// the tool-name argument of the prompt builders (the name is fixed) and the
// `toolName` validation; the kernel's "mediaOnly ships neutral" case is the
// default here. Adds the refusal of a target an active block covers.

use serde_json::json;

use hanafubuki_agent_compression::{
    append_absorb_prompts, apply_absorb, build_absorb_prompt, build_absorb_system_prompt,
    hide_absorbed_messages, is_absorb_candidate, is_rendered_summary, parse_absorb_input,
    AbsorbConfig, AbsorbInput, AbsorbOutcome, Config, Kind, Message, ParsedAbsorb, State,
    ABSORB_PROMPT_MARKER,
};

use super::{has, text_of, turn};
use crate::fixtures::{apply, call, ids, range, result, state_for, user};

fn big_result(id: &str, call_id: &str, tool: &str) -> Message {
    result(id, call_id, tool, &"x".repeat(4800))
}

fn small_result(id: &str, call_id: &str, tool: &str) -> Message {
    result(id, call_id, tool, "ok")
}

fn tool_call(id: &str, call_id: &str, tool: &str) -> Message {
    call(id, call_id, tool, &format!("args-{call_id}"))
}

fn media_result(id: &str, call_id: &str, tool: &str) -> Message {
    Message {
        images: vec!["image-1".to_owned()],
        ..small_result(id, call_id, tool)
    }
}

fn session() -> Vec<Message> {
    vec![
        user("u1", "run the build"),
        tool_call("c1", "call1", "read"),
        big_result("r1", "call1", "read"),
        tool_call("c2", "call2", "bash"),
        small_result("r2", "call2", "bash"),
    ]
}

/// Absorption on, over text results (the media gate off).
fn absorbing(limit: u64) -> Config {
    let mut config = Config::new(limit);
    config.absorb = AbsorbConfig {
        enabled: true,
        min_tool_tokens: 1000,
        ..AbsorbConfig::default()
    };
    config
}

fn absorb(
    r#ref: &str,
    summary: &str,
    messages: &[Message],
    state: &State,
    config: &Config,
) -> AbsorbOutcome {
    apply_absorb(AbsorbInput {
        r#ref,
        summary,
        absorb_call_id: None,
        messages,
        state,
        config,
        now_ms: 0,
    })
}

fn ref_of(state: &State, id: &str) -> String {
    state.message_refs.by_raw[id].clone()
}

#[test]
fn prompts_are_off_while_absorption_is() {
    let messages = session();
    let state = state_for(&messages);
    let out = append_absorb_prompts(messages.clone(), &state, &Config::new(200_000), 5000);
    assert_eq!(out.prompted_count, 0);
    assert_eq!(out.messages, messages);
}

#[test]
fn a_large_result_gets_the_demand_citing_its_ref() {
    let messages = session();
    let state = state_for(&messages);
    let out = append_absorb_prompts(messages, &state, &absorbing(200_000), 5000);
    assert_eq!(out.prompted_count, 1);
    let r1 = text_of(&out.messages, "r1");
    assert!(r1.contains(ABSORB_PROMPT_MARKER));
    assert!(r1.contains(&format!("ref: \"{}\"", ref_of(&state, "r1"))));
    assert!(r1.starts_with(&"x".repeat(10)));
    assert!(!text_of(&out.messages, "r2").contains(ABSORB_PROMPT_MARKER));
}

#[test]
fn acp_protected_and_excluded_tools_are_skipped() {
    let messages = vec![
        tool_call("c1", "call1", "read"),
        big_result("r1", "call1", "read"),
        tool_call("c2", "call2", "search_context"),
        big_result("r2", "call2", "search_context"),
        tool_call("c3", "call3", "grep"),
        big_result("r3", "call3", "grep"),
        tool_call("c4", "call4", "absorb"),
        big_result("r4", "call4", "absorb"),
    ];
    let state = state_for(&messages);
    let mut config = absorbing(200_000);
    config.absorb.exclude_tools = vec!["gr*".to_owned()];
    config.protected_tools = vec!["read".to_owned()];
    let out = append_absorb_prompts(messages.clone(), &state, &config, 5000);
    assert_eq!(out.prompted_count, 0);
    for index in [1, 3, 5, 7] {
        assert!(!is_absorb_candidate(&messages[index], &config), "{index}");
    }
}

#[test]
fn the_context_usage_gate_holds_prompts_back() {
    let messages = session();
    let state = state_for(&messages);
    let mut config = absorbing(10_000);
    config.absorb.context_threshold_pct = 0.5;
    let below = append_absorb_prompts(messages.clone(), &state, &config, 4000);
    assert_eq!(below.prompted_count, 0);
    let at = append_absorb_prompts(messages, &state, &config, 5000);
    assert_eq!(at.prompted_count, 1);
}

#[test]
fn the_demand_is_never_appended_twice() {
    let messages = session();
    let state = state_for(&messages);
    let config = absorbing(200_000);
    let once = append_absorb_prompts(messages, &state, &config, 5000);
    let twice = append_absorb_prompts(once.messages, &state, &config, 5000);
    assert_eq!(twice.prompted_count, 0);
    assert_eq!(
        text_of(&twice.messages, "r1")
            .matches(ABSORB_PROMPT_MARKER)
            .count(),
        1
    );
}

#[test]
fn an_absorption_records_the_pair_and_counts_its_tokens() {
    let messages = session();
    let state = state_for(&messages);
    let config = absorbing(200_000);
    let outcome = apply_absorb(AbsorbInput {
        r#ref: &ref_of(&state, "r1"),
        summary: "build ok, 45 tests",
        absorb_call_id: Some("abs-call-1"),
        messages: &messages,
        state: &state,
        config: &config,
        now_ms: 42,
    });
    assert!(outcome.ok, "{}", outcome.result_text);
    let [record] = outcome.state.absorbed.as_slice() else {
        panic!("one record");
    };
    assert_eq!(record.tool_call_id, "call1");
    assert_eq!(record.call_message_id, "c1");
    assert_eq!(record.result_message_id, "r1");
    assert_eq!(record.absorb_call_id.as_deref(), Some("abs-call-1"));
    assert_eq!(record.created_at, 42);
    assert!(record.tokens_reclaimed >= 1000);
    assert_eq!(outcome.state.stats.absorbed_tokens, record.tokens_reclaimed);
    assert_eq!(
        outcome.result_text,
        "absorbed m00003 (~1.2K tokens → summary ~5). The original tool output is now hidden; your summary is the durable record."
    );
}

#[test]
fn bad_targets_are_refused() {
    let messages = vec![
        user("u1", "hi"),
        tool_call("c1", "call1", "read"),
        big_result("r1", "call1", "read"),
        tool_call("c2", "call2", "search_context"),
        big_result("r2", "call2", "search_context"),
        tool_call("c3", "call3", "todo"),
        big_result("r3", "call3", "todo"),
    ];
    let state = state_for(&messages);
    let mut config = absorbing(200_000);
    config.protected_tools = vec!["todo".to_owned()];
    let user_ref = ref_of(&state, "u1");
    let refused = |r#ref: &str, summary: &str| {
        let outcome = absorb(r#ref, summary, &messages, &state, &config);
        assert!(!outcome.ok);
        assert_eq!(outcome.state, state);
        outcome.result_text
    };
    assert_eq!(
        refused(&user_ref, "  "),
        "absorb failed: summary is empty — provide the distilled key info of the tool result."
    );
    assert_eq!(
        refused("m99999", "s"),
        "absorb failed: ref m99999 does not exist in this session (it may be hidden, already compressed, or stale)."
    );
    assert_eq!(
        refused(&user_ref, "s"),
        "absorb failed: ref m00001 is a text, not a tool result."
    );
    assert_eq!(
        refused(&ref_of(&state, "r2"), "s"),
        "absorb failed: search_context is an ACP-managed tool result — it is not absorbable."
    );
    assert_eq!(
        refused(&ref_of(&state, "r3"), "s"),
        "absorb failed: todo is a protected tool — its results must stay visible."
    );
}

#[test]
fn a_second_absorption_of_the_same_result_changes_nothing() {
    let messages = session();
    let state = state_for(&messages);
    let config = absorbing(200_000);
    let r#ref = ref_of(&state, "r1");
    let first = absorb(&r#ref, "s1", &messages, &state, &config);
    assert!(first.ok);
    let second = absorb(&r#ref, "s2", &messages, &first.state, &config);
    assert!(second.ok);
    assert_eq!(second.result_text, "already absorbed (m00003) — no change.");
    assert_eq!(second.state, first.state);
}

#[test]
fn a_summary_not_smaller_than_the_original_is_warned_about() {
    let messages = session();
    let state = state_for(&messages);
    let outcome = absorb(
        &ref_of(&state, "r1"),
        &"y".repeat(4800),
        &messages,
        &state,
        &absorbing(200_000),
    );
    assert!(outcome.ok);
    assert!(outcome.result_text.ends_with(
        " WARNING: your summary (~1.2K tokens) is not smaller than the original (~1.2K tokens) — distill harder next time."
    ));
}

#[test]
fn a_target_folded_into_a_block_is_not_visible() {
    let messages = session();
    let mut config = absorbing(200_000);
    config.compress.min_compress_range = 0;
    config.compress.min_summary_length = 0;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    let state = state_for(&messages);
    let fold = apply(
        &[range("m00002", "m00003", "read the file")],
        &messages,
        &state,
        &config,
    );
    assert_eq!(fold.result.blocks_created, 1);
    let outcome = absorb("m00003", "s", &messages, &fold.state, &config);
    assert!(!outcome.ok);
    assert_eq!(
        outcome.result_text,
        "absorb failed: ref m00003 is not visible in this session (hidden or compressed)."
    );
}

#[test]
fn hiding_drops_both_halves_of_the_pair_only() {
    let messages = session();
    let state = state_for(&messages);
    let absorbed = absorb(
        &ref_of(&state, "r1"),
        "s",
        &messages,
        &state,
        &absorbing(200_000),
    )
    .state;
    let visible = hide_absorbed_messages(messages, &absorbed);
    assert_eq!(ids(&visible), ["u1", "c2", "r2"]);
}

#[test]
fn a_turn_asks_then_hides_the_pair_and_keeps_the_absorb_call() {
    let config = absorbing(10_000);
    let mut messages = vec![
        user("u1", "read the file"),
        tool_call("c1", "call1", "read"),
        big_result("r1", "call1", "read"),
    ];
    let first = turn(&messages, State::default(), &config, 3000);
    assert!(text_of(&first.messages, "r1").contains(ABSORB_PROMPT_MARKER));
    let r#ref = ref_of(&first.state, "r1");
    messages.extend([
        tool_call("c2", "call2", "absorb"),
        result("r2", "call2", "absorb", "absorbed m00003"),
    ]);
    let outcome = apply_absorb(AbsorbInput {
        r#ref: &r#ref,
        summary: "file defines foo(a,b) at src/foo.ts:12",
        absorb_call_id: Some("call2"),
        messages: &messages,
        state: &first.state,
        config: &config,
        now_ms: 0,
    });
    assert!(outcome.ok);
    let second = turn(&messages, outcome.state, &config, 3200);
    assert!(!has(&second.messages, "c1"), "the call is hidden");
    assert!(!has(&second.messages, "r1"), "the result is hidden");
    assert!(has(&second.messages, "c2") && has(&second.messages, "r2"));
    assert!(second
        .messages
        .iter()
        .filter(|message| message.kind == Kind::ToolResult)
        .all(|message| !message.text.contains(ABSORB_PROMPT_MARKER)));
    assert!(!second.messages.iter().any(is_rendered_summary));
}

#[test]
fn absorbed_records_survive_a_fold() {
    let messages = vec![
        user("u1", "go"),
        tool_call("c1", "call1", "read"),
        big_result("r1", "call1", "read"),
        user("u2", "done"),
    ];
    let state = turn(&messages, State::default(), &Config::new(200_000), 100).state;
    let mut config = absorbing(200_000);
    config.compress.min_compress_range = 0;
    config.compress.min_summary_length = 0;
    let state = absorb(&ref_of(&state, "r1"), "s", &messages, &state, &config).state;
    let applied = apply(
        &[range("m00001", "m00002", "sum of the range")],
        &messages,
        &state,
        &config,
    );
    assert_eq!(applied.result.blocks_created, 1);
    assert_eq!(applied.state.absorbed, state.absorbed);
    assert_eq!(
        applied.state.stats.absorbed_tokens,
        state.stats.absorbed_tokens
    );
}

#[test]
fn absorb_arguments_are_read_leniently() {
    let parsed = |r#ref: &str, summary: &str| ParsedAbsorb {
        r#ref: r#ref.to_owned(),
        summary: summary.to_owned(),
    };
    assert_eq!(
        parse_absorb_input(&json!({"ref": "m00003", "summary": "s"})),
        Ok(parsed("m00003", "s"))
    );
    assert_eq!(
        parse_absorb_input(&json!({"of": "m00004", "content": "s2"})),
        Ok(parsed("m00004", "s2"))
    );
    assert_eq!(
        parse_absorb_input(&json!(r#"{"messageId":"m00005","summary":"s3"}"#)),
        Ok(parsed("m00005", "s3"))
    );
    for bad in [
        json!("not json"),
        json!({"ref": 1, "summary": "s"}),
        json!(null),
    ] {
        let error = parse_absorb_input(&bad).unwrap_err();
        assert!(error.starts_with("[acp-absorb-input]"), "{error}");
    }
}

#[test]
fn the_prompts_carry_the_tool_and_the_marker() {
    let prompt = build_absorb_prompt("m00007", 1234);
    assert_eq!(
        prompt,
        "[ACP absorb] This tool result (~1.2K tokens) will be REMOVED from context. Your IMMEDIATE next action: call absorb({ ref: \"m00007\", summary: \"...\" }) — summary = distilled essentials only (outcome, key values, exact paths:lines, error text verbatim, decisions). Afterwards work from your summary; do NOT re-run this tool. If the result contains nothing you need, call absorb with summary \"(nothing needed)\"."
    );
    let system = build_absorb_system_prompt();
    assert!(system.starts_with("INSTANT TOOL-RESULT ABSORPTION (absorb)\n\nSome tool results end with a [ACP absorb] instruction."));
    assert!(system.ends_with("absorb calls are ordinary context: the regular compression system may fold them later like any other message."));
}

#[test]
fn the_context_threshold_is_validated() {
    let mut config = Config::new(200_000);
    config.absorb.context_threshold_pct = 1.5;
    assert_eq!(
        config.validate(),
        ["absorb.contextThresholdPct must be in [0, 1]"]
    );
}

// --- the media gate (#3402) ---

#[test]
fn media_only_takes_only_results_with_images() {
    let text = big_result("r1", "call1", "read");
    let media = media_result("r2", "call2", "screenshot");
    let mut on = absorbing(200_000);
    on.absorb.media_only = true;
    let off = absorbing(200_000);
    assert!(!is_absorb_candidate(&text, &on));
    assert!(is_absorb_candidate(&media, &on));
    assert!(is_absorb_candidate(&text, &off));
    assert!(is_absorb_candidate(&media, &off));
}

#[test]
fn media_only_asks_for_small_image_results_and_skips_text() {
    let messages = vec![
        tool_call("c1", "call1", "read"),
        big_result("r1", "call1", "read"),
        tool_call("c2", "call2", "screenshot"),
        media_result("r2", "call2", "screenshot"),
    ];
    let state = state_for(&messages);
    let mut on = absorbing(200_000);
    on.absorb.media_only = true;
    let out = append_absorb_prompts(messages.clone(), &state, &on, 5000);
    assert!(!text_of(&out.messages, "r1").contains(ABSORB_PROMPT_MARKER));
    assert!(text_of(&out.messages, "r2").contains(ABSORB_PROMPT_MARKER));
    assert_eq!(out.prompted_count, 1);
    let out = append_absorb_prompts(messages, &state, &absorbing(200_000), 5000);
    assert!(text_of(&out.messages, "r1").contains(ABSORB_PROMPT_MARKER));
    assert!(!text_of(&out.messages, "r2").contains(ABSORB_PROMPT_MARKER));
}

#[test]
fn media_only_refuses_a_target_without_images() {
    let messages = vec![
        tool_call("c1", "call1", "read"),
        big_result("r1", "call1", "read"),
        tool_call("c2", "call2", "screenshot"),
        media_result("r2", "call2", "screenshot"),
    ];
    let state = state_for(&messages);
    let mut config = absorbing(200_000);
    config.absorb.media_only = true;
    let refused = absorb(&ref_of(&state, "r1"), "s", &messages, &state, &config);
    assert!(!refused.ok);
    assert!(refused.result_text.contains("no media payload"));
    let accepted = absorb(&ref_of(&state, "r2"), "s", &messages, &state, &config);
    assert!(accepted.ok, "{}", accepted.result_text);
    assert_eq!(accepted.state.absorbed.len(), 1);
}

#[test]
fn the_defaults_are_the_kernels() {
    let defaults = Config::new(100_000).absorb;
    assert!(!defaults.enabled);
    assert_eq!(defaults.min_tool_tokens, 4000);
    assert!(!defaults.media_only);
}
