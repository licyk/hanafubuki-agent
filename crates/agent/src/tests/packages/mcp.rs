//! An installed plugin's MCP servers against scripted servers: one runs
//! while its plugin is on and goes once it is off; the user's server and
//! the first plugin in key order come first for a name; the user's
//! settings restrict a plugin's server.

use hanafubuki_agent_mcp::{McpConnectionPhase, McpTransportConfig};
use hanafubuki_agent_packages::{PluginMcpServerSettings, PluginSettings};

use super::*;
use crate::McpServerWriteRequest;

/// An HTTP server declaration for `.mcp.json`, Claude Code's way.
fn http_server(mcp: &ScriptedMcpServers, id: &str) -> serde_json::Value {
    let McpTransportConfig::StreamableHttp(http) =
        mcp.config(id, McpExposurePolicy::Direct).transport
    else {
        unreachable!("the scripted servers are HTTP");
    };
    json!({ "type": "http", "url": http.url })
}

fn running(snapshot: &crate::McpApplicationSnapshot) -> Vec<(String, String)> {
    snapshot
        .plugin_servers
        .iter()
        .map(|server| (server.plugin.clone(), server.server.id.as_str().to_string()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_enabled_plugins_mcp_server_connects_and_goes_when_it_is_turned_off() {
    let mcp = ScriptedMcpServers::start(BTreeMap::from([(
        "plugsrv",
        vec![scripted_tool("lookup", "Look up a record.")],
    )]))
    .await;
    let (service, paths, config) = service_with_config("packages-mcp", |_, _| {});
    let source = claude_plugin(
        &paths,
        "demo",
        json!({ "plugsrv": http_server(&mcp, "plugsrv") }),
        None,
    );
    install(&paths, &source, "demo@local");
    set_enabled(&config, "demo@local", true);

    service.reload_packages().await.unwrap();

    let snapshot = service.mcp_snapshot().await.unwrap();
    assert!(
        snapshot.settings.servers.is_empty(),
        "mcp.toml is untouched"
    );
    let [server] = snapshot.plugin_servers.as_slice() else {
        panic!("one plugin server: {:?}", snapshot.plugin_servers);
    };
    assert_eq!(
        (
            server.plugin.as_str(),
            server.server.id.as_str(),
            server.status.phase
        ),
        ("demo@local", "plugsrv", McpConnectionPhase::Ready)
    );
    assert!(snapshot
        .tools
        .iter()
        .any(|tool| tool.server_id.as_str() == "plugsrv" && tool.remote_name == "lookup"));
    assert_eq!(
        package(&service, "demo@local").await.content.mcp_servers,
        ["plugsrv"]
    );

    set_enabled(&config, "demo@local", false);
    service.reload_packages().await.unwrap();

    let snapshot = service.mcp_snapshot().await.unwrap();
    assert!(snapshot.plugin_servers.is_empty());
    assert!(!snapshot
        .tools
        .iter()
        .any(|tool| tool.server_id.as_str() == "plugsrv"));
    let status = package(&service, "demo@local").await;
    assert!(!status.enabled);
    assert_eq!(status.content, crate::AgentPackageContent::default());
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_server_and_the_first_plugin_come_first_for_a_name() {
    let tools = || vec![scripted_tool("lookup", "Look up a record.")];
    let mcp = ScriptedMcpServers::start(BTreeMap::from([
        ("shared", tools()),
        ("only-a", tools()),
        ("only-b", tools()),
    ]))
    .await;
    let (service, paths, config) = service_with_config("packages-precedence", |_, _| {});
    let first = claude_plugin(
        &paths,
        "first",
        json!({ "shared": http_server(&mcp, "shared"), "only-a": http_server(&mcp, "only-a") }),
        None,
    );
    let second = claude_plugin(
        &paths,
        "second",
        json!({ "only-a": http_server(&mcp, "only-a"), "only-b": http_server(&mcp, "only-b") }),
        None,
    );
    install(&paths, &first, "first@local");
    install(&paths, &second, "second@local");
    service.reload_packages().await.unwrap();

    let snapshot = service.mcp_snapshot().await.unwrap();
    assert_eq!(
        running(&snapshot),
        [
            ("first@local".to_string(), "only-a".to_string()),
            ("first@local".to_string(), "shared".to_string()),
            ("second@local".to_string(), "only-b".to_string()),
        ]
    );
    assert!(package(&service, "second@local").await.issues.contains(
        &"MCP server `only-a` is also declared by `first@local`, which comes first".to_string()
    ));

    // The user's server of the same name takes over, and keeps running.
    service
        .mcp_write_server(McpServerWriteRequest {
            expected_revision: snapshot.settings.revision,
            server: mcp.config("shared", McpExposurePolicy::Direct),
        })
        .await
        .unwrap();
    service.reload_packages().await.unwrap();

    let snapshot = service.mcp_snapshot().await.unwrap();
    assert_eq!(
        running(&snapshot),
        [
            ("first@local".to_string(), "only-a".to_string()),
            ("second@local".to_string(), "only-b".to_string()),
        ]
    );
    assert_eq!(snapshot.servers[0].phase, McpConnectionPhase::Ready);
    assert!(package(&service, "first@local").await.issues.contains(
        &"MCP server `shared` is also configured in mcp.toml, which comes first".to_string()
    ));

    set_enabled(&config, "first@local", false);
    service.reload_packages().await.unwrap();
    let snapshot = service.mcp_snapshot().await.unwrap();
    assert_eq!(
        running(&snapshot),
        [
            ("second@local".to_string(), "only-a".to_string()),
            ("second@local".to_string(), "only-b".to_string()),
        ]
    );
    assert_eq!(snapshot.servers[0].phase, McpConnectionPhase::Ready);
    finish(service, paths).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_settings_restrict_a_plugins_server() {
    let mcp = ScriptedMcpServers::start(BTreeMap::from([
        (
            "files",
            vec![
                scripted_tool("lookup", "Look up a record."),
                scripted_tool("echo", "Echo the value."),
            ],
        ),
        ("off", vec![scripted_tool("lookup", "Look up a record.")]),
    ]))
    .await;
    let (service, paths, config) = service_with_config("packages-overlay", |_, _| {});
    let source = claude_plugin(
        &paths,
        "demo",
        json!({ "files": http_server(&mcp, "files"), "off": http_server(&mcp, "off") }),
        None,
    );
    install(&paths, &source, "demo@local");
    config
        .set_package_settings(
            &PluginId::parse("demo@local").unwrap(),
            Some(&PluginSettings {
                enabled: Some(true),
                mcp_servers: BTreeMap::from([
                    (
                        "files".to_string(),
                        PluginMcpServerSettings {
                            disabled_tools: Some(vec!["lookup".to_string()]),
                            ..PluginMcpServerSettings::default()
                        },
                    ),
                    (
                        "off".to_string(),
                        PluginMcpServerSettings {
                            enabled: Some(false),
                            ..PluginMcpServerSettings::default()
                        },
                    ),
                ]),
                settings: None,
            }),
        )
        .unwrap();

    service.reload_packages().await.unwrap();

    let snapshot = service.mcp_snapshot().await.unwrap();
    let tools = snapshot
        .tools
        .iter()
        .map(|tool| (tool.server_id.as_str(), tool.remote_name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(tools, [("files", "echo")]);
    assert_eq!(
        package(&service, "demo@local").await.content.mcp_servers,
        ["files"]
    );
    finish(service, paths).await;
}
