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
    assert!(encoded_output.contains("exitCode"), "{encoded_output}");
    assert!(
        encoded_output.contains("approved-agent-command"),
        "{encoded_output}"
    );
    assert!(!encoded_output.contains("executorId"));
    assert!(!encoded_output.contains("receipt"));
    assert_eq!(
        service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE),
        []
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A write of the command timeout reaches the running execution plugin in
/// place: the tool it registered stays, and the approval a command asks for
/// from then on is of the new timeout.
#[tokio::test(flavor = "multi_thread")]
async fn the_command_timeout_reaches_the_running_plugin_in_place() {
    use hanafubuki_agent_contracts::{
        AgentToolHandler, RegistryService, ToolCallContext, ToolInvocation, TurnExtensions,
    };
    use hanafubuki_agent_kernel::PermissionRequirement;

    let (service, paths, _) = service_with_config("command-timeout-in-place", |_, _| {});
    service.start_plugins().await;
    let registries = service.test_plugin_registries();
    let exec_command = || {
        registries
            .tools
            .registry()
            .snapshot()
            .items()
            .find(|tool| tool.descriptor().name.qualified() == "exec_command")
            .cloned()
            .unwrap()
    };
    let workdir = paths.data_dir.clone();
    let digest = |tool: Arc<dyn AgentToolHandler>| {
        let context = crate::test_support::test_tool_context();
        let invocation = ToolInvocation {
            context: ToolCallContext {
                session_id: context.session_id,
                turn_id: context.turn_id,
                generation: context.generation,
                mode: context.mode,
                session_revision: context.session_revision,
                call_id: context.call_id,
                image_input: false,
                origin: Default::default(),
            },
            arguments: json!({ "cmd": "ls", "workdir": workdir }),
            cancellation: Default::default(),
            progress: context.progress,
            plan_leases: Vec::new(),
            turn: TurnExtensions::default().with(Arc::new(
                hanafubuki_agent_sandbox::SandboxSettings::full_access(),
            )),
        };
        async move {
            match tool.permission(&invocation).await.unwrap() {
                PermissionRequirement::ReviewRequired(action) => action.digest,
                other => panic!("a command asks for its approval: {other:?}"),
            }
        }
    };
    let tool = exec_command();
    let before = digest(tool.clone()).await;

    let mut options = serde_json::Map::new();
    options.insert("commandTimeoutMinutes".to_string(), json!(45));
    service
        .set_plugin_options("execution".into(), options)
        .await
        .unwrap();
    let deadline = Instant::now() + TEST_WAIT;
    while digest(tool.clone()).await == before {
        assert!(
            Instant::now() < deadline,
            "the option never reached the plugin"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        Arc::ptr_eq(&tool, &exec_command()),
        "the plugin did not restart"
    );
    let _ = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
