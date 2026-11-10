use std::path::PathBuf;

use hanafubuki_agent_foundation::paths::ResolvedPath;
use serde_json::json;

use super::*;

fn document(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn stdio(server: &McpServerConfig) -> &McpStdioConfig {
    match &server.transport {
        McpTransportConfig::Stdio(stdio) => stdio,
        McpTransportConfig::StreamableHttp(_) => panic!("{} is not stdio", server.id.as_str()),
    }
}

fn http(server: &McpServerConfig) -> &McpStreamableHttpConfig {
    match &server.transport {
        McpTransportConfig::StreamableHttp(http) => http,
        McpTransportConfig::Stdio(_) => panic!("{} is not HTTP", server.id.as_str()),
    }
}

#[test]
fn both_shapes_of_mcp_json_read_with_claude_codes_keys() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("demo");
    let data = temporary.path().join("data");
    let (root_text, data_text) = (root.display().to_string(), data.display().to_string());
    let wrapped = document(json!({ "mcpServers": {
        "files": {
            "type": "stdio",
            "command": "${CLAUDE_PLUGIN_ROOT}/bin/files",
            "args": ["--data", "${CLAUDE_PLUGIN_DATA}"],
            "env": { "MODE": "plugin" },
            "cwd": "work",
            "tool_timeout_sec": 2.5,
            "disabled_tools": ["drop"],
        },
        "remote": {
            "type": "http",
            "url": "https://example.com/mcp",
            "headers": { "X-Team": "agents" },
            "oauth": { "clientId": "abc", "callbackPort": 8765 },
        },
    }}));

    let outcome = plugin_mcp_servers(&root, &data, &wrapped);

    assert_eq!(outcome.errors, []);
    let [files, remote] = outcome.servers.as_slice() else {
        panic!("two servers: {:?}", outcome.servers);
    };
    let files_stdio = stdio(files);
    assert_eq!(files_stdio.program, format!("{root_text}/bin/files"));
    assert_eq!(files_stdio.argv, ["--data", data_text.as_str()]);
    assert_eq!(files_stdio.cwd, Some(root.join("work")));
    assert_eq!(files_stdio.public_environment["MODE"], "plugin");
    assert_eq!(
        files_stdio.public_environment["CLAUDE_PLUGIN_ROOT"],
        root_text
    );
    assert_eq!(files_stdio.public_environment["PLUGIN_DATA"], data_text);
    assert_eq!(files.call_timeout_seconds, 3);
    assert!(files.disabled_tools.contains("drop"));
    assert!(files.enabled && !files.required);
    assert_eq!(
        files.approval_policy,
        McpServerDefaults::default().approval_policy
    );

    let remote_http = http(remote);
    assert_eq!(remote_http.url, "https://example.com/mcp");
    assert_eq!(remote_http.public_headers["X-Team"], "agents");
    assert_eq!(remote_http.oauth, McpOAuthPolicy::Optional);

    let bare = document(json!({ "tools": { "command": "node", "args": ["server.js"] } }));
    let outcome = plugin_mcp_servers(&root, &data, &bare);
    assert_eq!(outcome.servers.len(), 1);
    assert_eq!(stdio(&outcome.servers[0]).program, "node");
}

#[test]
fn one_bad_server_costs_only_itself() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_path_buf();
    let file = document(json!({ "mcpServers": {
        "good": { "command": "node" },
        "both": { "command": "node", "url": "https://example.com" },
        "neither": { "args": ["x"] },
        "typed": { "command": 7 },
        "bad name": { "command": "node" },
        "token": { "url": "https://example.com", "bearer_token_env_var": "TOKEN" },
    }}));

    let outcome = plugin_mcp_servers(&root, &root, &file);

    let names = outcome
        .servers
        .iter()
        .map(|server| server.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["good"]);
    let mut failed = outcome
        .errors
        .iter()
        .map(|error| error.name.as_str())
        .collect::<Vec<_>>();
    failed.sort();
    assert_eq!(failed, ["bad name", "both", "neither", "token", "typed"]);
}

fn agent_plugins_file(servers: Value) -> String {
    json!({
        "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
        "mcpServers": servers,
    })
    .to_string()
}

#[test]
fn an_agent_plugins_mcp_json_expands_its_placeholders_and_stays_in_its_package() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("package");
    let data = temporary.path().join("data");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::write(root.join("bin/server"), "").unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let root_text = ResolvedPath::resolve(&root)
        .unwrap()
        .native_text()
        .to_string();
    let data_text = ResolvedPath::resolve(&data)
        .unwrap()
        .native_text()
        .to_string();
    let contents = agent_plugins_file(json!({
        "local": {
            "type": "stdio",
            "command": "./bin/server",
            "args": ["${PLUGIN_ROOT}/config", "${PLUGIN_DATA}"],
            "env": { "LEVEL": "${PLUGIN_DATA}/level" },
            "cwd": "${PLUGIN_DATA}",
        },
        "remote": {
            "type": "streamable-http",
            "url": "https://example.com/mcp",
            "headers": { "Authorization": "Bearer x", "X-Team": "agents" },
        },
        "loopback": { "type": "streamable-http", "url": "http://127.0.0.1:9/mcp" },
        "plain": { "type": "streamable-http", "url": "http://example.com/mcp" },
        "escape": { "type": "stdio", "command": "../outside" },
        "reserved": { "type": "stdio", "command": "node", "env": { "PLUGIN_ROOT": "/x" } },
        "unknown": { "type": "stdio", "command": "node", "timeout": 5 },
        "sse": { "type": "sse", "url": "https://example.com/sse" },
        "null": { "type": "stdio", "command": "node", "cwd": null },
    }));

    let outcome = agent_plugin_mcp_servers(&root, &data, &contents).unwrap();

    let names = outcome
        .servers
        .iter()
        .map(|server| server.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["local", "loopback", "remote"]);
    let local = stdio(&outcome.servers[0]);
    assert_eq!(
        local.program,
        ResolvedPath::resolve(root.join("bin").join("server"))
            .unwrap()
            .native_text()
    );
    assert_eq!(
        local.argv,
        [format!("{root_text}/config"), data_text.clone()]
    );
    assert_eq!(
        local.public_environment["LEVEL"],
        format!("{data_text}/level")
    );
    assert_eq!(local.public_environment["PLUGIN_ROOT"], root_text);
    assert_eq!(local.cwd, Some(PathBuf::from(&data_text)));
    let remote = http(&outcome.servers[2]);
    assert_eq!(remote.public_headers.keys().collect::<Vec<_>>(), ["X-Team"]);
    let mut failed = outcome
        .errors
        .iter()
        .map(|error| error.name.as_str())
        .collect::<Vec<_>>();
    failed.sort();
    assert_eq!(
        failed,
        ["escape", "null", "plain", "reserved", "sse", "unknown"]
    );
}

#[test]
fn an_agent_plugins_mcp_json_off_its_schema_fails_as_a_whole() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path();
    for contents in [
        json!({ "mcpServers": {} }).to_string(),
        json!({ "$schema": "https://example.com/other.json", "mcpServers": {} }).to_string(),
        json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": {},
            "extra": true,
        })
        .to_string(),
        "[]".to_string(),
    ] {
        assert!(
            agent_plugin_mcp_servers(path, path, &contents).is_err(),
            "{contents} must fail"
        );
    }
}
