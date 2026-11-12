use super::*;

#[test]
fn mcp_identifiers_reject_path_components() {
    assert!(McpServerId::parse(".").is_err());
    assert!(McpServerId::parse("..").is_err());
    assert!(McpCredentialKey::parse("..").is_err());
}

#[test]
fn stdio_config_separates_public_and_secret_references() {
    let config = stdio_config();
    config.validate().unwrap();
    let encoded = serde_json::to_string(&config).unwrap();
    assert!(encoded.contains("mcp-local-token"));
    assert!(!encoded.contains("secret-value"));

    let mut overlap = config;
    let McpTransportConfig::Stdio(transport) = &mut overlap.transport else {
        unreachable!()
    };
    transport
        .public_environment
        .insert("TOKEN".into(), "not-allowed".into());
    assert!(overlap.validate().is_err());
}

/// The defaults a new server's form starts with make a valid server once
/// the user gives it an id and a transport.
#[test]
fn new_server_defaults_form_a_valid_server() {
    let defaults = McpServerDefaults::default();
    let mut config = stdio_config();
    config.enabled = defaults.enabled;
    config.required = defaults.required;
    config.protocol_mode = defaults.protocol_mode;
    config.connect_timeout_seconds = defaults.connect_timeout_seconds;
    config.initialize_timeout_seconds = defaults.initialize_timeout_seconds;
    config.list_timeout_seconds = defaults.list_timeout_seconds;
    config.call_timeout_seconds = defaults.call_timeout_seconds;
    config.shutdown_timeout_seconds = defaults.shutdown_timeout_seconds;
    config.maximum_concurrent_calls = defaults.maximum_concurrent_calls;
    config.approval_policy = defaults.approval_policy;
    config.exposure_policy = defaults.exposure_policy;
    config.validate().unwrap();
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: "https://example.test/mcp".into(),
        public_headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: defaults.oauth,
    });
    config.validate().unwrap();
}

#[test]
fn streamable_http_rejects_credentials_in_url_and_header_overlap() {
    let mut config = stdio_config();
    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: "https://user:password@example.test/mcp".into(),
        public_headers: BTreeMap::new(),
        credential_headers: BTreeMap::new(),
        oauth: McpOAuthPolicy::Optional,
    });
    assert!(config.validate().is_err());

    config.transport = McpTransportConfig::StreamableHttp(McpStreamableHttpConfig {
        url: "https://example.test/mcp".into(),
        public_headers: BTreeMap::from([("x-key".into(), "public".into())]),
        credential_headers: BTreeMap::from([(
            "x-key".into(),
            McpCredentialKey::parse("private-key").unwrap(),
        )]),
        oauth: McpOAuthPolicy::Optional,
    });
    assert!(config.validate().is_err());
}

#[test]
fn repair_drops_unknown_fields_and_keeps_every_value() {
    let mut server = serde_json::to_value(stdio_config()).unwrap();
    server["removedServerField"] = serde_json::Value::Bool(true);
    server["transport"]["removedTransportField"] = serde_json::Value::Bool(true);
    server["transport"]["publicEnvironment"]["DYNAMIC/KEY"] =
        serde_json::Value::String("preserved".to_string());
    let (settings, repairs) = repair_settings(json!({
        "revision": 3,
        "servers": [server],
        "credentials": {
            "mcp-local-token": {
                "value": "secret-value",
                "generation": 7,
                "removedCredentialField": true
            }
        },
        "removedField": true
    }));

    assert!(repairs.is_empty());
    assert_eq!(settings.revision, 3);
    assert_eq!(settings.servers.len(), 1);
    let McpTransportConfig::Stdio(transport) = &settings.servers[0].transport else {
        panic!("expected stdio transport");
    };
    assert_eq!(
        transport.public_environment.get("DYNAMIC/KEY"),
        Some(&"preserved".to_string())
    );
    assert_eq!(
        settings
            .credentials
            .get(&McpCredentialKey::parse("mcp-local-token").unwrap())
            .map(|credential| (credential.value.as_str(), credential.generation)),
        Some(("secret-value", 7))
    );
}

fn settings_document(servers: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "revision": 2,
        "servers": servers,
        "credentials": {
            "mcp-local-token": {"value": "secret-value", "generation": 0}
        }
    })
}

fn named_server(id: &str) -> serde_json::Value {
    let mut server = serde_json::to_value(stdio_config()).unwrap();
    server["id"] = json!(id);
    server
}

#[test]
fn repair_keeps_the_protocol_mode_without_a_repair() {
    let mut server = stdio_config();
    server.protocol_mode = McpProtocolMode::V20260728;
    let settings = McpSettings {
        revision: 4,
        servers: vec![server],
        credentials: BTreeMap::new(),
    };

    assert_eq!(
        repair_settings(serde_json::to_value(&settings).unwrap()),
        (settings, Vec::new())
    );
}

#[test]
fn settings_repair_clamps_numbers_and_reports_every_dropped_entry() {
    let mut clamped = named_server("clamped");
    clamped["connectTimeoutSeconds"] = json!(0);
    clamped["callTimeoutSeconds"] = json!(1_000_000_000_000u64);
    clamped["listTimeoutSeconds"] = json!(-5);
    clamped["shutdownTimeoutSeconds"] = json!(2.5);
    clamped["maximumConcurrentCalls"] = json!(500);
    clamped["toolMaximumConcurrentCalls"] = json!({"slow": 0, "fast": 9000});
    let mut unreadable = named_server("unreadable");
    unreadable["transport"] = json!({"type": "sse", "url": "https://example.test/sse"});
    let mut credential_url = named_server("credential-url");
    credential_url["transport"] = json!({
        "type": "streamable_http",
        "url": "https://user:password@example.test/mcp",
        "publicHeaders": {},
        "credentialHeaders": {},
        "oauth": "disabled"
    });
    let mut bad_environment = named_server("bad-environment");
    bad_environment["transport"]["publicEnvironment"] = json!({"A=B": "value"});
    let mut overlap = named_server("overlap");
    overlap["transport"]["publicEnvironment"] = json!({"TOKEN": "public"});
    let mut bad_program = named_server("bad-program");
    bad_program["transport"]["program"] = json!("../bin/server");
    let bad_id = named_server("bad id");
    let (settings, repairs) = repair_settings(settings_document(vec![
        named_server("kept"),
        clamped,
        unreadable,
        json!("not a server"),
        credential_url,
        bad_environment,
        overlap,
        bad_program,
        bad_id,
        named_server("kept"),
    ]));

    let ids = settings
        .servers
        .iter()
        .map(|server| server.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["kept", "clamped"]);
    let server = &settings.servers[1];
    assert_eq!(server.connect_timeout_seconds, 1);
    assert_eq!(server.call_timeout_seconds, MAX_MCP_TIMEOUT_SECONDS);
    assert_eq!(server.list_timeout_seconds, 1);
    assert_eq!(server.shutdown_timeout_seconds, 2);
    assert_eq!(server.maximum_concurrent_calls, MAX_MCP_CONCURRENT_CALLS);
    assert_eq!(
        server.tool_maximum_concurrent_calls,
        BTreeMap::from([
            ("fast".into(), MAX_MCP_CONCURRENT_CALLS),
            ("slow".into(), 1)
        ])
    );
    // A generation is change detection: it is clamped, and the value it
    // guards is kept.
    let credential = &settings.credentials[&McpCredentialKey::parse("mcp-local-token").unwrap()];
    assert_eq!(
        (credential.value.as_str(), credential.generation),
        ("secret-value", 1)
    );

    let clamped_fields = repairs
        .iter()
        .filter(|repair| repair.kind == McpSettingsRepairKind::ValueClamped)
        .map(|repair| (repair.subject.as_deref(), repair.field.as_deref().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        clamped_fields,
        [
            (Some("clamped"), "connectTimeoutSeconds"),
            (Some("clamped"), "listTimeoutSeconds"),
            (Some("clamped"), "callTimeoutSeconds"),
            (Some("clamped"), "shutdownTimeoutSeconds"),
            (Some("clamped"), "maximumConcurrentCalls"),
            (Some("clamped"), "toolMaximumConcurrentCalls"),
            (Some("mcp-local-token"), "generation"),
        ]
    );
    let dropped = repairs
        .iter()
        .filter(|repair| repair.kind == McpSettingsRepairKind::ServerDropped)
        .map(|repair| (repair.subject.as_deref(), repair.detail.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(dropped.len(), 8);
    assert_eq!(
        dropped
            .iter()
            .map(|(subject, _)| *subject)
            .collect::<Vec<_>>(),
        [
            Some("unreadable"),
            None,
            Some("credential-url"),
            Some("bad-environment"),
            Some("overlap"),
            Some("bad-program"),
            None,
            Some("kept"),
        ]
    );
    assert!(dropped[2].1.contains("without credentials"));
    assert!(dropped[3].1.contains("environment variable name"));
    assert!(dropped[4].1.contains("overlap"));
    assert!(dropped[5].1.contains("stdio program"));
    assert!(dropped[6].1.contains("server id"));
    assert!(dropped[7].1.contains("same id"));
    // Nothing a report carries comes from a value in the file.
    let encoded = serde_json::to_string(&repairs).unwrap();
    assert!(!encoded.contains("secret-value") && !encoded.contains("password"));
}

/// A key a server leaves out takes what a server the user adds starts with,
/// and the server is kept without a repair.
#[test]
fn a_server_missing_a_key_takes_a_new_servers_value() {
    let defaults = serde_json::to_value(McpServerDefaults::default()).unwrap();
    let http = json!({
        "type": "streamable_http",
        "url": "https://example.test/mcp",
        "publicHeaders": {"x-team": "agents"},
        "credentialHeaders": {"x-key": "mcp-local-token"},
        "oauth": "optional"
    });
    let mut cases = defaults
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, _)| *key != "oauth")
        .map(|(key, value)| (None, key.as_str(), value.clone()))
        .collect::<Vec<_>>();
    cases.extend([
        (None, "enabledTools", json!([])),
        (None, "disabledTools", json!([])),
        (None, "toolMaximumConcurrentCalls", json!({})),
        (None, "toolApprovalPolicies", json!({})),
        (None, "toolExposurePolicies", json!({})),
        (Some("stdio"), "argv", json!([])),
        (Some("stdio"), "cwd", json!(null)),
        (Some("stdio"), "publicEnvironment", json!({})),
        (Some("stdio"), "credentialEnvironment", json!({})),
        (Some("streamable_http"), "publicHeaders", json!({})),
        (Some("streamable_http"), "credentialHeaders", json!({})),
        (Some("streamable_http"), "oauth", defaults["oauth"].clone()),
    ]);
    for (transport, key, expected) in cases {
        let mut server = named_server("partial");
        server["toolExposurePolicies"] = json!({"search": "direct"});
        if transport == Some("streamable_http") {
            server["transport"] = http.clone();
        }
        let table = match transport {
            Some(_) => &mut server["transport"],
            None => &mut server,
        };
        assert!(
            table.as_object_mut().unwrap().remove(key).is_some(),
            "{key}"
        );
        let (settings, repairs) = repair_settings(json!({"revision": 1, "servers": [server]}));

        assert!(repairs.is_empty(), "{key}: {repairs:?}");
        assert_eq!(settings.servers.len(), 1, "{key}");
        let read = serde_json::to_value(&settings.servers[0]).unwrap();
        let table = match transport {
            Some(_) => &read["transport"],
            None => &read,
        };
        assert_eq!(table[key], expected, "{key}");
    }
}

/// A server without what it runs or reaches, or without an id, is still
/// dropped alone.
#[test]
fn a_server_without_its_id_transport_program_or_url_is_dropped() {
    let mut no_program = named_server("no-program");
    no_program["transport"]
        .as_object_mut()
        .unwrap()
        .remove("program");
    let mut no_url = named_server("no-url");
    no_url["transport"] = json!({"type": "streamable_http"});
    let mut no_type = named_server("no-type");
    no_type["transport"].as_object_mut().unwrap().remove("type");
    let mut no_transport = named_server("no-transport");
    no_transport.as_object_mut().unwrap().remove("transport");
    let mut no_id = named_server("no-id");
    no_id.as_object_mut().unwrap().remove("id");
    let (settings, repairs) = repair_settings(json!({
        "revision": 1,
        "servers": [no_program, named_server("kept"), no_url, no_type, no_transport, no_id]
    }));

    let ids = settings
        .servers
        .iter()
        .map(|server| server.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["kept"]);
    assert!(repairs.iter().all(|repair| {
        repair.kind == McpSettingsRepairKind::ServerDropped
            && repair.detail == "the entry is not a readable MCP server"
    }));
    let dropped = repairs
        .iter()
        .map(|repair| repair.subject.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        dropped,
        [
            Some("no-program"),
            Some("no-url"),
            Some("no-type"),
            Some("no-transport"),
            None
        ]
    );
}
