//! The golden of a wake turn's request: its only opener is a
//! `<background_events>` fragment with one event of each kind
//! (`golden.rs` holds the helpers); and, with `wakeOnBackgroundTasks` off,
//! a request is the one from before wakes. Linux-only, as the fixtures
//! are.

use hanafubuki_agent_kernel::AgentExecutionProgressStatus;
use serde_json::Value;

use super::golden::{assert_golden, first_request_with, golden_path, normalize};
use super::wake::{run_wake_turn, session_with_events};
use super::*;
use crate::{AgentSubagentWake, AgentTerminalWake, AgentWakeSource};

#[tokio::test(flavor = "multi_thread")]
async fn wake_events_request_matches_golden() {
    let server = FakeServer::start(vec![(assistant_events("Noted."), Duration::ZERO)]).await;
    let (service, paths) = configured_service("golden-wake-events", &server.base_url);
    let session_id = session_with_events(&service, every_kind_of_event()).await;
    run_wake_turn(&service, session_id).await;
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_golden(
        "wake_events_request",
        &normalize(&body, &paths.app_data_dir),
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Every kind of event: what the golden's wake turn reads.
fn every_kind_of_event() -> Vec<AgentWakeSource> {
    let subagent = |id: u64, name: &str, event| AgentWakeSource::Subagent {
        session_id: AgentSessionId::new(id),
        name: name.to_string(),
        event,
    };
    let terminal = |id: u64, command: &str, event| AgentWakeSource::Terminal {
        execution_id: id,
        command: command.to_string(),
        event,
    };
    let ended = |status, exit_code, output: &str| AgentTerminalWake::Ended {
        status,
        exit_code,
        duration_millis: 252_000,
        output: output.to_string(),
        truncated: false,
    };
    vec![
        subagent(
            41,
            "explore_api",
            AgentSubagentWake::Completed {
                answer: Some("The API has three endpoints.".to_string()),
                truncated: false,
            },
        ),
        subagent(
            42,
            "fix_tests",
            AgentSubagentWake::Errored {
                error: "model remote error status=Some(500)".to_string(),
            },
        ),
        subagent(43, "docs", AgentSubagentWake::ClosedByUser),
        subagent(44, "lint", AgentSubagentWake::Lost),
        subagent(
            41,
            "explore_api",
            AgentSubagentWake::Message {
                text: "Found the schema.".to_string(),
            },
        ),
        terminal(
            12,
            "cargo test --workspace",
            ended(
                AgentExecutionProgressStatus::Exited,
                Some(101),
                "test result: FAILED",
            ),
        ),
        terminal(
            13,
            "npm run dev",
            ended(AgentExecutionProgressStatus::TimedOut, None, ""),
        ),
        terminal(
            14,
            "./missing",
            ended(AgentExecutionProgressStatus::Failed, None, ""),
        ),
        terminal(
            15,
            "python -m http.server",
            AgentTerminalWake::StoppedByUser { exit_code: None },
        ),
        terminal(16, "make bench", AgentTerminalWake::Lost),
    ]
}

/// The tools whose descriptions changed with wakes, to be true with the
/// option on and off.
const REWORDED: [&str; 3] = ["spawn_agent", "send_message", "wait_agent"];

/// With `wakeOnBackgroundTasks` off, the first request of a session is the
/// one before wakes (`default_mode_request_before_wake.json`, kept as it
/// was, never regenerated), byte for byte, but for the reworded
/// descriptions of three tools: the guidance tells the model to wait.
#[tokio::test(flavor = "multi_thread")]
async fn with_the_wake_off_a_request_is_the_one_before_wakes() {
    let off = first_request_with(
        "golden-wake-off",
        AgentSessionMode::Default,
        |_, config| {
            config
                .write_settings(crate::AgentSettingsPatch {
                    wake_on_background_tasks: Some(false),
                    ..crate::AgentSettingsPatch::default()
                })
                .unwrap();
        },
        |_, _| {},
    )
    .await;
    let before: Value = serde_json::from_str(
        &std::fs::read_to_string(golden_path("default_mode_request_before_wake.json")).unwrap(),
    )
    .unwrap();

    assert_ne!(
        off, before,
        "tool descriptions and nesting guidance intentionally changed"
    );
    assert_eq!(without_reworded(off), without_reworded(before));
}

/// `request` without the descriptions of the [`REWORDED`] tools and their
/// arguments, allowing the later nesting/default-limit guidance change.
fn without_reworded(mut request: Value) -> Value {
    for tool in request["tools"].as_array_mut().unwrap() {
        if REWORDED.contains(&tool["name"].as_str().unwrap_or_default()) {
            tool.as_object_mut().unwrap().remove("description");
            for property in tool["parameters"]["properties"]
                .as_object_mut()
                .unwrap()
                .values_mut()
            {
                property.as_object_mut().unwrap().remove("description");
            }
        }
    }
    for item in request["input"].as_array_mut().unwrap() {
        for part in item["content"].as_array_mut().into_iter().flatten() {
            if let Some(text) = part["text"].as_str() {
                let text = text.replace(
                    "At most 8 sub-agents work at once in this conversation, counting the ones they spawn; a sub-agent you spawn can spawn sub-agents of its own, which cannot spawn further.",
                    "At most 3 sub-agents run at once, and a sub-agent cannot spawn sub-agents of its own.",
                );
                part["text"] = Value::String(text);
            }
        }
    }
    request
}
