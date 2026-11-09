// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the `processTurn` and node cases of `crush.test.ts`.

use hanafubuki_agent_compression::{
    apply_crush_to_messages, Config, Message, State, ABSORB_PROMPT_MARKER,
};

use super::crush::{request_rows, service_rows};
use super::{text_of, turn};
use crate::fixtures::{call, user};

fn messages(output: &str) -> Vec<Message> {
    vec![
        user("u1", "run the job"),
        call("a-tc", "call_1", "bash", r#"{"command":"job"}"#),
        Message {
            tool_name: None,
            ..crate::fixtures::result("t-res", "call_1", "", output)
        },
    ]
}

fn config(absorb: bool, min_tool_tokens: u64, crush: bool) -> Config {
    let mut config = Config::new(200_000);
    config.absorb.enabled = absorb;
    config.absorb.min_tool_tokens = min_tool_tokens;
    config.crush.enabled = crush;
    config
}

fn texts(messages: &[Message]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message.text.as_str())
        .collect()
}

#[test]
fn either_switch_off_leaves_the_view() {
    let rows: Vec<String> = (0..10)
        .map(|index| format!(r#"{{"id":{index},"ts":"2026-09-21T12:00:00Z","level":"INFO"}}"#))
        .collect();
    let source = format!("[{}]", rows.join(","));
    for config in [
        config(true, 100_000, false),
        config(true, 100_000, true),
        config(false, 4000, true),
    ] {
        let out = turn(&messages(&source), State::default(), &config, 0);
        assert_eq!(
            texts(&out.messages),
            ["run the job", r#"{"command":"job"}"#, source.as_str()]
        );
    }
}

#[test]
fn a_result_under_the_gate_is_never_touched() {
    let small = r#"[{"id":1,"zone":"z1"}]"#;
    let out = turn(
        &messages(small),
        State::default(),
        &config(true, 100, true),
        0,
    );
    assert_eq!(
        texts(&out.messages),
        ["run the job", r#"{"command":"job"}"#, small]
    );
}

#[test]
fn a_result_crushed_under_the_gate_needs_no_absorption() {
    let source = service_rows(10);
    let out = turn(
        &messages(&source),
        State::default(),
        &config(true, 100, true),
        0,
    );
    let crushed: serde_json::Value = serde_json::from_str(text_of(&out.messages, "t-res")).unwrap();
    assert_eq!(crushed["__acp_crush"], "rows");
    assert!(out
        .messages
        .iter()
        .all(|message| !message.text.contains(ABSORB_PROMPT_MARKER)));
    assert_eq!(
        texts(&out.messages[..2]),
        ["run the job", r#"{"command":"job"}"#]
    );
}

#[test]
fn a_result_still_over_the_gate_is_crushed_and_asked_for() {
    let out = turn(
        &messages(&request_rows()),
        State::default(),
        &config(true, 100, true),
        0,
    );
    let text = text_of(&out.messages, "t-res");
    let marker = text.find(ABSORB_PROMPT_MARKER).expect("asked for");
    let crushed: serde_json::Value = serde_json::from_str(text[..marker].trim_end()).unwrap();
    assert_eq!(crushed["__acp_crush"], "rows");
    assert!(text[..marker].len() / 4 >= 100);
}

#[test]
fn a_log_crushed_under_the_gate_keeps_its_errors() {
    let mut lines = vec!["INFO boot ok"];
    lines.extend(["warn retry x"; 55]);
    lines.extend([
        "ERROR db connection lost",
        "ERROR transaction aborted",
        "Summary: 2 errors, 55 warnings",
    ]);
    let log = lines.join("\n");
    let out = turn(
        &messages(&log),
        State::default(),
        &config(true, 150, true),
        0,
    );
    let text = text_of(&out.messages, "t-res");
    assert!(text.len().div_ceil(4) < 150);
    assert!(text.contains("ERROR db connection lost"));
    assert!(text.contains("ERROR transaction aborted"));
    assert!(!text.contains(ABSORB_PROMPT_MARKER));
}

#[test]
fn a_stale_demand_is_dropped_and_decided_again() {
    let source = service_rows(10);
    let first = turn(
        &messages(&source),
        State::default(),
        &config(true, 100, false),
        0,
    );
    assert!(text_of(&first.messages, "t-res").contains(ABSORB_PROMPT_MARKER));
    let second = turn(&first.messages, first.state, &config(true, 100, true), 0);
    let text = text_of(&second.messages, "t-res");
    assert!(text.contains("__acp_crush"));
    assert!(!text.contains(ABSORB_PROMPT_MARKER));
}

#[test]
fn the_step_counts_what_it_did() {
    let out = apply_crush_to_messages(messages(&service_rows(10)), &config(true, 100, true), 0);
    assert_eq!((out.crushed_count, out.distilled_count), (1, 0));
    assert!(text_of(&out.messages, "t-res").contains("__acp_crush"));
}
