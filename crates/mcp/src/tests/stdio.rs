use super::*;

#[tokio::test]
async fn modern_discovery_uses_correlated_legacy_fallback() {
    let fixture = r#"
import json, sys
discovered = False
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "server/discover":
        discovered = True
        response = {"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "Method not found"}}
    elif method == "initialize":
        if not discovered: raise RuntimeError("legacy initialize arrived before server/discover")
        response = {"jsonrpc": "2.0", "id": request["id"], "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "legacy", "version": "1"}}}
    elif method == "tools/list":
        response = {"jsonrpc": "2.0", "id": request["id"], "result": {"tools": []}}
    else:
        response = {"jsonrpc": "2.0", "id": request["id"], "result": {}}
    print(json.dumps(response), flush=True)
"#;
    let mut config = stdio_config();
    config.protocol_mode = McpProtocolMode::V20260728;
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec!["-u".into(), "-c".into(), fixture.into()];
    stdio.public_environment.clear();
    stdio.credential_environment.clear();
    let executor = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let connection = McpStdioConnection::connect(
        &config,
        McpResolvedSecretEnvironment::new(BTreeMap::new()).unwrap(),
        1,
        executor.clone(),
        workers.clone(),
        McpInteractionHub::default(),
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(connection.capabilities().protocol_version, "2025-06-18");
    assert!(connection
        .discover_tools(&config)
        .await
        .unwrap()
        .tools
        .is_empty());
    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[tokio::test]
async fn discovery_failure_confirms_candidate_process_cleanup() {
    let fixture = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    if request.get("method") == "notifications/initialized": continue
    if request.get("method") == "initialize": result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}
    elif request.get("method") == "tools/list": result = {"tools": {"bad": {"inputSchema": []}}}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
    // One bad tool costs only itself, so the listing fails as a page: its
    // tools are not a list.
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
    assert!(manager
        .connect_or_refresh(
            config,
            McpConnectionSecrets::empty(1),
            CancellationToken::default(),
        )
        .await
        .is_err());
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[tokio::test]
async fn stdio_connection_uses_receipt_transport_and_separates_stderr() {
    let fixture = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized":
        continue
    if method == "initialize":
        result = {
            "protocolVersion": request["params"]["protocolVersion"],
            "capabilities": {"tools": {"listChanged": True}, "resources": {}},
            "serverInfo": {"name": "fake-hanafubuki-mcp", "version": "1"}
        }
    elif method == "tools/list":
        tool = {
            "name": "echo" if not request.get("params", {}).get("cursor") else "second",
            "description": "Echo one value",
            "inputSchema": {
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
                "additionalProperties": False
            }
        }
        result = {"tools": [tool]}
        if not request.get("params", {}).get("cursor"): result["nextCursor"] = "page-2"
    elif method == "tools/call":
        result = {"content": [{"type": "text", "text": request["params"]["arguments"]["value"]}], "isError": False}
    elif method == "resources/list":
        result = {"resources": [{"uri": "mcp://local/doc", "name": "doc", "mimeType": "text/plain"}]}
    elif method == "resources/templates/list":
        result = {"resourceTemplates": [{"uriTemplate": "mcp://local/{name}", "name": "named document"}]}
    elif method == "resources/read":
        result = {"contents": [{"uri": request["params"]["uri"], "mimeType": "text/plain", "text": "resource body"}]}
    else:
        result = {}
    response = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}) + "\n"
    split = max(1, len(response) // 2)
    sys.stdout.write(response[:split]); sys.stdout.flush()
    sys.stderr.write("fixture diagnostic\n"); sys.stderr.flush()
    sys.stdout.write(response[split:]); sys.stdout.flush()
    if method == "tools/list":
        print(json.dumps({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}), flush=True)
"#;
    let mut config = stdio_config();
    config.exposure_policy = McpExposurePolicy::Direct;
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec!["-u".into(), "-c".into(), fixture.into()];
    stdio.public_environment.clear();
    stdio.credential_environment.clear();

    let manager = Arc::new(ExecutorManager::default());
    let workers = BackgroundWorkerGroup::default();
    let connection = McpStdioConnection::connect(
        &config,
        McpResolvedSecretEnvironment::new(BTreeMap::new()).unwrap(),
        1,
        manager.clone(),
        workers.clone(),
        McpInteractionHub::default(),
        CancellationToken::default(),
    )
    .await
    .unwrap();
    assert_eq!(executor_registry_counts(&manager), (1, 0, 1));

    let tools = connection.discover_tools(&config).await.unwrap().tools;
    assert_eq!(tools.len(), 2);
    for _ in 0..20 {
        if connection.tools_changed() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(connection.tools_changed());
    connection.clear_tools_changed();
    assert!(!connection.tools_changed());
    let result = connection
        .call_tool(
            &tools[0],
            serde_json::Map::from_iter([("value".into(), json!("hello"))]),
            Duration::from_secs(5),
            crate::interaction::ElicitationPauseState::new(),
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
    let resources = connection
        .list_resources(Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(resources.items.len(), 1);
    assert_eq!(resources.items[0].uri, "mcp://local/doc");
    let templates = connection
        .list_resource_templates(Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(templates.items.len(), 1);
    let resource = connection
        .read_resource("mcp://local/doc".into(), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(matches!(
        &resource.content[0],
        hanafubuki_agent_tool_contracts::ToolOutputContent::EmbeddedResource { text: Some(text), .. }
            if text == "resource body"
    ));

    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    assert_eq!(executor_registry_counts(&manager), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[tokio::test]
async fn stdio_tool_active_timeout_pauses_for_real_user_elicitation() {
    let fixture = r#"
import json, sys
pending_call = None
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized":
        continue
    if method == "initialize":
        result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "elicitation-fixture", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "ask", "description": "Ask before continuing", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}}]}
    elif method == "tools/call":
        pending_call = request["id"]
        print(json.dumps({"jsonrpc": "2.0", "id": "server-elicit", "method": "elicitation/create", "params": {"message": "Continue?", "requestedSchema": {"type": "object", "properties": {}, "additionalProperties": False}}}), flush=True)
        continue
    elif method is None and request.get("id") == "server-elicit":
        print(json.dumps({"jsonrpc": "2.0", "id": pending_call, "result": {"content": [{"type": "text", "text": "continued-after-user-wait"}], "isError": False}}), flush=True)
        continue
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
    let mut config = stdio_config();
    config.call_timeout_seconds = 1;
    config.approval_policy = McpApprovalPolicy::Allow;
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
    let lease = manager.tool_leases().remove(0);
    let call = tokio::spawn({
        let lease = lease.clone();
        async move {
            lease
                .call(
                    Default::default(),
                    McpCallContext {
                        session_id: "session-active-time".into(),
                        turn_id: 7,
                        call_id: "call-active-time".into(),
                        tool_generation: "fixture-generation".into(),
                    },
                    CancellationToken::default(),
                )
                .await
        }
    });
    let elicitation = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(request) = manager.pending_elicitations().into_iter().next() {
                break request;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(elicitation.session_id, "session-active-time");
    assert_eq!(elicitation.turn_id, 7);
    assert_eq!(elicitation.call_id, "call-active-time");
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert!(!call.is_finished());
    manager
        .resolve_elicitation(
            elicitation.id,
            McpElicitationDecision::Accept { content: json!({}) },
        )
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(output
        .legacy_text_projection()
        .unwrap()
        .contains("continued-after-user-wait"));

    drop(lease);
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
async fn turn_cancellation_clears_the_exact_real_protocol_elicitation() {
    let fixture = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method in ("notifications/initialized", "notifications/cancelled"):
        continue
    if method == "initialize":
        result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "cancel-elicit", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "ask", "description": "ask", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}}]}
    elif method == "tools/call":
        print(json.dumps({"jsonrpc": "2.0", "id": "server-elicit-cancel", "method": "elicitation/create", "params": {"message": "Continue?", "requestedSchema": {"type": "object", "properties": {}, "additionalProperties": False}}}), flush=True)
        continue
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
    let mut config = stdio_config();
    config.call_timeout_seconds = 5;
    config.approval_policy = McpApprovalPolicy::Allow;
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
    let lease = manager.tool_leases().remove(0);
    let cancellation = CancellationToken::default();
    let call = tokio::spawn({
        let lease = lease.clone();
        let cancellation = cancellation.clone();
        async move {
            lease
                .call(
                    Default::default(),
                    McpCallContext {
                        session_id: "session-cancel".into(),
                        turn_id: 9,
                        call_id: "call-cancel".into(),
                        tool_generation: "frozen-tool".into(),
                    },
                    cancellation,
                )
                .await
        }
    });
    let pending = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(request) = manager.pending_elicitations().into_iter().next() {
                break request;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pending.session_id, "session-cancel");
    assert_eq!(pending.call_id, "call-cancel");
    cancellation.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), call)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        McpError::Cancelled
    );
    assert!(manager.pending_elicitations().is_empty());

    drop(lease);
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
async fn resource_listing_enforces_total_time_and_repeated_cursor_limits() {
    // The listing that times out is abandoned, and rmcp tells the server so
    // with `notifications/cancelled`; a server must not answer a notification.
    let fixture = r#"
import json, sys, time
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method in ("notifications/initialized", "notifications/cancelled"): continue
    if method == "initialize":
        result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}, "resources": {}}, "serverInfo": {"name": "limits", "version": "1"}}
    elif method == "resources/list":
        time.sleep(0.05)
        result = {"resources": [{"uri": "mcp://limits/item", "name": "item"}], "nextCursor": "repeat"}
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
    let connection = McpStdioConnection::connect(
        &config,
        McpResolvedSecretEnvironment::new(BTreeMap::new()).unwrap(),
        1,
        executor.clone(),
        workers.clone(),
        McpInteractionHub::default(),
        CancellationToken::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        connection
            .list_resources(Duration::from_millis(10))
            .await
            .unwrap_err(),
        McpError::Timeout
    );
    assert!(matches!(
        connection
            .list_resources(Duration::from_secs(1))
            .await
            .unwrap_err(),
        McpError::Protocol(message) if message.contains("cursor repeated")
    ));

    connection.shutdown(Duration::from_secs(5)).await.unwrap();
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

#[tokio::test]
async fn stdio_rejects_an_oversized_outbound_jsonrpc_frame_without_echoing_content() {
    let fixture = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "initialize": result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "outbound-limit", "version": "1"}}
    elif method == "tools/list": result = {"tools": [{"name": "echo", "description": "echo", "inputSchema": {"type": "object", "properties": {"value": {"type": "string"}}, "required": ["value"], "additionalProperties": False}}]}
    elif method == "tools/call": result = {"content": [{"type": "text", "text": "unexpected"}], "isError": False}
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
    let connection = McpStdioConnection::connect(
        &config,
        McpResolvedSecretEnvironment::new(BTreeMap::new()).unwrap(),
        1,
        executor.clone(),
        workers.clone(),
        McpInteractionHub::default(),
        CancellationToken::default(),
    )
    .await
    .unwrap();
    let tools = connection.discover_tools(&config).await.unwrap().tools;
    let sensitive = "secret=never-log-this";
    let mut value = String::with_capacity(crate::client_adapter::MAXIMUM_MCP_MESSAGE_BYTES + 128);
    value.push_str(sensitive);
    value.push_str(&"x".repeat(crate::client_adapter::MAXIMUM_MCP_MESSAGE_BYTES));
    let error = connection
        .call_tool(
            &tools[0],
            serde_json::Map::from_iter([("value".into(), json!(value))]),
            Duration::from_secs(5),
            crate::interaction::ElicitationPauseState::new(),
            CancellationToken::default(),
        )
        .await
        .unwrap_err();
    let error = error.to_string();
    assert!(error.len() <= 2200, "{error}");
    assert!(!error.contains(sensitive), "{error}");
    assert!(error.contains("exceeds its limit") || error.contains("transport"));

    let _ = connection.shutdown(Duration::from_secs(5)).await;
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}
