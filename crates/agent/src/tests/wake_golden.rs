//! The golden of a wake turn's request: its only opener is a
//! `<background_events>` fragment with one event of each kind
//! (`golden.rs` holds the helpers). Linux-only, as the fixtures are.

use hanafubuki_agent_kernel::AgentExecutionProgressStatus;
use serde_json::Value;

use super::golden::{assert_golden, normalize};
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
