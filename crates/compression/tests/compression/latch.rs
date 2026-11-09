// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Pins the contract of `issue3939-view-latch.test.ts` on the latch itself,
// with the transforms' output built by hand; the kernel's cases, through
// the transforms themselves, are in `opt_in/latch.rs`.

use serde_json::json;

use hanafubuki_agent_compression::{
    apply_view_latch, process_turn, view_latch_config_digest, Config, Message, RenderStrategy,
    State, TurnInput, TurnOutput, ViewLatch,
};

use crate::fixtures::{apply, assistant, call, config, range, result, state_for, user};

const DIGEST: &str = "{}";

fn history() -> Vec<Message> {
    let filler = "filler ".repeat(500);
    vec![
        user("raw-a", &format!("first question {filler}")),
        assistant("raw-b", &format!("answer {filler}")),
        user("raw-c", "second question"),
        assistant("raw-d", "second answer"),
    ]
}

fn rewritten(messages: &[Message], id: &str, text: &str) -> Vec<Message> {
    messages
        .iter()
        .map(|message| {
            if message.id == id {
                Message {
                    text: text.to_owned(),
                    ..message.clone()
                }
            } else {
                message.clone()
            }
        })
        .collect()
}

fn without(messages: &[Message], id: &str) -> Vec<Message> {
    messages
        .iter()
        .filter(|message| message.id != id)
        .cloned()
        .collect()
}

fn text_of<'a>(messages: &'a [Message], id: &str) -> Option<&'a str> {
    messages
        .iter()
        .find(|message| message.id == id)
        .map(|message| message.text.as_str())
}

#[test]
fn a_rewrite_is_latched_at_first_sight_and_replayed_after_the_transform_changes() {
    let pre = history();
    let mut state = state_for(&pre);
    let first = apply_view_latch(
        &pre,
        rewritten(&pre, "raw-c", "[redacted] question"),
        &mut state,
        DIGEST,
    );
    assert_eq!(text_of(&first, "raw-c"), Some("[redacted] question"));
    assert_eq!(
        state.view_latches["m00003"],
        ViewLatch {
            text: Some("[redacted] question".to_owned()),
            hidden: false,
        }
    );

    // The transform now decides otherwise; the latched bytes replay.
    let second = apply_view_latch(
        &pre,
        rewritten(&pre, "raw-c", "demoted"),
        &mut state,
        DIGEST,
    );
    assert_eq!(second, first);
    let unchanged = apply_view_latch(&pre, pre.clone(), &mut state, DIGEST);
    assert_eq!(
        text_of(&unchanged, "raw-c"),
        Some("[redacted] question"),
        "a transform that stops rewriting does not restore the bytes either"
    );
}

#[test]
fn a_message_hidden_at_first_sight_stays_hidden() {
    let pre = history();
    let mut state = state_for(&pre);
    // A rewrite elsewhere makes the latch worth recording.
    let post = rewritten(&without(&pre, "raw-b"), "raw-c", "changed");
    let first = apply_view_latch(&pre, post, &mut state, DIGEST);
    assert_eq!(text_of(&first, "raw-b"), None);
    assert!(state.view_latches["m00002"].hidden);

    let second = apply_view_latch(&pre, pre.clone(), &mut state, DIGEST);
    assert_eq!(
        text_of(&second, "raw-b"),
        None,
        "the hide waits for the next fold"
    );
}

#[test]
fn a_passthrough_latch_keeps_the_pre_transform_bytes() {
    let pre = history();
    let mut state = state_for(&pre);
    let post = rewritten(&pre, "raw-c", "changed");
    apply_view_latch(&pre, post, &mut state, DIGEST);
    assert_eq!(state.view_latches["m00004"], ViewLatch::default());

    // A later rewrite of a message latched as passthrough is deferred.
    let later = apply_view_latch(
        &pre,
        rewritten(
            &rewritten(&pre, "raw-c", "changed"),
            "raw-d",
            "rewritten late",
        ),
        &mut state,
        DIGEST,
    );
    assert_eq!(text_of(&later, "raw-d"), Some("second answer"));
}

#[test]
fn nothing_is_recorded_while_no_transform_rewrites() {
    let pre = history();
    let mut state = state_for(&pre);
    let before = state.clone();
    let out = apply_view_latch(&pre, without(&pre, "raw-b"), &mut state, DIGEST);
    assert_eq!(out, without(&pre, "raw-b"));
    assert_eq!(state, before);
}

#[test]
fn a_steady_state_leaves_the_state_untouched() {
    let pre = history();
    let mut state = state_for(&pre);
    let post = rewritten(&pre, "raw-c", "changed");
    let first = apply_view_latch(&pre, post.clone(), &mut state, DIGEST);
    let latched = state.clone();
    let second = apply_view_latch(&pre, post, &mut state, DIGEST);
    assert_eq!(second, first);
    assert_eq!(state, latched);
}

#[test]
fn a_digest_change_releases_the_latch() {
    let pre = history();
    let mut state = state_for(&pre);
    apply_view_latch(&pre, rewritten(&pre, "raw-c", "first"), &mut state, DIGEST);
    let out = apply_view_latch(
        &pre,
        rewritten(&pre, "raw-c", "second"),
        &mut state,
        "{\"crush\":true}",
    );
    assert_eq!(text_of(&out, "raw-c"), Some("second"));
    assert_eq!(
        state.view_latch_config_digest.as_deref(),
        Some("{\"crush\":true}")
    );
    assert_eq!(state.view_latches["m00003"].text.as_deref(), Some("second"));
}

#[test]
fn a_fold_releases_the_latch() {
    let pre = history();
    let mut state = state_for(&pre);
    apply_view_latch(
        &pre,
        rewritten(&pre, "raw-c", "[redacted]"),
        &mut state,
        DIGEST,
    );
    assert!(!state.view_latches.is_empty());
    let fold = apply(
        &[range(
            "m00001",
            "m00002",
            "a long enough summary for the fold gate to accept this range, over fifty chars",
        )],
        &pre,
        &state,
        &config(),
    );
    assert_eq!(fold.result.blocks_created, 1, "{:?}", fold.result.errors);
    assert!(fold.state.view_latches.is_empty());

    // After the fold the transform's fresh output lands.
    let mut state = fold.state;
    let out = apply_view_latch(&pre, rewritten(&pre, "raw-c", "fresh"), &mut state, DIGEST);
    assert_eq!(text_of(&out, "raw-c"), Some("fresh"));
}

fn turn(messages: &[Message], state: State, config: &Config) -> TurnOutput {
    process_turn(TurnInput {
        messages,
        state,
        config,
        token_count: 1000,
        token_count_source: None,
        render: RenderStrategy::None,
    })
}

/// A failed `compress` exchange folds like any message, so it has a
/// numbered ref; kept as an orphan, its long summary is stubbed.
fn failed_exchange_history() -> Vec<Message> {
    let arguments = json!({
        "content": [{"startId": "m00001", "endId": "m00002", "summary": "y".repeat(4000)}]
    })
    .to_string();
    vec![
        user("u1", "question"),
        assistant("a1", "answer"),
        call("fc", "failed", "compress", &arguments),
        result(
            "fr",
            "failed",
            "compress",
            "[Compression FAILED: no ranges applied]",
        ),
        user("u2", "next"),
        assistant("a2", "done"),
    ]
}

#[test]
fn process_turn_latches_compacted_compress_arguments() {
    let messages = failed_exchange_history();
    let config = Config::new(100_000);
    let first = turn(&messages, State::default(), &config);
    let stubbed = text_of(&first.messages, "fc").expect("kept").to_owned();
    assert!(
        stubbed.len() < messages[2].text.len(),
        "the summary is stubbed"
    );
    let r#ref = &first.state.message_refs.by_raw["fc"];
    assert_eq!(
        first.state.view_latches[r#ref].text.as_deref(),
        Some(stubbed.as_str())
    );
    assert_eq!(
        first.state.view_latch_config_digest.as_deref(),
        Some(view_latch_config_digest(&config).as_str())
    );

    let second = turn(&messages, first.state.clone(), &config);
    assert_eq!(text_of(&second.messages, "fc"), Some(stubbed.as_str()));
    assert_eq!(second.state.view_latches, first.state.view_latches);
}
