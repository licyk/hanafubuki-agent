use super::*;
use serde_json::Value;

#[tokio::test]
async fn unified_exec_completes_only_after_manual_exact_approval() {
    let arguments = serde_json::to_string(&json!({
        "cmd": approved_command(),
        "shell": null,
        "login": null,
        "tty": null,
        // The call returns when the command exits; a cold PowerShell start
        // on a loaded runner takes longer than a short yield.
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768,
        "workdir": null
    }))
    .unwrap();
    let server = FakeServer::start(vec![
        (tool_events("exec_command", &arguments), Duration::ZERO),
        (
            assistant_events("The approved command was started."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths) = configured_service("approved-command", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn = tokio::spawn(async move {
        executing
            .start_turn(session_id, "Run the approved command.".to_string().into())
            .await
    });

    // The first turn's sampling and planning, on a loaded CI runner.
    let (turn_identity, request) = tokio::time::timeout(TEST_WAIT, async {
        loop {
            let snapshot = service.snapshot(session_id).unwrap();
            if let Some(turn_identity) = snapshot.turn {
                if let Some(request) = service
                    .events_since(session_id, None)
                    .unwrap()
                    .events
                    .into_iter()
                    .find_map(|event| match event.kind {
                        AgentEventKind::ApprovalRequested { request } => Some(request),
                        _ => None,
                    })
                {
                    break (turn_identity, request);
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("approval request must be published within the fixture deadline");
    assert_eq!(
        server.requests().len(),
        1,
        "no second sampling before approval"
    );
    assert_eq!(
        request.action.kind,
        hanafubuki_agent_approval::ApprovalActionKind::ShellExecution
    );
    // Without a workdir the command runs in the first workspace root, not
    // in this process's directory.
    let root =
        hanafubuki_agent_foundation::paths::ResolvedPath::resolve(&paths.managed_instances_dir)
            .unwrap();
    assert!(
        request
            .action
            .summary
            .contains(&format!(" command in {}: ", root.native_text())),
        "{}",
        request.action.summary
    );
    service
        .resolve_approval(
            session_id,
            turn_identity.id,
            turn_identity.generation,
            request.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();

    assert_eq!(
        // A cold PowerShell start on a loaded CI runner takes seconds.
        tokio::time::timeout(TEST_WAIT, turn)
            .await
            .expect("approved command turn must finish within the fixture deadline")
            .unwrap()
            .unwrap()
            .final_answer,
        "The approved command was started."
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let command_output = second["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output" && item["call_id"] == "call-tool")
        .expect("command tool output");
    let encoded_output = command_output.to_string();
    assert!(encoded_output.contains("exitCode"));
    assert!(encoded_output.contains("approved-agent-command"));
    assert!(!encoded_output.contains("executorId"));
    assert!(!encoded_output.contains("receipt"));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
