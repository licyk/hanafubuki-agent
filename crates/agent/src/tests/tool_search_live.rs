//! Opt-in discovery probe against Hoshimi and an installed filesystem MCP server.

use super::*;
use hanafubuki_agent_kernel::ConversationItem;
use hanafubuki_agent_mcp::{McpServerDefaults, McpStdioConfig, McpTransportConfig};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires HOSHIMI_TOOL_TEST_URL and MCP_FILESYSTEM_ENTRY (installed server-filesystem)"]
async fn live_hoshimi_discovers_and_calls_default_mcp_tools() {
    let url = std::env::var("HOSHIMI_TOOL_TEST_URL").unwrap();
    let entry = std::env::var("MCP_FILESYSTEM_ENTRY").unwrap();
    for protocol in [
        AgentModelProtocol::Responses,
        AgentModelProtocol::ChatCompletions,
        AgentModelProtocol::AnthropicMessages,
    ] {
        if std::env::var("HOSHIMI_TOOL_TEST_PROTOCOL")
            .is_ok_and(|selected| selected != format!("{protocol:?}"))
        {
            continue;
        }
        let (service, paths) = service_with("tool-search-live", |_, config| {
            configure_test_provider(config, &url, Some(ModelReasoningEffort::Low));
            config
                .mutate_providers(1, |registry| {
                    registry.providers[0].protocol = protocol;
                    registry.model_policy.model = "DeepSeek-Flash".into();
                    Ok(())
                })
                .unwrap();
        });
        let fixture = paths.managed_instances_dir.join("discovery-probe.txt");
        let token = format!(
            "MCP_DISCOVERY_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::fs::write(&fixture, &token).unwrap();
        let mut config = serde_json::to_value(McpServerDefaults::default()).unwrap();
        config["id"] = json!("filesystem");
        config["transport"] = json!(McpTransportConfig::Stdio(McpStdioConfig {
            program: "node".into(),
            argv: vec![
                entry.clone(),
                paths.managed_instances_dir.to_string_lossy().into()
            ],
            cwd: None,
            public_environment: BTreeMap::new(),
            credential_environment: BTreeMap::new(),
        }));
        for field in [
            "toolMaximumConcurrentCalls",
            "toolApprovalPolicies",
            "toolExposurePolicies",
        ] {
            config[field] = json!({});
        }
        config["enabledTools"] = json!([]);
        config["disabledTools"] = json!([]);
        let connected = service
            .mcp_write_server(McpServerWriteRequest {
                expected_revision: service.mcp_snapshot().await.unwrap().settings.revision,
                server: serde_json::from_value(config).unwrap(),
            })
            .await
            .unwrap();
        assert!(!connected.tools.is_empty(), "{connected:?}");
        assert!(connected
            .tools
            .iter()
            .all(|tool| tool.exposure == McpExposurePolicy::Deferred));
        let session = service
            .create_session(AgentSessionMode::Default, None)
            .await
            .unwrap();
        let prompt = format!(
            "Test MCP discovery. First use tool_list with source mcp.filesystem to list tool names. Then use the ordinary function search_tools with the exact listed name of read_text_file and limit 1 to obtain its schema. Finally call that MCP tool to read {} and reply with the exact file contents. Do not use exec_command, shell, or other filesystem tools. If discovery is unavailable or fails, report the failure and stop; do not retry more than twice.",
            fixture.display()
        );
        let outcome = tokio::time::timeout(Duration::from_secs(180), async {
            let mut turn = Box::pin(service.start_turn(session.id, prompt.into()));
            let mut poll = tokio::time::interval(Duration::from_millis(100));
            let mut approved = Vec::new();
            loop {
                tokio::select! {
                    result = &mut turn => break result,
                    _ = poll.tick() => {
                        for event in service.events_since(session.id, None).unwrap().events {
                            if let AgentEventKind::ApprovalRequested { request } = event.kind {
                                if approved.contains(&request.request_id) {
                                    continue;
                                }
                                assert_eq!(request.action.kind, hanafubuki_agent_approval::ApprovalActionKind::McpToolCall);
                                let identity = service.snapshot(session.id).unwrap().turn.unwrap();
                                service.resolve_approval(session.id, identity.id, identity.generation, request.request_id, ApprovalUserDecision::AllowOnce).unwrap();
                                approved.push(request.request_id);
                            }
                        }
                    }
                }
            }
        }).await;
        let context = service.core.test_model_context(session.id).unwrap();
        let calls = context
            .iter()
            .filter_map(|item| match item {
                ConversationItem::ToolCall {
                    namespace,
                    name,
                    arguments,
                    ..
                } => Some(json!({"namespace":namespace,"name":name,"arguments":arguments})),
                _ => None,
            })
            .collect::<Vec<_>>();
        println!(
            "LIVE {}",
            json!({"protocol":format!("{protocol:?}"),"mcp_tools":connected.tools.len(),"calls":calls,"outcome":format!("{outcome:?}")})
        );
        let succeeded = outcome.as_ref().is_ok_and(|outcome| {
            outcome
                .as_ref()
                .is_ok_and(|outcome| outcome.final_answer.contains(&token))
        });
        let read_returned_token = context.iter().any(|item| matches!(item, ConversationItem::ToolOutput { output, .. } if serde_json::to_string(output).unwrap().contains(&token)));
        let deadline = Instant::now() + TEST_SHUTDOWN_DEADLINE;
        let stopping = service.clone();
        assert!(tokio::task::spawn_blocking(
            move || stopping.stop_accepting_and_cancel_turns(deadline)
        )
        .await
        .unwrap()
        .is_empty());
        service.shutdown_mcp(deadline).await.unwrap();
        assert!(service.shutdown_plugins(deadline).await.is_empty());
        assert!(
            tokio::task::spawn_blocking(move || service.shutdown_workers_after_mcp(deadline))
                .await
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(paths.app_data_dir).unwrap();
        assert!(
            succeeded && read_returned_token,
            "{protocol:?}: discovery/read failed"
        );
        assert!(calls.iter().any(|call| call["name"] == "tool_list"));
        assert!(calls.iter().any(|call| call["name"] == "search_tools"));
    }
}
