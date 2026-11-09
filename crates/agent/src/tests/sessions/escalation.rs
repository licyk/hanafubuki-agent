//! A command the model asks to run outside the sandbox (Codex's
//! `sandbox_permissions: "require_escalated"`), in each permission mode.
//! Outside macOS this test binary has no sandbox (see `sandbox.rs`), so a
//! sandboxed command is refused there and only one that left the sandbox
//! runs: what runs, ran unconfined.

use super::*;
use hanafubuki_agent_approval::{AgentPermissionMode, ApprovalRequest, SandboxEscalation};
use hanafubuki_agent_kernel::ApprovalUserDecision;
use serde_json::Value;

const JUSTIFICATION: &str = "Do you want to run the fixture command outside the sandbox?";

/// An `exec_command` call `call_id` of `command`, escalated or not.
fn command_call(call_id: &str, command: &str, escalated: bool) -> Vec<u8> {
    let (permissions, justification) = if escalated {
        (json!("require_escalated"), json!(JUSTIFICATION))
    } else {
        (Value::Null, Value::Null)
    };
    let arguments = json!({
        "cmd": command,
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null,
        "sandbox_permissions": permissions,
        "justification": justification
    });
    String::from_utf8(tool_events("exec_command", &arguments.to_string()))
        .unwrap()
        .replace("call-tool", call_id)
        .into_bytes()
}

#[cfg(unix)]
fn other_command() -> String {
    "printf other-agent-command".to_string()
}

#[cfg(windows)]
fn other_command() -> String {
    "Write-Output other-agent-command".to_string()
}

/// A session of a service whose model gives `responses`, in `mode` with
/// the `sandbox` mode.
async fn session_in(
    name: &str,
    responses: Vec<Vec<u8>>,
    mode: AgentPermissionMode,
    sandbox: AgentSandboxMode,
) -> (
    AgentApplicationService,
    AgentSessionId,
    FakeServer,
    TestPaths,
) {
    let server = FakeServer::start(
        responses
            .into_iter()
            .map(|response| (response, Duration::ZERO))
            .collect(),
    )
    .await;
    let (service, paths) = configured_service(name, &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .set_session_permission_mode(session.id, mode)
        .unwrap();
    service
        .set_session_sandbox_mode(session.id, sandbox)
        .unwrap();
    (service, session.id, server, paths)
}

/// Runs a turn, answering its approvals with `answers` in order, and
/// returns the requests answered once the turn ended.
async fn run_turn_answering(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    answers: &[ApprovalUserDecision],
) -> (String, Vec<ApprovalRequest>) {
    let running = service.clone();
    let turn = tokio::spawn(async move {
        running
            .start_turn(session_id, "Run the commands.".to_string().into())
            .await
    });
    let mut answered = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while answered.len() < answers.len() {
        assert!(Instant::now() < deadline, "the approvals were not asked");
        let pending = service
            .events_since(session_id, None)
            .unwrap()
            .events
            .into_iter()
            .filter_map(|event| match event.kind {
                AgentEventKind::ApprovalRequested { request } => Some(request),
                _ => None,
            })
            .nth(answered.len());
        let (Some(request), Some(turn)) = (pending, service.snapshot(session_id).unwrap().turn)
        else {
            tokio::time::sleep(Duration::from_millis(5)).await;
            continue;
        };
        service
            .resolve_approval(
                session_id,
                turn.id,
                turn.generation,
                request.request_id,
                answers[answered.len()],
            )
            .unwrap();
        answered.push(request);
    }
    let outcome = tokio::time::timeout(Duration::from_secs(60), turn)
        .await
        .expect("the turn ends")
        .unwrap()
        .unwrap();
    let asked = service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. }))
        .count();
    assert_eq!(asked, answers.len(), "every approval asked was answered");
    (outcome.final_answer, answered)
}

fn request_text(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

/// What the model read back from its call `call_id`, in the last request.
fn output_of(server: &FakeServer, call_id: &str) -> String {
    let requests = server.requests();
    let last: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    last["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output" && item["call_id"] == call_id)
        .unwrap_or_else(|| panic!("no output of {call_id}"))["output"]
        .to_string()
}

fn shutdown(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// Manual: the user is asked, with the justification and that the command
/// leaves the sandbox. Denied, it does not run; approved, it runs, here
/// where no sandbox could have run it.
#[tokio::test]
async fn the_user_approves_an_escalated_command_which_then_runs_unsandboxed() {
    let (service, session_id, server, paths) = session_in(
        "escalation-manual",
        vec![
            command_call("call-denied", &approved_command(), true),
            command_call("call-approved", &approved_command(), true),
            assistant_events("Ran it."),
        ],
        AgentPermissionMode::Manual,
        AgentSandboxMode::WorkspaceWrite,
    )
    .await;

    let (answer, requests) = run_turn_answering(
        &service,
        session_id,
        &[ApprovalUserDecision::Deny, ApprovalUserDecision::AllowOnce],
    )
    .await;

    assert_eq!(answer, "Ran it.");
    let action = &requests[0].action;
    assert_eq!(
        action.sandbox_escalation,
        Some(SandboxEscalation {
            justification: Some(JUSTIFICATION.to_string())
        })
    );
    assert!(
        action.summary.contains("command outside the sandbox in ")
            && action
                .summary
                .ends_with(&format!("(reason: {JUSTIFICATION})")),
        "{}",
        action.summary
    );
    assert!(requests[0].offers_allow_for_session);
    let denied = output_of(&server, "call-denied");
    assert!(
        denied.contains("user denied the requested operation"),
        "{denied}"
    );
    let ran = output_of(&server, "call-approved");
    assert!(
        ran.contains("exitCode") && ran.contains("approved-agent-command"),
        "{ran}"
    );
    shutdown(service, paths);
}

/// "Allow for this session" of an escalated command allows that command
/// outside the sandbox again, and nothing else: not the tool's sandboxed
/// commands, nor another command outside the sandbox; and the tool's
/// allowance does not cover an escalated command.
#[tokio::test]
async fn an_escalated_allowance_is_kept_apart_from_the_tools() {
    let (service, session_id, server, paths) = session_in(
        "escalation-allowance",
        vec![
            command_call("call-1", &approved_command(), true),
            command_call("call-2", &approved_command(), false),
            command_call("call-3", &other_command(), true),
            command_call("call-4", &approved_command(), true),
            command_call("call-5", &other_command(), false),
            assistant_events("Done."),
        ],
        AgentPermissionMode::Manual,
        // No sandbox, so the sandboxed commands run too.
        AgentSandboxMode::FullAccess,
    )
    .await;

    let (answer, requests) = run_turn_answering(
        &service,
        session_id,
        &[
            ApprovalUserDecision::AllowForSession,
            ApprovalUserDecision::AllowForSession,
            ApprovalUserDecision::AllowOnce,
        ],
    )
    .await;

    assert_eq!(answer, "Done.");
    let escalated: Vec<bool> = requests
        .iter()
        .map(|request| request.action.sandbox_escalation.is_some())
        .collect();
    assert_eq!(escalated, [true, false, true]);
    assert!(requests[2].action.summary.contains("other-agent-command"));
    // The fourth and fifth commands ran without asking.
    assert!(output_of(&server, "call-4").contains("approved-agent-command"));
    assert!(output_of(&server, "call-5").contains("other-agent-command"));
    shutdown(service, paths);
}

/// Full access asks no one, so, as under Codex's `never`, nothing leaves
/// the sandbox: the model reads why, and no one is asked.
#[tokio::test]
async fn full_access_refuses_an_escalated_command() {
    let (service, session_id, server, paths) = session_in(
        "escalation-full-access",
        vec![
            command_call("call-tool", &approved_command(), true),
            assistant_events("Refused."),
        ],
        AgentPermissionMode::FullAccess,
        AgentSandboxMode::WorkspaceWrite,
    )
    .await;

    let (answer, _) = run_turn_answering(&service, session_id, &[]).await;

    assert_eq!(answer, "Refused.");
    let refused = output_of(&server, "call-tool");
    assert!(
        refused.contains("the permission mode is full access, which asks no one"),
        "{refused}"
    );
    assert!(!refused.contains("exitCode"), "{refused}");
    shutdown(service, paths);
}

/// Auto-review: the reviewer decides, seeing that the command leaves the
/// sandbox and the justification; allowed, it runs.
#[tokio::test]
async fn automatic_review_decides_an_escalated_command() {
    let allow = json!({
        "riskLevel": "low",
        "userAuthorization": "explicit",
        "outcome": "allow",
        "rationale": "The user asked for this command."
    })
    .to_string();
    let (service, session_id, server, paths) = session_in(
        "escalation-auto-review",
        vec![
            command_call("call-tool", &approved_command(), true),
            assistant_events(&allow),
            assistant_events("Reviewed and ran."),
        ],
        AgentPermissionMode::AutoReview,
        AgentSandboxMode::WorkspaceWrite,
    )
    .await;

    let (answer, _) = run_turn_answering(&service, session_id, &[]).await;

    assert_eq!(answer, "Reviewed and ran.");
    let review = request_text(&server, 1);
    assert!(review.contains("sandboxEscalation"), "{review}");
    assert!(review.contains(JUSTIFICATION), "{review}");
    assert!(review.contains("runs outside the sandbox"), "{review}");
    assert!(output_of(&server, "call-tool").contains("approved-agent-command"));
    shutdown(service, paths);
}
