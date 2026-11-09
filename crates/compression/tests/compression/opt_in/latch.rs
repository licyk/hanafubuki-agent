// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `issue3939-view-latch.test.ts` through the transforms themselves
// (`latch.rs` pins the latch on hand-made transform output): a transform's
// new decision about an old message waits for a fold, an absorption or a
// change of the transforms' settings; token pressure alone never rewrites
// bytes already sent.

use hanafubuki_agent_compression::{
    apply_absorb, view_latch_config_digest, AbsorbInput, Config, ContentStore, FilterAction,
    Message, MessageFilterContext, State,
};

use super::filter::{filters_on, Filter};
use super::{has, text_of, turn, turn_with};
use crate::fixtures::{apply, assistant, call, range, result, user};

fn config() -> Config {
    let mut config = Config::new(100_000);
    config.preserve_recent_tokens = 100;
    config
}

fn history() -> Vec<Message> {
    let filler = "filler ".repeat(500);
    vec![
        user("raw-a", &format!("first question {filler}")),
        assistant("raw-b", &format!("answer {filler}")),
        user("raw-c", "second question"),
        assistant("raw-d", "second answer"),
        user("raw-e", "third question"),
        assistant("raw-f", "third answer"),
    ]
}

#[test]
fn a_keep_last_only_demotion_waits_for_the_fold() {
    let redact = Filter {
        name: "pin-marker",
        keep_last_only: true,
        decide: |context: &MessageFilterContext<'_>| {
            if context.text.contains("SECRETTOKEN") {
                FilterAction::Modify(context.text.replace("SECRETTOKEN", "[redacted]"))
            } else {
                FilterAction::Keep
            }
        },
    };
    let mut config = config();
    config.message_filters = filters_on(&["pin-marker"]);
    let mut messages = history();
    messages.push(user("raw-occ1", "here is SECRETTOKEN one"));
    let (first, _) = turn_with(
        &messages,
        State::default(),
        &config,
        1000,
        ContentStore::default(),
        &[&redact],
    );
    let occurrence = text_of(&first.messages, "raw-occ1").to_owned();
    assert_eq!(
        occurrence, "here is [redacted] one",
        "the last occurrence is filtered on arrival"
    );

    messages.push(user("raw-occ2", "again SECRETTOKEN two"));
    let (second, _) = turn_with(
        &messages,
        first.state,
        &config,
        1000,
        ContentStore::default(),
        &[&redact],
    );
    assert_eq!(
        text_of(&second.messages, "raw-occ1"),
        occurrence,
        "the demotion waits"
    );
    assert_eq!(
        text_of(&second.messages, "raw-occ2"),
        "again [redacted] two"
    );

    let fold = apply(
        &[range(
            &second.state.message_refs.by_raw["raw-a"],
            &second.state.message_refs.by_raw["raw-b"],
            "a long enough summary for the fold gate to accept this range, over fifty chars",
        )],
        &messages,
        &second.state,
        &config,
    );
    assert_eq!(fold.result.blocks_created, 1, "{:?}", fold.result.errors);
    assert!(
        fold.state.view_latches.is_empty(),
        "a fold releases the latch"
    );
}

#[test]
fn an_absorption_hides_its_pair_on_the_next_request() {
    let mut config = Config::new(100_000);
    config.absorb.enabled = true;
    config.absorb.min_tool_tokens = 1;
    let messages = vec![
        user("u1", "read the file"),
        call("c1", "call1", "read", "{}"),
        result(
            "r1",
            "call1",
            "read",
            &format!("file body line one\n{}", "x".repeat(400)),
        ),
    ];
    let first = turn(&messages, State::default(), &config, 3000);
    assert!(has(&first.messages, "r1"));
    let outcome = apply_absorb(AbsorbInput {
        r#ref: &first.state.message_refs.by_raw["r1"],
        summary: "file defines foo(a,b) at src/foo.ts:12",
        absorb_call_id: None,
        messages: &messages,
        state: &first.state,
        config: &config,
        now_ms: 0,
    });
    assert!(outcome.ok);
    let second = turn(&messages, outcome.state, &config, 3200);
    assert!(!has(&second.messages, "r1") && !has(&second.messages, "c1"));
}

#[test]
fn turning_crush_on_applies_at_once_and_pressure_rewrites_nothing() {
    let rows: Vec<String> = (0..40)
        .map(|index| {
            format!(
                r#"{{"id":{},"ts":"2026-09-21T12:00:00Z","level":"INFO","name":"svc-{index}"}}"#,
                index * 7
            )
        })
        .collect();
    let messages = vec![
        user("u1", "read the log"),
        call("c1", "call1", "read", "{}"),
        result("r1", "call1", "read", &format!("[{}]", rows.join(","))),
    ];
    let mut off = Config::new(100_000);
    off.absorb.enabled = true;
    off.absorb.min_tool_tokens = 100;
    let first = turn(&messages, State::default(), &off, 0);
    assert!(!text_of(&first.messages, "r1").contains("__acp_crush"));

    let mut on = off.clone();
    on.crush.enabled = true;
    let second = turn(&messages, first.state, &on, 0);
    let crushed = text_of(&second.messages, "r1").to_owned();
    assert!(
        crushed.contains("__acp_crush"),
        "a settings change applies now"
    );

    let third = turn(&messages, second.state, &on, 99_000);
    assert_eq!(
        text_of(&third.messages, "r1"),
        crushed,
        "pressure replays the latched bytes"
    );
}

#[test]
fn turning_the_content_store_off_brings_the_raw_bytes_back() {
    let big = "line of build output ".repeat(400);
    let messages = vec![
        user("u1", "run a big build"),
        call("c1", "call1", "bash", r#"{"command":"npm run build"}"#),
        result("r1", "call1", "bash", &big),
    ];
    let mut armed = config();
    armed.content_store.enabled = true;
    armed.content_store.min_tool_tokens = 50;
    let (first, store) = turn_with(
        &messages,
        State::default(),
        &armed,
        0,
        ContentStore::default(),
        &[],
    );
    let placeholder = text_of(&first.messages, "r1").to_owned();
    assert!(placeholder.contains("[acp-stored") && !placeholder.contains("line of build output"));

    let (second, store) = turn_with(&messages, first.state, &armed, 0, store, &[]);
    assert_eq!(text_of(&second.messages, "r1"), placeholder);

    let (third, _) = turn_with(&messages, second.state, &config(), 0, store, &[]);
    let raw = text_of(&third.messages, "r1");
    assert!(raw.contains("line of build output") && !raw.contains("[acp-stored"));
}

#[test]
fn only_the_transforms_that_are_on_are_in_the_digest() {
    let defaults = Config::new(100_000);
    assert_eq!(view_latch_config_digest(&defaults), "{}");
    let mut idle = defaults.clone();
    idle.absorb.min_tool_tokens = 1;
    idle.crush.enabled = true;
    idle.content_store.max_head_chars = 10;
    assert_eq!(
        view_latch_config_digest(&idle),
        "{}",
        "crush needs absorption on"
    );
    let mut on = defaults.clone();
    on.content_store.enabled = true;
    let digest = view_latch_config_digest(&on);
    assert!(
        digest.starts_with(r#"{"contentStore":{"enabled":true"#),
        "{digest}"
    );
    on.content_store.min_tool_tokens = 50;
    assert_ne!(view_latch_config_digest(&on), digest);
}
