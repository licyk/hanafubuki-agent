use super::*;

#[tokio::test]
async fn rapid_identical_connects_coalesce_into_one_connection() {
    let fixture = r#"
import json, sys, time
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "initialize":
        time.sleep(0.05)
        result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "coalesced", "version": "1"}}
    elif method == "tools/list": result = {"tools": []}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
    let mut config = stdio_config();
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec!["-u".into(), "-c".into(), fixture.into()];
    stdio.public_environment.clear();
    stdio.credential_environment.clear();
    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let manager = McpManager::new(executor.clone(), workers.clone());
    let (first, second) = tokio::join!(
        manager.connect_or_refresh(
            config.clone(),
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        ),
        manager.connect_or_refresh(
            config,
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.connection_generation, second.connection_generation);
    assert_eq!(executor_registry_counts(&executor), (1, 0, 1));
    manager
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(10))
        .await
        .unwrap();
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[tokio::test]
async fn manager_refresh_keeps_the_old_generation_until_plan_lease_release() {
    let fixture = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "initialize": result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}, "resources": {}}, "serverInfo": {"name": "fake", "version": "1"}}
    elif method == "tools/list": result = {"tools": [{"name": "echo", "description": "Echo", "inputSchema": {"type": "object", "properties": {"value": {"type": "string"}}, "required": ["value"], "additionalProperties": False}}]}
    elif method == "tools/call": result = {"content": [{"type": "text", "text": request["params"]["arguments"]["value"]}], "isError": False}
    elif method == "resources/list":
        cursor = (request.get("params") or {}).get("cursor")
        if cursor == "page-2": result = {"resources": [{"uri": "mcp://local/doc-2", "name": "doc 2"}]}
        else: result = {"resources": [{"uri": "mcp://local/doc", "name": "doc"}], "nextCursor": "page-2"}
    elif method == "resources/templates/list": result = {"resourceTemplates": [{"uriTemplate": "mcp://local/{name}", "name": "named document"}]}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
    let mut config = stdio_config();
    config.exposure_policy = McpExposurePolicy::Direct;
    config
        .tool_approval_policies
        .insert("echo".into(), McpApprovalPolicy::Deny);
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec!["-u".into(), "-c".into(), fixture.into()];
    stdio.public_environment.clear();
    stdio.credential_environment.clear();

    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let manager = McpManager::new(executor.clone(), workers.clone());
    let status = manager
        .connect_or_refresh(
            config.clone(),
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(status.resource_count, 2);
    assert_eq!(status.resource_template_count, 1);
    let old_lease = manager.tool_leases().remove(0);
    let old_step = manager.step_binding();
    let old_server = old_step.servers()[0].clone();
    assert_eq!(old_lease.approval_policy, McpApprovalPolicy::Deny);
    let old_generation = old_lease.binding.connection_generation;

    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.argv[2] = fixture.replace("mcp://local/doc", "mcp://local/new-doc");
    manager
        .connect_or_refresh(
            config,
            McpConnectionSecrets::empty(2),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    let new_generation = manager.tool_leases()[0].binding.connection_generation;
    assert_ne!(old_generation, new_generation);
    assert_eq!(executor_registry_counts(&executor), (2, 0, 2));
    let old_resources = old_server.list_resources(None).await.unwrap();
    assert_eq!(old_resources.items[0].uri, "mcp://local/doc");
    assert_eq!(old_resources.next_cursor.as_deref(), Some("page-2"));
    let old_second_page = old_server
        .list_resources(old_resources.next_cursor)
        .await
        .unwrap();
    assert_eq!(old_second_page.items[0].uri, "mcp://local/doc-2");
    let new_step = manager.step_binding();
    assert_ne!(
        old_server.connection_generation,
        new_step.servers()[0].connection_generation
    );
    let new_resources = new_step.servers()[0].list_resources(None).await.unwrap();
    assert_eq!(new_resources.items[0].uri, "mcp://local/new-doc");

    let old_result = old_lease
        .call(
            serde_json::Map::from_iter([("value".into(), json!("old"))]),
            McpCallContext {
                session_id: "session-old".into(),
                turn_id: 1,
                call_id: "call-old".into(),
                tool_generation: "old-tool-generation".into(),
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert!(!old_result.is_error);
    manager.cleanup_retired().await.unwrap();
    assert_eq!(executor_registry_counts(&executor), (2, 0, 2));

    drop(old_lease);
    drop(old_server);
    drop(old_step);
    manager.cleanup_retired().await.unwrap();
    assert_eq!(executor_registry_counts(&executor), (1, 0, 1));
    drop(new_step);
    manager
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[tokio::test]
async fn resolve_program_failure_never_reserves_a_process_and_is_diagnosed() {
    let mut config = stdio_config();
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = "definitely-missing-mcp-program-fixture".into();
    stdio
        .public_environment
        .insert("PATH".into(), "/definitely/missing".into());
    stdio.credential_environment.clear();
    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let manager = McpManager::new(executor.clone(), workers.clone());

    assert!(matches!(
        manager
            .connect_or_refresh(
                config.clone(),
                McpConnectionSecrets::empty(1),
                CancellationToken::default(),
            )
            .await,
        Err(McpError::ResolveProgram(_))
    ));
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    let diagnostics = manager.diagnostics(&config.id);
    assert!(diagnostics.entries.iter().any(|entry| {
        entry.stage == McpDiagnosticStage::ResolveProgram && entry.code == "resolve_program_failed"
    }));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
}

#[tokio::test]
async fn shutdown_past_its_deadline_reports_the_connection_and_keeps_its_cleanup() {
    // Keeps running after the end of its input; its cleanup has to stop it.
    let fixture = r#"
import json, sys, time
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "initialize":
        result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "lingering", "version": "1"}}
    elif method == "tools/list": result = {"tools": []}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
time.sleep(60)
"#;
    let mut config = stdio_config();
    config.shutdown_timeout_seconds = 1;
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec!["-u".into(), "-c".into(), fixture.into()];
    stdio.public_environment.clear();
    stdio.credential_environment.clear();
    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let manager = McpManager::new(executor.clone(), workers.clone());
    manager
        .connect_or_refresh(
            config,
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(executor_registry_counts(&executor), (1, 0, 1));

    // A deadline already passed: the cleanup gets no time to confirm.
    let started = Instant::now();
    let error = manager.shutdown(started).await.unwrap_err();

    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(matches!(error, McpError::CleanupUnconfirmed(_)), "{error}");
    // The connection keeps its cleanup obligation, which a later shutdown
    // completes.
    assert!(manager
        .statuses()
        .iter()
        .any(|status| status.cleanup_pending));
    manager
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
