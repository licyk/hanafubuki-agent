//! Golden baselines of compaction: a manual one, one in the background,
//! and a window compacted mid-turn (`golden.rs` holds the rest and the
//! helpers). Linux-only, as the fixtures are.

use super::golden::{agent_history_dir, assert_golden, copy_tree, normalize};
use super::*;
use crate::AgentConfig;
use serde_json::Value;

/// A manual compaction after one turn: the summary request it sends, and the
/// next turn's request, which starts from the installed checkpoint. The
/// session carries a Skill catalog and workspace instructions, so what the
/// compaction keeps of the initial context is visible.
#[tokio::test]
async fn compacted_request_matches_golden() {
    let server = FakeServer::start(vec![
        (assistant_events("Done."), Duration::ZERO),
        (
            assistant_events("The user said hello; nothing else happened."),
            Duration::ZERO,
        ),
        (assistant_events("Still here."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("golden-compacted", &server.base_url);
    enable_two_skills(&service, &paths);
    std::fs::write(
        paths.managed_instances_dir.join("AGENTS.md"),
        "# Workspace rules\n\nRun the narrow checks first.\n",
    )
    .unwrap();
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Hello.".to_string().into())
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, before.generation, before.revision)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Are you there?".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let body = |index: usize| -> Value { serde_json::from_slice(&requests[index].body).unwrap() };
    let pinned = serde_json::json!({
        "compaction_request": body(1),
        "next_request": body(2),
    });
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let normalized = normalize(&pinned, &paths.app_data_dir);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    assert_golden("compacted_request", &normalized);
}

/// A compaction run in the background (`[compaction] strategy =
/// "background"`) and installed before the next turn: the summary request it
/// sends, of the first turn only, and the next turn's request, whose window
/// is the summary, the turn it did not compact and the turn itself. Reopened
/// from its history, the session continues from the same window.
// The managed turns' tasks must finish while `shutdown` blocks this thread.
#[tokio::test(flavor = "multi_thread")]
async fn background_compacted_request_matches_golden() {
    let server = FakeServer::start(
        [
            assistant_events(&"The first answer is long. ".repeat(100)),
            // In the band from 65% of the window to the foreground 80%.
            with_input_tokens(assistant_events("Done."), 70_000),
            assistant_events("The user said hello."),
            assistant_events("Still here."),
            assistant_events("Continued."),
        ]
        .into_iter()
        .map(|response| (response, Duration::ZERO))
        .collect(),
    )
    .await;
    let configure = |config: &AgentConfig| {
        configure_test_provider(config, &server.base_url, None);
        config
            .mutate_providers(1, |registry| {
                registry.model_policy.context_window_tokens = Some(100_000);
                Ok(())
            })
            .unwrap();
        config
            .write_settings(AgentSettingsPatch {
                compaction: Some(crate::AgentCompactionSettings {
                    strategy: crate::AgentCompactionStrategy::Background,
                    ..crate::AgentCompactionSettings::default()
                }),
                ..AgentSettingsPatch::default()
            })
            .unwrap();
    };
    let (service, paths) =
        service_with("golden-background-compacted", |_, config| configure(config));
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    for prompt in ["Hello.", "Go on."] {
        service
            .start_turn(session.id, prompt.to_string().into())
            .await
            .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while service.snapshot(session.id).unwrap().background_compaction
        != Some(hanafubuki_agent_kernel::AgentBackgroundCompactionState::Ready)
    {
        assert!(
            Instant::now() < deadline,
            "the background compaction never ended"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    service
        .start_turn(session.id, "Are you there?".to_string().into())
        .await
        .unwrap();
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .compaction
            .unwrap()
            .phase,
        hanafubuki_agent_kernel::AgentCompactionPhase::Background
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    let body = |index: usize| -> Value { serde_json::from_slice(&requests[index].body).unwrap() };
    let pinned = serde_json::json!({
        "compaction_request": body(2),
        "next_request": body(3),
    });
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let normalized = normalize(&pinned, &paths.app_data_dir);

    let (reopened, reopened_paths) = service_with("golden-background-reopened", |copy, config| {
        copy_tree(&agent_history_dir(&paths), &agent_history_dir(copy));
        configure(config);
    });
    reopened
        .start_turn(session.id, "Continue.".to_string().into())
        .await
        .unwrap();
    let continued = String::from_utf8(server.requests()[4].body.clone()).unwrap();
    assert!(continued.contains("The user said hello."));
    assert!(continued.contains("Go on."));
    assert!(continued.contains("Still here."));
    assert!(!continued.contains("The first answer is long."));
    assert!(reopened
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
    let _ = std::fs::remove_dir_all(reopened_paths.app_data_dir);
    assert_golden("background_compacted_request", &normalized);
}

/// A window compacted mid-turn (the model asked for a new context) renders
/// every section again from the registry: the step after the compaction
/// carries the Skill catalog, as the first step did, beside the compacted
/// window's own initial context.
#[tokio::test]
async fn a_window_compacted_mid_turn_keeps_the_skill_catalog() {
    let server = FakeServer::start(vec![
        (tool_events("new_context", "{}"), Duration::ZERO),
        (
            assistant_events("The user asked for a fresh context."),
            Duration::ZERO,
        ),
        (assistant_events("Done."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("compacted-mid-turn", &server.base_url);
    enable_two_skills(&service, &paths);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    service
        .start_turn(session.id, "Start over.".to_string().into())
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let input = |index: usize| -> Vec<Value> {
        let body: Value = serde_json::from_slice(&requests[index].body).unwrap();
        body["input"].as_array().unwrap().clone()
    };
    let has_text =
        |items: &[Value], needle: &str| items.iter().any(|item| item.to_string().contains(needle));
    assert!(has_text(&input(0), "<skills_instructions>"));
    assert!(!has_text(&input(1), "<skills_instructions>"));
    let after = input(2);
    assert!(has_text(&after, "<skills_instructions>"));
    assert!(has_text(&after, "Mode: default"));
    assert!(has_text(&after, "The user asked for a fresh context."));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
