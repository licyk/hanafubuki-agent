use super::*;
#[cfg(unix)]
use hanafubuki_agent_approval::ApprovalOrigin;
use hanafubuki_agent_config::McpSettingsStore;
#[cfg(unix)]
use hanafubuki_agent_execution::{AgentExecutionScope, AgentExecutionService, StartCommandRequest};
#[cfg(unix)]
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_mcp::{McpOAuthManager, MemoryMcpSecretStore};
#[cfg(unix)]
use hanafubuki_agent_process::executor::ExecutorManager;
#[cfg(unix)]
use serde_json::Value;
#[cfg(unix)]
use std::collections::BTreeSet;

#[test]
#[cfg(unix)]
fn generic_executor_install_then_mcp_path_connect_is_deterministic_and_cleanup_confirmed() {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "hanafubuki-mcp-generic-install-fixture-{}-{stamp}",
        std::process::id()
    ));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = root.join("package-manager.argv");
    let package_manager = root.join("fake-package-manager");
    hanafubuki_agent_process::test_support::write_executable_script(
        &package_manager,
        &format!(
            "#!/bin/sh\nprintf 'cwd=%s\\n' \"$PWD\" >> '{}'\nprintf '%s\\n' \"$@\" >> '{}'\ncase \"$1\" in\n  install) ln -s \"$2\" \"$3\" ;;\n  uninstall) rm \"$2\" ;;\n  verify-absent) [ ! -e \"$2\" ] ;;\n  *) exit 9 ;;\nesac\n",
            log.display(),
            log.display()
        ),
    );
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
    let installed = bin.join("fixture-mcp");

    let executor = ExecutorManager::default();
    let executions = AgentExecutionService::new(executor.clone());
    let scope = AgentExecutionScope::for_agent_session(77).unwrap();
    let request = StartCommandRequest {
        argv: vec![
            package_manager.to_string_lossy().into_owned(),
            "install".to_string(),
            python,
            installed.to_string_lossy().into_owned(),
        ],
        cwd: Some(root.clone()),
        environment: BTreeMap::new(),
        timeout_ms: Some(5_000),
        output_bytes_cap: Some(4096),
        origin: ApprovalOrigin::Model,
    };
    let action = executions.prepare(&request).unwrap().action().clone();
    let context = hanafubuki_agent_approval::ApprovalContext::from_canonical_bytes(
        "mcp-install-fixture".to_string(),
        b"generic-executor",
    )
    .unwrap();
    let approval = hanafubuki_agent_approval::ApprovalService::new(Arc::new(
        hanafubuki_agent_approval::test_support::ManualApprovalClock::new(1),
    ))
    .authorize(
        action,
        &context,
        Duration::from_secs(30),
        hanafubuki_agent_approval::ApprovalDecisionSource::FullAccess,
    )
    .unwrap();
    let started = executions
        .start(
            scope.clone(),
            request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    let waited = executions
        .wait(
            &scope,
            started.execution_id,
            5_000,
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(waited.completed);
    assert_eq!(waited.snapshot.exit_code, Some(0));
    executions.dispose(&scope, started.execution_id).unwrap();
    let install_log = std::fs::read_to_string(&log).unwrap();
    // `$PWD` is the physical directory: on macOS the temp dir is reached
    // through the `/var` symlink, so only its resolved form matches.
    let cwd = hanafubuki_agent_foundation::paths::ResolvedPath::resolve(&root).unwrap();
    assert!(install_log.contains(&format!("cwd={}\n", cwd.native_text())));
    assert!(install_log.contains(&format!(
        "install\n{}\n{}\n",
        installed.read_link().unwrap().display(),
        installed.display()
    )));

    let workers = BackgroundWorkerGroup::default();
    let manager = Arc::new(hanafubuki_agent_mcp::McpManager::new(
        Arc::new(executor),
        workers.clone(),
    ));
    let frozen_before_install = manager.tool_leases();
    let fixture = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized": continue
    if method == "initialize": result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "installed-fixture", "version": "1"}}
    elif method == "tools/list": result = {"tools": [{"name": "read_fixture", "description": "Read fixture", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}, "annotations": {"readOnlyHint": True, "destructiveHint": False}}]}
    elif method == "tools/call": result = {"content": [{"type": "text", "text": "ok"}], "isError": False}
    else: result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
    let config = hanafubuki_agent_mcp::McpServerConfig {
        id: hanafubuki_agent_mcp::McpServerId::parse("generic-install-fixture").unwrap(),
        enabled: false,
        required: false,
        protocol_mode: hanafubuki_agent_mcp::McpProtocolMode::Legacy,
        transport: hanafubuki_agent_mcp::McpTransportConfig::Stdio(
            hanafubuki_agent_mcp::McpStdioConfig {
                program: "fixture-mcp".to_string(),
                argv: vec!["-u".to_string(), "-c".to_string(), fixture.to_string()],
                cwd: Some(root.clone()),
                public_environment: BTreeMap::from([(
                    "PATH".to_string(),
                    bin.to_string_lossy().into_owned(),
                )]),
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
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let repository = McpSettingsStore::new(root.join("mcp.toml"));
    let application = AgentMcpApplication {
        mcp: manager.clone(),
        mcp_settings: repository,
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
            hanafubuki_agent_mcp::McpHttpProxyPolicy::Disabled,
        )),
        lifetime: Default::default(),
    };
    let disabled_revision = runtime.block_on(async {
        let registered = application
            .mcp_write_server(McpServerWriteRequest {
                expected_revision: 1,
                server: config.clone(),
            })
            .await
            .unwrap();
        assert!(!registered.settings.servers[0].enabled);
        assert!(registered.tools.is_empty());
        let enabled = application
            .mcp_write_server(McpServerWriteRequest {
                expected_revision: registered.settings.revision,
                server: hanafubuki_agent_mcp::McpServerConfig {
                    enabled: true,
                    ..config.clone()
                },
            })
            .await
            .unwrap();
        let connected = application.mcp_connect(config.id.clone()).await.unwrap();
        assert!(frozen_before_install.is_empty());
        assert_eq!(connected.tools.len(), 1);
        let leases = manager.tool_leases();
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].binding.remote_name, "read_fixture");
        let output = leases[0]
            .call(
                Default::default(),
                hanafubuki_agent_mcp::McpCallContext {
                    session_id: "mcp-tool-test".into(),
                    turn_id: 1,
                    call_id: "read-fixture".into(),
                    tool_generation: "fixture-generation".into(),
                },
                CancellationToken::default(),
            )
            .await
            .unwrap();
        assert!(!output.is_error);
        drop(leases);
        application.mcp_disconnect(config.id.clone()).await.unwrap();
        let disabled = application
            .mcp_write_server(McpServerWriteRequest {
                expected_revision: enabled.settings.revision,
                server: hanafubuki_agent_mcp::McpServerConfig {
                    enabled: false,
                    ..config.clone()
                },
            })
            .await
            .unwrap();
        disabled.settings.revision
    });

    let uninstall_request = StartCommandRequest {
        argv: vec![
            package_manager.to_string_lossy().into_owned(),
            "uninstall".to_string(),
            installed.to_string_lossy().into_owned(),
        ],
        cwd: Some(root.clone()),
        environment: BTreeMap::new(),
        timeout_ms: Some(5_000),
        output_bytes_cap: Some(4096),
        origin: ApprovalOrigin::User,
    };
    let action = executions
        .prepare(&uninstall_request)
        .unwrap()
        .action()
        .clone();
    let context = hanafubuki_agent_approval::ApprovalContext::from_canonical_bytes(
        "mcp-uninstall-fixture".to_string(),
        b"generic-executor-uninstall",
    )
    .unwrap();
    let approval = hanafubuki_agent_approval::ApprovalService::new(Arc::new(
        hanafubuki_agent_approval::test_support::ManualApprovalClock::new(2),
    ))
    .authorize(
        action,
        &context,
        Duration::from_secs(30),
        hanafubuki_agent_approval::ApprovalDecisionSource::FullAccess,
    )
    .unwrap();
    let started = executions
        .start(
            scope.clone(),
            uninstall_request,
            Some(&approval),
            &CancellationToken::default(),
        )
        .unwrap();
    assert!(
        executions
            .wait(
                &scope,
                started.execution_id,
                5_000,
                &CancellationToken::default(),
            )
            .unwrap()
            .completed
    );
    executions.dispose(&scope, started.execution_id).unwrap();
    assert!(!installed.exists());

    runtime.block_on(async {
        let deleted = application
            .mcp_delete_server(McpServerDeleteRequest {
                expected_revision: disabled_revision,
                server_id: config.id.clone(),
            })
            .await
            .unwrap();
        assert!(deleted.settings.servers.is_empty());
        manager
            .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(10))
            .await
            .unwrap();
    });
    let lifecycle_log = std::fs::read_to_string(&log).unwrap();
    assert!(lifecycle_log.contains("uninstall\n"));
    assert!(workers
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn native_search_admits_exact_frozen_mcp_call_and_commits_multimodal_output() {
    let arguments = json!({"value": "exact-model-argument"}).to_string();
    let model = FakeServer::start(vec![
        (tool_search_events("echo exact input"), Duration::ZERO),
        (
            namespace_tool_events("mcp__fixture", "echo", &arguments),
            Duration::ZERO,
        ),
        (assistant_events("MCP result received."), Duration::ZERO),
    ])
    .await;
    let (service, paths) = configured_service("mcp-model-e2e", &model.base_url);
    let invocation_log = paths.data_dir.join("mcp-invocation.json");
    let log_literal = serde_json::to_string(invocation_log.to_string_lossy().as_ref()).unwrap();
    let fixture = format!(
        r#"
import json, sys
log_path = {log_literal}
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized":
        continue
    if method == "initialize":
        result = {{"protocolVersion": request["params"]["protocolVersion"], "capabilities": {{"tools": {{}}}}, "serverInfo": {{"name": "fixture", "version": "1"}}, "instructions": "Use the exact fixture connection."}}
    elif method == "tools/list":
        result = {{"tools": [{{"name": "echo", "description": "Echo exact input", "inputSchema": {{"type": "object", "properties": {{"value": {{"type": "string"}}}}, "required": ["value"], "additionalProperties": False}}}}]}}
    elif method == "tools/call":
        with open(log_path, "w", encoding="utf-8") as output:
            json.dump(request["params"], output)
        result = {{"content": [{{"type": "text", "text": request["params"]["arguments"]["value"]}}, {{"type": "image", "mimeType": "image/png", "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="}}], "structuredContent": {{"received": True}}, "isError": False}}
    else:
        result = {{}}
    print(json.dumps({{"jsonrpc": "2.0", "id": request["id"], "result": result}}), flush=True)
"#
    );
    let config = hanafubuki_agent_mcp::McpServerConfig {
        id: hanafubuki_agent_mcp::McpServerId::parse("fixture").unwrap(),
        enabled: true,
        required: true,
        protocol_mode: hanafubuki_agent_mcp::McpProtocolMode::Legacy,
        transport: hanafubuki_agent_mcp::McpTransportConfig::Stdio(
            hanafubuki_agent_mcp::McpStdioConfig {
                program: if cfg!(windows) { "python" } else { "python3" }.into(),
                argv: vec!["-u".into(), "-c".into(), fixture],
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
        maximum_concurrent_calls: 1,
        tool_maximum_concurrent_calls: BTreeMap::new(),
        enabled_tools: Default::default(),
        disabled_tools: Default::default(),
        approval_policy: hanafubuki_agent_mcp::McpApprovalPolicy::Ask,
        tool_approval_policies: BTreeMap::new(),
        exposure_policy: hanafubuki_agent_mcp::McpExposurePolicy::Deferred,
        tool_exposure_policies: BTreeMap::new(),
    };
    let revision = service.mcp_snapshot().await.unwrap().settings.revision;
    let connected = service
        .mcp_write_server(McpServerWriteRequest {
            expected_revision: revision,
            server: config,
        })
        .await
        .unwrap();
    assert!(connected
        .tools
        .iter()
        .any(|tool| tool.model_name == "mcp::fixture::echo"));

    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let session_id = session.id;
    let executing = service.clone();
    let turn = tokio::spawn(async move {
        executing
            .start_turn(session_id, "Call the fixture MCP tool.".into())
            .await
    });
    let (identity, approval) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = service.snapshot(session_id).unwrap();
            if let Some(identity) = snapshot.turn {
                if let Some(approval) = service
                    .events_since(session_id, None)
                    .unwrap()
                    .events
                    .into_iter()
                    .find_map(|event| match event.kind {
                        AgentEventKind::ApprovalRequested { request } => Some(request),
                        _ => None,
                    })
                {
                    break (identity, approval);
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("MCP approval must be published");
    assert_eq!(
        approval.action.kind,
        hanafubuki_agent_approval::ApprovalActionKind::McpToolCall
    );
    service
        .resolve_approval(
            session_id,
            identity.id,
            identity.generation,
            approval.request_id,
            ApprovalUserDecision::AllowOnce,
        )
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), turn)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .final_answer,
        "MCP result received."
    );
    let invoked: Value = serde_json::from_slice(&std::fs::read(&invocation_log).unwrap()).unwrap();
    assert_eq!(invoked["name"], "echo");
    assert_eq!(
        invoked["arguments"],
        json!({"value": "exact-model-argument"})
    );
    let requests = model.requests();
    let first_request: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(first_request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["type"] == "tool_search"));
    assert!(!first_request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "mcp__fixture" || tool["name"] == "mcp__fixture__echo"));
    let search_request: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert!(search_request["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "tool_search_output"
            && item.to_string().contains("mcp__fixture")));
    assert!(!search_request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "mcp__fixture" || tool["name"] == "mcp__fixture__echo"));
    let output_request: Value = serde_json::from_slice(&requests[2].body).unwrap();
    let output = output_request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap();
    assert!(output.to_string().contains("image/png"));
    let transcript = service
        .read_transcript_page(hanafubuki_agent_kernel::AgentTranscriptPageRequest {
            session_id,
            page_size: 100,
            cursor: None,
            direction: hanafubuki_agent_kernel::AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    assert!(serde_json::to_string(&transcript)
        .unwrap()
        .contains("image/png"));
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

#[tokio::test]
async fn optional_mcp_start_failure_warns_while_required_failure_blocks_startup() {
    let model = FakeServer::start(Vec::new()).await;
    let (service, paths) = configured_service("mcp-required-policy", &model.base_url);
    let mut config = hanafubuki_agent_mcp::McpServerConfig {
        id: hanafubuki_agent_mcp::McpServerId::parse("missing-fixture").unwrap(),
        enabled: true,
        required: false,
        protocol_mode: hanafubuki_agent_mcp::McpProtocolMode::Legacy,
        transport: hanafubuki_agent_mcp::McpTransportConfig::Stdio(
            hanafubuki_agent_mcp::McpStdioConfig {
                program: "definitely-missing-mcp-fixture".into(),
                argv: Vec::new(),
                cwd: None,
                public_environment: BTreeMap::new(),
                credential_environment: BTreeMap::new(),
            },
        ),
        connect_timeout_seconds: 1,
        initialize_timeout_seconds: 1,
        list_timeout_seconds: 1,
        call_timeout_seconds: 1,
        shutdown_timeout_seconds: 1,
        maximum_concurrent_calls: 1,
        tool_maximum_concurrent_calls: BTreeMap::new(),
        enabled_tools: Default::default(),
        disabled_tools: Default::default(),
        approval_policy: hanafubuki_agent_mcp::McpApprovalPolicy::Ask,
        tool_approval_policies: BTreeMap::new(),
        exposure_policy: hanafubuki_agent_mcp::McpExposurePolicy::Deferred,
        tool_exposure_policies: BTreeMap::new(),
    };
    let initial = service.mcp_snapshot().await.unwrap();
    let optional = service
        .mcp_write_server(McpServerWriteRequest {
            expected_revision: initial.settings.revision,
            server: config.clone(),
        })
        .await
        .unwrap();
    let warnings = service.start_mcp_servers().await.unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("missing-fixture"));

    config.required = true;
    service
        .mcp_write_server(McpServerWriteRequest {
            expected_revision: optional.settings.revision,
            server: config,
        })
        .await
        .unwrap();
    assert!(service.start_mcp_servers().await.is_err());
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A Skill written for another agent names that product's tools in
/// `allowed-tools`. That list is advisory: the Skill still loads and still
/// applies, and it still cannot put a tool this application does not have in
/// front of the model.
#[cfg(unix)]
#[tokio::test]
async fn an_unrecognised_allowed_tool_neither_hides_a_skill_nor_expands_the_tool_surface() {
    let server = FakeServer::start(vec![(assistant_events("done"), Duration::ZERO)]).await;
    let (service, paths) = configured_service("skill-required-tools", &server.base_url);
    let package = paths
        .data_dir
        .join("agent-test/skills-bundled/requires-missing");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("SKILL.md"),
        "---\nname: requires-missing\ndescription: Names another agent's tools\nallowed-tools:\n  - Bash\n  - not_a_fixed_tool\n---\nUse this guidance.\n",
    )
    .unwrap();
    let entry = service.list_skills().unwrap().entries.remove(0);
    service.enable_skill(&entry.id).unwrap();

    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(
            session.id,
            hanafubuki_agent_kernel::AgentTurnInput {
                text: "use it".into(),
                images: Vec::new(),
                skills: vec![hanafubuki_agent_kernel::AgentSkillRef { skill: entry.id }],
                turn: Default::default(),
                world_state: Default::default(),
                permission_mode: hanafubuki_agent_kernel::AgentPermissionMode::Manual,
                origin: Default::default(),
                provisional_title: None,
            },
        )
        .await
        .unwrap();

    let request: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let input = request["input"].as_array().unwrap();
    assert!(input.iter().any(|item| item["content"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains("Use this guidance."))));
    let tools = request["tools"].as_array().unwrap();
    assert!(!tools
        .iter()
        .any(|tool| tool["name"] == "not_a_fixed_tool" || tool["name"] == "Bash"));
    assert!(service
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// A proxy settings change reaches MCP without a restart, once: an unchanged
/// policy is a no-op, and a proxy the host cannot name leaves the policy in
/// use.
#[tokio::test]
async fn a_proxy_change_rebuilds_the_mcp_client_once() {
    struct Unnamed;

    impl NetworkSettingsPort for Unnamed {
        fn proxy(&self) -> Result<AgentProxy, String> {
            Err("manual proxy mode requires an address".to_string())
        }
    }

    let paths = test_paths("mcp-proxy-change");
    let application = mcp_application(&paths);

    assert!(!application
        .apply_proxy_policy(&AgentProxy::Disabled)
        .await
        .unwrap());
    assert!(application
        .apply_proxy_policy(&AgentProxy::System)
        .await
        .unwrap());
    assert!(!application
        .apply_proxy_policy(&AgentProxy::System)
        .await
        .unwrap());
    assert_eq!(
        *application.proxy.lock(),
        hanafubuki_agent_mcp::McpHttpProxyPolicy::System
    );

    assert!(application.apply_proxy_settings(&Unnamed).await.is_err());
    assert_eq!(
        *application.proxy.lock(),
        hanafubuki_agent_mcp::McpHttpProxyPolicy::System
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

fn mcp_application(paths: &TestPaths) -> AgentMcpApplication {
    AgentMcpApplication {
        mcp: Arc::new(hanafubuki_agent_mcp::McpManager::new(
            Arc::new(hanafubuki_agent_process::executor::ExecutorManager::default()),
            hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup::default(),
        )),
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
            hanafubuki_agent_mcp::McpHttpProxyPolicy::Disabled,
        )),
        lifetime: Default::default(),
    }
}

/// A settings file the application cannot use whole is repaired per server,
/// and the MCP snapshot says what was repaired after the file is clean.
#[tokio::test]
async fn the_mcp_snapshot_reports_settings_repairs() {
    let paths = test_paths("mcp-settings-repairs");
    let server = |id: &str, program: &str, timeout: i64| {
        format!(
            r#"
[[servers]]
id = "{id}"
enabled = false
required = false
connectTimeoutSeconds = {timeout}
initializeTimeoutSeconds = 5
listTimeoutSeconds = 5
callTimeoutSeconds = 5
shutdownTimeoutSeconds = 5
maximumConcurrentCalls = 1
toolMaximumConcurrentCalls = {{}}
enabledTools = []
disabledTools = []
approvalPolicy = "ask"
toolApprovalPolicies = {{}}
exposurePolicy = "deferred"
toolExposurePolicies = {{}}

[servers.transport]
type = "stdio"
program = "{program}"
argv = []
publicEnvironment = {{}}
credentialEnvironment = {{}}
"#
        )
    };
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    std::fs::write(
        paths.data_dir.join("mcp.toml"),
        format!(
            "revision = 3\ncredentials = {{}}\n{}{}",
            server("slow", "fake-mcp", 0),
            server("unusable", "relative/server", 5)
        ),
    )
    .unwrap();
    let application = mcp_application(&paths);

    let snapshot = application.mcp_snapshot().await.unwrap();
    let again = application.mcp_snapshot().await.unwrap();

    assert_eq!(snapshot.settings.servers.len(), 1);
    assert_eq!(snapshot.settings.servers[0].connect_timeout_seconds, 1);
    // One diagnostics summary per configured server, in settings order.
    assert_eq!(
        snapshot
            .diagnostics
            .iter()
            .map(|summary| summary.server_id.as_str())
            .collect::<Vec<_>>(),
        ["slow"]
    );
    assert_eq!(snapshot.diagnostics[0].headline, None);
    let repairs = serde_json::to_value(&again.settings_repairs).unwrap();
    assert_eq!(
        repairs,
        serde_json::json!([
            {"kind": "value_clamped", "subject": "slow", "field": "connectTimeoutSeconds",
             "detail": "must be a whole number from 1 to 86400"},
            {"kind": "server_dropped", "subject": "unusable", "field": null,
             "detail": "MCP configuration is invalid: stdio program must be an absolute executable path or a PATH command name"}
        ])
    );
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
