//! A plain `$mention` of a tool the turn can call or of a visible MCP name
//! names that, not a same-named Skill: the `skills` plugin's admission
//! excludes those names from the turn's selectable Skills.

use super::*;
use serde_json::Value;

/// Writes one bundled Skill per `(folder, name)` and turns them all on.
fn enable_skills(service: &AgentApplicationService, paths: &TestPaths, skills: &[(&str, &str)]) {
    for (folder, name) in skills {
        let package = paths
            .data_dir
            .join("agent-test/skills-bundled")
            .join(folder);
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: The {folder} work\n---\nDo the {folder} work.\n"
            ),
        )
        .unwrap();
    }
    for entry in service.list_skills().unwrap().entries {
        service.enable_skill(&entry.id).unwrap();
    }
}

/// The names of the Skills whose instructions the model's first request
/// carries.
fn selected_skill_names(server: &FakeServer) -> Vec<String> {
    let request: Value = serde_json::from_slice(&server.requests()[0].body).unwrap();
    let mut names = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["content"][0]["text"].as_str())
        .filter_map(|text| {
            let name = text.split_once("<skill>\n<name>")?.1;
            Some(name.split_once("</name>")?.0.to_string())
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[tokio::test]
async fn a_mention_of_a_registered_plugin_tool_selects_no_same_named_skill() {
    let server = FakeServer::start(vec![(assistant_events("done"), Duration::ZERO)]).await;
    let (service, paths) = configured_service("skills-mention-plugin-tool", &server.base_url);
    // `skills_list` is the `skills` plugin's tool, `view_image` the
    // `view_image` plugin's.
    enable_skills(
        &service,
        &paths,
        &[
            ("list-helper", "skills_list"),
            ("image-helper", "View_Image"),
            ("diagnostics", "diagnostics"),
        ],
    );
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(
            session.id,
            "$skills_list $view_image and $diagnostics".into(),
        )
        .await
        .unwrap();

    assert_eq!(selected_skill_names(&server), ["diagnostics"]);
    assert!(service
        .shutdown(Instant::now() + Duration::from_secs(1))
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The server, the tool and its flattened alias of a visible MCP tool select
/// no Skill; a Hidden tool's name is not a name the model sees, so it still
/// selects one.
#[cfg(unix)]
#[tokio::test]
async fn a_visible_mcp_name_selects_no_skill_while_a_hidden_tool_name_does() {
    let server = FakeServer::start(vec![(assistant_events("done"), Duration::ZERO)]).await;
    let (service, paths) = configured_service("skills-mention-mcp", &server.base_url);
    let fixture = r#"
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized":
        continue
    if method == "initialize":
        result = {"protocolVersion": request["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}}
    elif method == "tools/list":
        schema = {"type": "object", "properties": {}, "additionalProperties": False}
        result = {"tools": [{"name": "echo", "description": "Echo", "inputSchema": schema}, {"name": "quiet", "description": "Quiet", "inputSchema": schema}]}
    else:
        result = {}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
    let config = hanafubuki_agent_mcp::McpServerConfig {
        id: hanafubuki_agent_mcp::McpServerId::parse("fixture").unwrap(),
        enabled: true,
        required: true,
        protocol_mode: hanafubuki_agent_mcp::McpProtocolMode::Legacy,
        transport: hanafubuki_agent_mcp::McpTransportConfig::Stdio(
            hanafubuki_agent_mcp::McpStdioConfig {
                program: "python3".into(),
                argv: vec!["-u".into(), "-c".into(), fixture.into()],
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
        tool_exposure_policies: BTreeMap::from([(
            "quiet".to_string(),
            hanafubuki_agent_mcp::McpExposurePolicy::Hidden,
        )]),
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
    enable_skills(
        &service,
        &paths,
        &[
            ("server", "fixture"),
            ("tool", "echo"),
            ("alias", "mcp__fixture__echo"),
            ("hidden", "quiet"),
        ],
    );
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    service
        .start_turn(
            session.id,
            "$fixture $echo $mcp__fixture__echo and $quiet".into(),
        )
        .await
        .unwrap();

    assert_eq!(selected_skill_names(&server), ["quiet"]);
    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
