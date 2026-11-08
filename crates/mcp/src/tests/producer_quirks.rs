//! A server that lists one bad tool, resource or template among good ones,
//! ends paging with an empty cursor and answers with content this client does
//! not support or cannot decode stays connected with everything it can use;
//! a page over its bounds or an error response still fails the listing.

use hanafubuki_agent_tool_contracts::ToolOutputContent;

use super::*;

const QUIRKY_SERVER: &str = r#"
import json, sys
mode = sys.argv[1] if len(sys.argv) > 1 else "quirky"
schema = {"type": "object", "properties": {"value": {"type": "string"}}}
deep = {"type": "string"}
for _ in range(40): deep = {"type": "object", "properties": {"nested": deep}}
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    params = request.get("params") or {}
    if method == "notifications/initialized": continue
    if method == "initialize": result = {"protocolVersion": params["protocolVersion"], "capabilities": {"tools": {}, "resources": {}}, "serverInfo": {"name": "quirky", "version": "1"}, "vendorExtension": {"x": 1}}
    elif method == "tools/list" and mode == "unauthorized":
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32001, "message": "Unauthorized"}}), flush=True)
        continue
    elif method == "tools/list" and mode == "oversized":
        result = {"tools": [
            {"name": "broken", "inputSchema": "not an object"},
            {"name": "big", "description": "b" * (5 * 1024 * 1024), "inputSchema": schema},
        ]}
    elif method == "tools/list" and not params.get("cursor"):
        result = {"tools": [
            {"name": "echo", "description": "Echo", "inputSchema": schema, "vendorField": True},
            {"name": "broken", "description": "Not a tool", "inputSchema": "not an object"},
            {"name": "deep", "description": "Too deep", "inputSchema": deep},
        ], "nextCursor": "page-2"}
    elif method == "tools/list":
        result = {"tools": [
            {"name": "echo", "description": "A later duplicate", "inputSchema": schema},
            {"name": "long", "description": "d" * 20000, "inputSchema": schema},
        ], "nextCursor": ""}
    elif method == "tools/call":
        result = {"content": [
            {"type": "text", "text": "kept"},
            {"type": "video", "data": "AAAA"},
            {"type": "image", "data": "AAA", "mimeType": "image/PNG; charset=binary"},
        ], "isError": False}
    elif method == "resources/list": result = {"resources": [{"uri": "mcp://quirky/doc", "name": "doc"}, {"name": "no-uri"}], "nextCursor": ""}
    elif method == "resources/templates/list": result = {"resourceTemplates": [{"uriTemplate": "mcp://quirky/{id}", "name": "by-id"}, {"name": "no-template"}]}
    elif method == "resources/read":
        result = {"contents": [
            {"uri": "mcp://quirky/doc", "text": "body"},
            {"uri": "mcp://quirky/doc#2"},
            {"uri": "mcp://quirky/blob", "blob": "AAA", "mimeType": "image/PNG; q=1"},
        ]}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;

fn quirky_config(mode: &str) -> McpServerConfig {
    let mut config = stdio_config();
    config.exposure_policy = McpExposurePolicy::Direct;
    config.approval_policy = McpApprovalPolicy::Allow;
    let McpTransportConfig::Stdio(stdio) = &mut config.transport else {
        unreachable!()
    };
    stdio.program = python_program().into();
    stdio.argv = vec!["-u".into(), "-c".into(), QUIRKY_SERVER.into(), mode.into()];
    stdio.public_environment.clear();
    stdio.credential_environment.clear();
    config
}

#[tokio::test]
async fn a_quirky_server_stays_connected_with_its_usable_tools() {
    let config = quirky_config("quirky");
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
    assert_eq!(status.phase, McpConnectionPhase::Ready);
    assert_eq!(status.tool_count, 2);
    assert_eq!(status.resource_count, 1);
    assert_eq!(status.resource_template_count, 1);
    assert_eq!(status.last_error, None);

    let entries = manager.diagnostics(&config.id).entries;
    let warnings = entries
        .iter()
        .filter(|entry| entry.outcome == McpDiagnosticOutcome::Warning)
        .map(|entry| (entry.code.as_str(), entry.tool_identity.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        warnings,
        [
            ("tool_skipped_undecodable", Some("broken")),
            ("tool_skipped_invalid_input_schema", Some("deep")),
            ("tool_skipped_duplicate_name", Some("echo")),
            ("tool_description_truncated", Some("long")),
            ("tool_discovery_partial", None),
            ("resource_skipped_undecodable", Some("no-uri")),
            ("resource_template_skipped_undecodable", Some("no-template")),
            ("resource_discovery_partial", None),
        ]
    );
    let summary = entries
        .iter()
        .find(|entry| entry.code == "tool_discovery_partial")
        .unwrap();
    assert_eq!(summary.stage, McpDiagnosticStage::DiscoverTools);
    assert_eq!(
        summary.message,
        "discovered 2 MCP tools; skipped 3 and adjusted 1 MCP tools"
    );
    let resources_summary = entries
        .iter()
        .find(|entry| entry.code == "resource_discovery_partial")
        .unwrap();
    assert_eq!(
        resources_summary.message,
        "discovered 1 resources and 1 templates; skipped 1 resources and 1 templates that could not be decoded"
    );
    assert!(!resources_summary.retryable);
    // The status summary counts what this connection attempt recorded and
    // heads with the latest of its most severe entries.
    let current = manager.diagnostic_summary(&config.id);
    assert_eq!((current.errors, current.warnings), (0, 8));
    assert_eq!(
        current.headline.map(|entry| entry.code),
        Some("resource_discovery_partial".to_string())
    );
    assert_eq!(current.entries.len(), entries.len());

    let leases = manager.tool_leases();
    let echo = leases
        .iter()
        .find(|lease| lease.binding.remote_name == "echo")
        .unwrap();
    assert_eq!(echo.binding.specification.description, "Echo");
    let long = leases
        .iter()
        .find(|lease| lease.binding.remote_name == "long")
        .unwrap();
    assert_eq!(
        long.binding.specification.description.len(),
        MAX_MCP_TOOL_DESCRIPTION_BYTES
    );

    let step = manager.step_binding();
    let recorded = manager.diagnostics(&config.id).entries.len();
    let resources = step.servers()[0].list_resources(None).await.unwrap();
    assert_eq!(resources.items.len(), 1);
    assert_eq!(resources.next_cursor, None);
    let templates = step.servers()[0]
        .list_resource_templates(None)
        .await
        .unwrap();
    assert_eq!(templates.items.len(), 1);
    // The Agent's page leaves the bad entries out; each page adds one
    // diagnostic that counts them.
    let page_warnings = manager.diagnostics(&config.id).entries[recorded..]
        .iter()
        .map(|entry| {
            (
                entry.code.clone(),
                entry.message.clone(),
                entry.tool_identity.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        page_warnings,
        [
            (
                "resource_skipped_undecodable".to_string(),
                "an MCP resources page left out 1 entries that could not be decoded".to_string(),
                None
            ),
            (
                "resource_template_skipped_undecodable".to_string(),
                "an MCP resource templates page left out 1 entries that could not be decoded"
                    .to_string(),
                None
            ),
        ]
    );
    let read = step.servers()[0]
        .read_resource("mcp://quirky/doc".into())
        .await
        .unwrap();
    assert_eq!(
        read.content,
        [
            ToolOutputContent::EmbeddedResource {
                uri: "mcp://quirky/doc".into(),
                mime_type: None,
                text: Some("body".into()),
                data: None,
            },
            ToolOutputContent::Text {
                text: "[unsupported MCP content: resource]".into()
            },
            ToolOutputContent::EmbeddedResource {
                uri: "mcp://quirky/blob".into(),
                mime_type: Some("image/png".into()),
                text: None,
                data: Some("AAA=".into()),
            },
        ]
    );

    let output = echo
        .call(
            serde_json::Map::from_iter([("value".into(), json!("x"))]),
            McpCallContext {
                session_id: "session".into(),
                turn_id: 1,
                call_id: "call".into(),
                tool_generation: "generation".into(),
            },
            CancellationToken::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        output.content,
        [
            ToolOutputContent::Text {
                text: "kept".into()
            },
            ToolOutputContent::Text {
                text: "[unsupported MCP content: video]".into()
            },
            ToolOutputContent::Image {
                mime_type: "image/png".into(),
                data: "AAA=".into(),
                detail: None,
            },
        ]
    );

    drop(step);
    drop(leases);
    manager
        .shutdown(Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    assert!(workers
        .shutdown(Instant::now() + Duration::from_secs(2))
        .is_empty());
}

/// Per-entry decoding never loosens the page: an error response and a page
/// over 4 MiB still fail the whole listing, so the server does not connect.
#[tokio::test]
async fn an_error_response_or_an_oversized_page_still_fails_the_listing() {
    for (mode, expected) in [
        ("unauthorized", "Unauthorized"),
        ("oversized", "exceeds local limits"),
    ] {
        let config = quirky_config(mode);
        let executor = Arc::new(ExecutorManager::default());
        let workers = BackgroundWorkerGroup::default();
        let manager = McpManager::new(executor.clone(), workers.clone());
        let error = manager
            .connect_or_refresh(
                config.clone(),
                McpConnectionSecrets::empty(1),
                CancellationToken::default(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{mode}: {error}");
        let status = manager.statuses().remove(0);
        assert_eq!(status.phase, McpConnectionPhase::Failed);
        assert!(manager.tool_leases().is_empty());
        manager
            .shutdown(Instant::now() + Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
        assert!(workers
            .shutdown(Instant::now() + Duration::from_secs(2))
            .is_empty());
    }
}
