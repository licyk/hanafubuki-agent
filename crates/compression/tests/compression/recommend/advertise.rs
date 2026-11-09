// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the recommendation cases of `issue4289-failed-compress-fold.test.ts`,
// `issue5531-stranded-reasoning-residue.test.ts` and
// `protected-latest.test.ts` (their apply and protection cases are in
// `merge.rs`, `integrity.rs` and `protect.rs`).

use std::collections::HashSet;

use hanafubuki_agent_compression::{
    build_compressible_ranges, Config, Message, Role, State, BLOCKED_REF,
};

use super::{ranges_of, ref_of, turn, unnamed_result, user};
use crate::fixtures::{apply, assistant, call, range, reasoning, result, state_for};

// --- #4289: a failed compress exchange is advertised like any message ---

fn failed_fold_config() -> Config {
    let mut config = Config::new(200_000);
    config.compress.min_compress_range = 0;
    config.compress.max_summary_length = 0;
    config.compress.min_summary_length = 0;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config.protected_tools = vec!["nomatch-*".to_owned()];
    config
}

const VALID_SUMMARY: &str = "A meaningful summary that captures the key information of the compressed range including file paths and decisions.";

fn compress_args(start: &str, end: &str) -> String {
    serde_json::json!({ "content": [{ "startId": start, "endId": end, "summary": VALID_SUMMARY }] })
        .to_string()
}

fn failed_and_successful_exchanges() -> Vec<Message> {
    let long = "x".repeat(6000);
    vec![
        user("u1", &long),
        call("fc1", "call-failed", "compress", &compress_args("m00001", "m00002")),
        result(
            "fr1",
            "call-failed",
            "compress",
            "[Compression FAILED: Requested range(s) cannot be anchored. Nothing compressed. [cause: gate:none-resolved]]",
        ),
        user("u2", &long),
        call("sc1", "call-ok", "compress", &compress_args("m00001", "m00001")),
        result(
            "sr1",
            "call-ok",
            "compress",
            "[Compressed m00001–m00001 → 1 block(s), ~1234 tokens saved.]",
        ),
        user("u3", &long),
    ]
}

#[test]
fn a_failed_compress_exchange_is_advertised_and_a_successful_one_is_not() {
    let config = failed_fold_config();
    let messages = failed_and_successful_exchanges();
    let out = turn(&messages, State::default(), &config, 1000);
    assert_eq!(out.state.message_refs.by_raw["sc1"], BLOCKED_REF);
    let ranges = ranges_of(&messages, &out.state, &config);
    assert!(
        ranges.compressible.iter().any(|advertised| {
            advertised.start_ref == ref_of(&out.state, "u1")
                && advertised.end_ref == ref_of(&out.state, "fr1")
                && advertised.count == 3
        }),
        "{:?}",
        ranges.compressible
    );
    assert!(!ranges
        .protected
        .iter()
        .any(|protected| protected.start_ref == ref_of(&out.state, "fc1")));
    assert_eq!(
        ranges
            .compressible
            .iter()
            .map(|advertised| advertised.count)
            .sum::<usize>(),
        5,
        "the successful exchange's blocked halves take no advertised slot"
    );
}

// --- #5531: a covered call cannot strand its turn's reasoning ---

fn stranded_config() -> Config {
    let mut config = Config::new(150_000);
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config.compress.min_compress_range = 1;
    config.compress.min_summary_length = 1;
    config
}

fn sized(message: Message, body: String) -> Message {
    Message {
        text: body,
        ..message
    }
}

/// `[reasoning, text, call, result]` flanked by user messages.
fn strict_thinking_turn() -> Vec<Message> {
    let user_of = |id: &str| user(id, &format!("user-{id} {}", "x".repeat(200)));
    vec![
        user_of("u1"),
        sized(reasoning("r", ""), format!("think-r {}", "t".repeat(400))),
        sized(assistant("t", ""), format!("text-t {}", "x".repeat(200))),
        call("c", "ca", "pwsh", "call-ca"),
        Message {
            role: Role::User,
            ..result(
                "res",
                "ca",
                "pwsh",
                &format!("result-ca {}", "r".repeat(200)),
            )
        },
        user_of("u6"),
        user_of("u7"),
    ]
}

/// Folds the call and its result alone, leaving reasoning and text visible.
fn reverse_split(messages: &[Message]) -> State {
    let seed = apply(
        &[range("m00004", "m00005", &"s".repeat(400))],
        messages,
        &state_for(messages),
        &stranded_config(),
    );
    assert!(seed.result.errors.is_empty(), "{:?}", seed.result.errors);
    seed.state
}

fn advertises(ranges: &[hanafubuki_agent_compression::CompressibleRange], r#ref: &str) -> bool {
    ranges.iter().any(|advertised| {
        advertised.start_ref.as_str() <= r#ref && r#ref <= advertised.end_ref.as_str()
    })
}

#[test]
fn the_stranded_reasoning_and_text_are_advertised_again() {
    let messages = strict_thinking_turn();
    let state = reverse_split(&messages);
    let ranges = ranges_of(&messages, &state, &stranded_config());
    assert!(advertises(&ranges.compressible, ref_of(&state, "r")));
    assert!(advertises(&ranges.compressible, ref_of(&state, "t")));
}

#[test]
fn the_advertised_stranded_range_folds_without_a_split_error() {
    let messages = strict_thinking_turn();
    let state = reverse_split(&messages);
    let ranges = ranges_of(&messages, &state, &stranded_config());
    let target = ranges
        .compressible
        .iter()
        .find(|advertised| advertises(std::slice::from_ref(*advertised), ref_of(&state, "r")))
        .expect("the stranded pair is recommended");
    let outcome = apply(
        &[range(
            &target.start_ref,
            &target.end_ref,
            "reclaiming the stranded turn head",
        )],
        &messages,
        &state,
        &stranded_config(),
    );
    assert!(
        outcome.result.errors.is_empty(),
        "{:?}",
        outcome.result.errors
    );
    assert!(outcome.result.blocks_created >= 1);
    let recovery = outcome
        .state
        .active_blocks()
        .find(|block| block.block_id != "b1")
        .expect("a recovery block beside the seed");
    assert!(recovery.effective_message_ids.contains(&"r".to_owned()));
    assert!(recovery.effective_message_ids.contains(&"t".to_owned()));
}

#[test]
fn the_pair_is_not_advertised_while_its_call_is_visible() {
    let messages = strict_thinking_turn();
    let state = state_for(&messages);
    let zone: HashSet<String> = [ref_of(&state, "c"), ref_of(&state, "res")]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let ranges = build_compressible_ranges(
        &messages,
        &state,
        &stranded_config(),
        Some(&zone),
        &messages,
    );
    assert!(!advertises(&ranges.compressible, ref_of(&state, "r")));
    assert!(!advertises(&ranges.compressible, ref_of(&state, "t")));
}

// --- protectedLatestTools: only the latest pair is protected ---

fn todo_call(id: &str, call_id: &str, revision: u32) -> Message {
    call(
        id,
        call_id,
        "todo_list",
        &serde_json::json!({ "action": "update", "revision": revision }).to_string(),
    )
}

fn todo_result(id: &str, call_id: &str, revision: u32) -> Message {
    unnamed_result(
        id,
        call_id,
        &serde_json::json!({
            "revision": revision,
            "todos": [{ "id": "t1", "content": format!("task {revision}"), "status": "pending" }],
        })
        .to_string(),
    )
}

#[test]
fn the_latest_pair_is_protected_and_the_older_pair_compressible() {
    let messages = [
        user("a", &"x".repeat(2000)),
        todo_call("c1", "t1", 1),
        todo_result("r1", "t1", 1),
        user("b", &"x".repeat(2000)),
        todo_call("c2", "t2", 2),
        todo_result("r2", "t2", 2),
    ];
    let mut config = failed_fold_config();
    config.protected_tools.clear();
    config.protected_latest_tools = vec!["todo_list".to_owned()];
    let ranges = ranges_of(&messages, &state_for(&messages), &config);
    assert_eq!(
        super::spans(&ranges.compressible),
        [
            super::span("m00001", "m00003", 3),
            super::span("m00004", "m00004", 1)
        ]
    );
    assert_eq!(ranges.protected.len(), 1);
    let protected = &ranges.protected[0];
    assert_eq!(
        (protected.start_ref.as_str(), protected.end_ref.as_str()),
        ("m00005", "m00006")
    );
    assert_eq!(protected.tools, ["todo_list"]);
}
