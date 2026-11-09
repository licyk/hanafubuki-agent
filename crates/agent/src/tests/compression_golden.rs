//! Golden baselines of the `hierarchical` strategy: a session the model
//! folds by itself, from the nudge to a tier-2 block, and the request once
//! the session switched back to `standard` (`golden.rs` and `compression.rs`
//! hold the helpers).
//! Linux-only, as the fixtures are.

use super::compression::{call_events, configure, tuned, with_usage};
use super::golden::{assert_golden, normalize};
use super::*;
use crate::{AgentCompactionSettings, AgentCompactionStrategy};
use serde_json::Value;

const SUMMARY: &str = "The user asked and the assistant answered at length; nothing in it is still open, and no file was changed.";

/// A model-scripted session over the HTTP fake: two long answers, the
/// nudge over the pressure band, two folds, their tier-2 distillation, a
/// `decompress`, then the switch to `standard`, which keeps the blocks
/// projecting without the prompt, the tools or the nudge. Each turn reads a
/// fixed clock, a minute after the last, so the tags' times and the
/// receipts' spans are the same on every run. The `standard` goldens stay
/// as they are: nothing here touches a `standard` session.
#[tokio::test]
async fn hierarchical_requests_match_golden() {
    let first = "The first answer is long. ".repeat(1_000);
    let second = "The second answer is long too. ".repeat(1_000);
    let server = FakeServer::start(
        [
            with_usage(assistant_events(&first), 1_000),
            with_usage(assistant_events(&second), 80_000),
            with_usage(
                call_events(
                    "fold-1",
                    "compress",
                    &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
                ),
                80_000,
            ),
            with_usage(
                call_events(
                    "fold-2",
                    "compress",
                    &json!({ "content": format!("m00004–m00004 Second answer\n{SUMMARY}") }),
                ),
                60_000,
            ),
            with_usage(
                call_events(
                    "distil",
                    "compress",
                    &json!({ "content": format!("b1–b2 Both answers\n{SUMMARY}") }),
                ),
                40_000,
            ),
            with_usage(
                call_events("restore", "decompress", &json!({ "blockId": "b1" })),
                20_000,
            ),
            with_usage(assistant_events("Folded."), 20_000),
            with_usage(assistant_events("Still here."), 20_000),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths) = service_with("golden-hierarchical", |_, config| {
        configure(config, &server.base_url)
    });
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let clock = |minute: u64| {
        service
            .core
            .test_fix_message_clock(
                session.id,
                Some(hanafubuki_agent_kernel::AgentLocalTime {
                    // 2026-11-09 09:00:00 at +08:00, then a minute a turn.
                    at: 1_794_186_000_000 + minute * 60_000,
                    utc_offset_seconds: 8 * 3600,
                }),
            )
            .unwrap()
    };
    for (minute, prompt) in ["Hello.", "Go on.", "Fold what is done."]
        .into_iter()
        .enumerate()
    {
        clock(minute as u64);
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }
    service
        .write_settings(AgentSettingsPatch {
            compaction: Some(AgentCompactionSettings {
                strategy: AgentCompactionStrategy::Standard,
                ..tuned()
            }),
            ..AgentSettingsPatch::default()
        })
        .await
        .unwrap();
    clock(3);
    service
        .start_turn(session.id, "Are you there?".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 8);
    let body = |index: usize| -> Value { serde_json::from_slice(&requests[index].body).unwrap() };
    let tool_output = |index: usize, call_id: &str| -> String {
        body(index)["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["call_id"] == call_id && item["type"] == "function_call_output")
            .map(|item| item["output"].to_string())
            .unwrap_or_default()
    };
    assert!(tool_output(3, "fold-1").contains("[Compressed m00002–m00002 → 1 block(s)"));
    assert!(tool_output(5, "distil").contains("[Compressed b1–b2 → 1 block(s)"));
    assert!(tool_output(6, "restore").contains("[Block b1 content — "));
    let after_switch = body(7);
    assert!(!after_switch["instructions"]
        .as_str()
        .unwrap()
        .contains("ACP TAGS"));
    // Folded: the first answer comes back only as what `decompress` returned.
    assert!(!after_switch["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["role"] == "assistant" && item.to_string().contains(&first)));

    let pinned = json!({
        "nudged_request": body(2),
        "after_fold": body(3)["input"],
        "after_distillation": body(5)["input"],
        "after_switch_away": after_switch,
    });
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let normalized = normalize(&pinned, &paths.app_data_dir);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    // The long answers stand for themselves once: the golden pins where
    // they go, not their bytes again.
    let shortened = serde_json::to_string(&normalized)
        .unwrap()
        .replace(&first, "<FIRST ANSWER>")
        .replace(&second, "<SECOND ANSWER>");
    assert_golden(
        "hierarchical_requests",
        &serde_json::from_str(&shortened).unwrap(),
    );
}
