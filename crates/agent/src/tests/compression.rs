//! The `hierarchical` strategy through the facade: switching away from it
//! between turns and editing an earlier message of a folded session. The
//! helpers here serve `compression_golden.rs` too.

use super::*;
use crate::{AgentCompactionSettings, AgentCompactionStrategy, AgentConfig};
use serde_json::Value;

const SUMMARY: &str = "The user asked and the assistant answered at length; nothing in it is still open, and no file was changed.";

/// A session switched to `standard` keeps projecting its blocks: the
/// request carries the summary and the `compress` call that holds it, but
/// not what it folded, and no prompt, tool or nudge of the strategy. The
/// next standard compaction ends the blocks.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn switching_to_standard_keeps_the_blocks_until_a_compaction() {
    let first = "The first answer is long. ".repeat(1_000);
    let server = FakeServer::start(
        [
            with_usage(assistant_events(&first), 1_000),
            with_usage(
                call_events(
                    "fold-1",
                    "compress",
                    &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
                ),
                1_000,
            ),
            with_usage(assistant_events("Folded."), 1_000),
            with_usage(assistant_events("Standard now."), 1_000),
            assistant_events("The user asked twice; the first answer was folded."),
            with_usage(assistant_events("Compacted."), 1_000),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths) = service_with("compression-switch-away", |_, config| {
        configure(config, &server.base_url)
    });
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    for prompt in ["Hello.", "Fold it."] {
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
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    service
        .start_turn(session.id, "And now?".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 6);
    let body = |index: usize| -> Value { serde_json::from_slice(&requests[index].body).unwrap() };
    let carried = body(3);
    let input = carried["input"].as_array().unwrap();
    assert!(!carried["instructions"]
        .as_str()
        .unwrap()
        .contains("ACP TAGS"));
    assert!(carried["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .all(|tool| tool["name"] != "compress" && tool["name"] != "decompress"));
    assert!(input
        .iter()
        .any(|item| item["type"] == "function_call" && item["name"] == "compress"));
    assert!(carried.to_string().contains("</acp>"));
    assert!(!carried.to_string().contains(&first));
    assert!(input.last().unwrap().to_string().contains("Are you there?"));

    let compaction = body(4).to_string();
    assert!(!compaction.contains(&first));
    assert!(compaction.contains("First answer"));
    let after = body(5).to_string();
    assert!(!after.contains("</acp>"));
    assert!(!after.contains(&first));
    finish(service, paths);
}

/// Editing a message after a fold forks the session before that message's
/// turn with the block, which keeps projecting the fork.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn editing_a_later_message_keeps_the_fold() {
    let first = "The first answer is long. ".repeat(1_000);
    let server = FakeServer::start(
        [
            with_usage(assistant_events(&first), 1_000),
            with_usage(
                call_events(
                    "fold-1",
                    "compress",
                    &json!({ "content": format!("m00002–m00002 First answer\n{SUMMARY}") }),
                ),
                1_000,
            ),
            with_usage(assistant_events("Folded."), 1_000),
            with_usage(assistant_events("Later."), 1_000),
            with_usage(assistant_events("Edited."), 1_000),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let (service, paths) = service_with("compression-edit", |_, config| {
        configure(config, &server.base_url)
    });
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    for prompt in ["Hello.", "Fold it.", "Later question."] {
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }
    let last_turn = service.snapshot(session.id).unwrap().turn.unwrap().id;

    let edited = service.edit_from_turn(session.id, last_turn).unwrap();
    assert_eq!(edited.restored_prompt, "Later question.");
    service
        .start_turn(edited.session.id, "Edited question.".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 5);
    let fork: Value = serde_json::from_slice(&requests[4].body).unwrap();
    let text = fork.to_string();
    assert!(!text.contains(&first));
    assert!(text.contains("First answer"));
    assert!(fork["instructions"].as_str().unwrap().contains("ACP TAGS"));
    finish(service, paths);
}

/// The `[compaction]` table the sessions run with: `hierarchical`, only the
/// newest message kept out of a fold.
pub(super) fn tuned() -> AgentCompactionSettings {
    let mut settings = AgentCompactionSettings {
        strategy: AgentCompactionStrategy::Hierarchical,
        ..AgentCompactionSettings::default()
    };
    settings.hierarchical.preserve_recent_messages = 1;
    settings.hierarchical.preserve_recent_tokens = 0;
    settings.hierarchical.min_compress_range_chars = 1_000;
    settings
}

pub(super) fn configure(config: &AgentConfig, base_url: &str) {
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
pub(super) fn call_events(call_id: &str, name: &str, arguments: &Value) -> Vec<u8> {
    String::from_utf8(tool_events(name, &arguments.to_string()))
        .unwrap()
        .replace("call-tool", call_id)
        .into_bytes()
}

/// `events` with the provider's count of the request's input tokens.
pub(super) fn with_usage(events: Vec<u8>, input_tokens: u64) -> Vec<u8> {
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

fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
