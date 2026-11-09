//! Golden baselines of the `hierarchical` strategy: a session the model
//! folds by itself, from the nudge to a tier-2 block, and the request once
//! the session switched back to `standard` (`golden.rs` holds the helpers).
//! Linux-only, as the fixtures are.

use super::golden::{assert_golden, normalize};
use super::*;
use crate::{AgentCompactionSettings, AgentCompactionStrategy, AgentConfig};
use serde_json::Value;

const SUMMARY: &str = "The user asked and the assistant answered at length; nothing in it is still open, and no file was changed.";

/// A model-scripted session over the HTTP fake: two long answers, the
/// nudge over the pressure band, two folds, their tier-2 distillation, a
/// `decompress`, then the switch to `standard`, which keeps the blocks
/// projecting without the prompt, the tools or the nudge. The `standard`
/// goldens stay as they are: nothing here touches a `standard` session.
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
    for prompt in ["Hello.", "Go on.", "Fold what is done."] {
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

/// The `[compaction]` table the session runs with: `hierarchical`, only the
/// newest message kept out of a fold.
fn tuned() -> AgentCompactionSettings {
    let mut settings = AgentCompactionSettings {
        strategy: AgentCompactionStrategy::Hierarchical,
        ..AgentCompactionSettings::default()
    };
    settings.hierarchical.preserve_recent_messages = 1;
    settings.hierarchical.preserve_recent_tokens = 0;
    settings.hierarchical.min_compress_range_chars = 1_000;
    settings
}

fn configure(config: &AgentConfig, base_url: &str) {
    configure_test_provider(config, base_url, None);
    config
        .mutate_providers(1, |registry| {
            registry.model_policy.context_window_tokens = Some(100_000);
            Ok(())
        })
        .unwrap();
    config
        .write_settings(AgentSettingsPatch {
            compaction: Some(tuned()),
            ..AgentSettingsPatch::default()
        })
        .unwrap();
}

/// A call of `name` with `arguments`, as `call_id`.
fn call_events(call_id: &str, name: &str, arguments: &Value) -> Vec<u8> {
    String::from_utf8(tool_events(name, &arguments.to_string()))
        .unwrap()
        .replace("call-tool", call_id)
        .into_bytes()
}

/// `events` with the provider's count of the request's input tokens.
fn with_usage(events: Vec<u8>, input_tokens: u64) -> Vec<u8> {
    let usage = format!("\"usage\":{{\"input_tokens\":{input_tokens},\"output_tokens\":1}}");
    String::from_utf8(events)
        .unwrap()
        .replace(
            "\"end_turn\":true}",
            &format!("\"end_turn\":true,{usage}}}"),
        )
        .replace(
            "\"end_turn\":false}",
            &format!("\"end_turn\":false,{usage}}}"),
        )
        .into_bytes()
}
