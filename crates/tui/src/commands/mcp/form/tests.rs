use std::collections::{BTreeMap, BTreeSet};

use hanafubuki_agent::{
    McpApplicationSnapshot, McpApprovalPolicy, McpExposurePolicy, McpProtocolMode, McpServerConfig,
    McpServerId, McpStdioConfig, McpTransportConfig,
};

use super::*;

fn snapshot() -> McpApplicationSnapshot {
    McpApplicationSnapshot {
        settings: Default::default(),
        servers: Vec::new(),
        oauth: Vec::new(),
        tools: Vec::new(),
        hidden_tool_count: 0,
        pending_elicitations: Vec::new(),
        diagnostics: Vec::new(),
        settings_repairs: Vec::new(),
        server_defaults: Default::default(),
        plugin_servers: Vec::new(),
    }
}

fn server() -> McpServerConfig {
    McpServerConfig {
        id: McpServerId::parse("tools").unwrap(),
        enabled: true,
        required: false,
        protocol_mode: McpProtocolMode::V20260728,
        transport: McpTransportConfig::Stdio(McpStdioConfig {
            program: "sh".into(),
            argv: vec!["-c".into(), "exit 0".into()],
            cwd: None,
            public_environment: BTreeMap::new(),
            credential_environment: BTreeMap::new(),
        }),
        connect_timeout_seconds: 15,
        initialize_timeout_seconds: 30,
        list_timeout_seconds: 30,
        call_timeout_seconds: 120,
        shutdown_timeout_seconds: 10,
        maximum_concurrent_calls: 4,
        tool_maximum_concurrent_calls: BTreeMap::from([("slow".into(), 2)]),
        enabled_tools: BTreeSet::new(),
        disabled_tools: BTreeSet::new(),
        approval_policy: McpApprovalPolicy::Ask,
        tool_approval_policies: BTreeMap::from([("delete".into(), McpApprovalPolicy::Deny)]),
        exposure_policy: McpExposurePolicy::Deferred,
        tool_exposure_policies: BTreeMap::from([("search".into(), McpExposurePolicy::Direct)]),
    }
}

#[test]
fn the_form_reads_and_writes_the_protocol_and_every_per_tool_map() {
    let server = server();
    let snapshot = snapshot();
    let mut form = server_form(Some(&server), &snapshot);

    assert_eq!(form.choice(PROTOCOL), 1);
    let limits = form.rows(TOOL_CALLS);
    assert_eq!(limits[0].name, "slow");
    assert_eq!(limits[0].value, "2");
    let approvals = form.rows(TOOL_APPROVALS);
    assert_eq!(approvals[0].name, "delete");
    assert_eq!(approvals[0].value, "deny");
    let exposures = form.rows(TOOL_EXPOSURES);
    assert_eq!(exposures[0].name, "search");
    assert_eq!(exposures[0].value, "direct");

    let write = server_write(&mut form, Some(&server), &snapshot).unwrap();
    assert_eq!(write.server.protocol_mode, McpProtocolMode::V20260728);
    assert_eq!(
        write.server.tool_maximum_concurrent_calls,
        BTreeMap::from([("slow".into(), 2)])
    );
    assert_eq!(
        write.server.tool_approval_policies,
        BTreeMap::from([("delete".into(), McpApprovalPolicy::Deny)])
    );
    assert_eq!(
        write.server.tool_exposure_policies,
        BTreeMap::from([("search".into(), McpExposurePolicy::Direct)])
    );
}

#[test]
fn invalid_per_tool_values_are_refused() {
    let row = |value: &str| RowValue {
        name: "tool".into(),
        value: value.into(),
        secret: false,
    };

    assert!(tool_limits(vec![row("0")]).is_err());
    assert!(tool_limits(vec![row("65")]).is_err());
    assert!(tool_limits(vec![row("many")]).is_err());
    assert!(tool_approvals(vec![row("sometimes")]).is_err());
    assert!(tool_exposures(vec![row("sometimes")]).is_err());

    assert_eq!(
        tool_approvals(vec![row("ASK")]).unwrap()["tool"],
        McpApprovalPolicy::Ask
    );
    assert_eq!(
        tool_exposures(vec![row("direct-model-only")]).unwrap()["tool"],
        McpExposurePolicy::DirectModelOnly
    );
}
