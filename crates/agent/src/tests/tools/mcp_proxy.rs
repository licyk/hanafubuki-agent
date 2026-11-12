//! A proxy change reaches the MCP stdio servers: they are started anew in
//! the environment the Agent's processes get from then on, so a server
//! started under one proxy reads the next one after the change.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::process::Command;

use hanafubuki_agent_config::McpSettingsStore;
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::environment::ChildProcessEnvironment;
use hanafubuki_agent_mcp::{McpOAuthManager, MemoryMcpSecretStore};
use hanafubuki_agent_process::executor::ExecutorManager;

use super::*;

/// A stdio server whose one tool answers with the proxy variables it was
/// started with.
const FIXTURE: &str = r#"
import json, os, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "initialize": result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "proxy-fixture", "version": "1"}}
    elif method == "tools/list": result = {"tools": [{"name": "proxy", "description": "Its proxy", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}, "annotations": {"readOnlyHint": True, "destructiveHint": False}}]}
    elif method == "tools/call": result = {"content": [{"type": "text", "text": "proxy=" + os.environ.get("HTTPS_PROXY", "-") + ";lower=" + os.environ.get("https_proxy", "-")}], "isError": False}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;

fn proxy_overlay(address: &str) -> HashMap<String, Option<String>> {
    ["HTTPS_PROXY", "https_proxy"]
        .into_iter()
        .map(|name| (name.to_string(), Some(address.to_string())))
        .collect()
}

#[test]
fn a_proxy_change_starts_the_stdio_servers_anew_under_it() {
    let python = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v python3"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let paths = test_paths("mcp-stdio-proxy");
    let processes = ChildProcessEnvironment::default();
    processes
        .replace(proxy_overlay("http://first.proxy:1"))
        .unwrap();
    let manager = Arc::new(hanafubuki_agent_mcp::McpManager::new(
        Arc::new(ExecutorManager::new(processes.clone())),
        BackgroundWorkerGroup::default(),
    ));
    let application = AgentMcpApplication {
        mcp: manager.clone(),
        mcp_settings: McpSettingsStore::new(paths.data_dir.join("mcp.toml")),
        mcp_oauth: McpOAuthManager::new(
            Arc::new(MemoryMcpSecretStore::default()),
            hanafubuki_agent_mcp::build_mcp_http_client(
                hanafubuki_agent_mcp::McpHttpProxyPolicy::Disabled,
            )
            .unwrap(),
        ),
        mcp_configuration_gate: Arc::new(tokio::sync::Mutex::new(())),
        blocking: Arc::new(hanafubuki_agent_foundation::blocking::InlineBlockingOperationPort),
        proxy: Arc::new(parking_lot::Mutex::new(
            hanafubuki_agent_mcp::McpHttpProxyPolicy::Explicit("http://first.proxy:1".into()),
        )),
        lifetime: Default::default(),
        plugin_servers: Default::default(),
    };
    let config = hanafubuki_agent_mcp::McpServerConfig {
        id: hanafubuki_agent_mcp::McpServerId::parse("proxy-fixture").unwrap(),
        enabled: true,
        required: false,
        protocol_mode: hanafubuki_agent_mcp::McpProtocolMode::Legacy,
        transport: hanafubuki_agent_mcp::McpTransportConfig::Stdio(
            hanafubuki_agent_mcp::McpStdioConfig {
                program: python,
                argv: vec!["-u".to_string(), "-c".to_string(), FIXTURE.to_string()],
                cwd: None,
                public_environment: BTreeMap::new(),
                credential_environment: BTreeMap::new(),
            },
        ),
        connect_timeout_seconds: 5,
        initialize_timeout_seconds: 5,
        list_timeout_seconds: 5,
        call_timeout_seconds: 5,
        shutdown_timeout_seconds: 5,
        maximum_concurrent_calls: 2,
        tool_maximum_concurrent_calls: BTreeMap::new(),
        enabled_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        approval_policy: hanafubuki_agent_mcp::McpApprovalPolicy::Ask,
        tool_approval_policies: BTreeMap::new(),
        exposure_policy: hanafubuki_agent_mcp::McpExposurePolicy::Deferred,
        tool_exposure_policies: BTreeMap::new(),
    };
    let called = || async {
        let leases = manager.tool_leases();
        let output = leases[0]
            .call(
                Default::default(),
                hanafubuki_agent_mcp::McpCallContext {
                    session_id: "mcp-proxy-test".into(),
                    turn_id: 1,
                    call_id: "proxy".into(),
                    tool_generation: "proxy-generation".into(),
                },
                CancellationToken::default(),
            )
            .await
            .unwrap();
        format!("{output:?}")
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (before, after) = runtime.block_on(async {
        application
            .mcp_write_server(McpServerWriteRequest {
                expected_revision: 1,
                server: config.clone(),
            })
            .await
            .unwrap();
        application.mcp_connect(config.id.clone()).await.unwrap();
        let before = called().await;

        processes
            .replace(proxy_overlay("http://second.proxy:2"))
            .unwrap();
        assert!(application
            .apply_proxy_policy(&AgentProxy::Explicit("http://second.proxy:2".into()))
            .await
            .unwrap());
        let after = called().await;
        application.mcp_disconnect(config.id.clone()).await.unwrap();
        (before, after)
    });

    assert!(
        before.contains("proxy=http://first.proxy:1;lower=http://first.proxy:1"),
        "{before}"
    );
    assert!(
        after.contains("proxy=http://second.proxy:2;lower=http://second.proxy:2"),
        "{after}"
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
