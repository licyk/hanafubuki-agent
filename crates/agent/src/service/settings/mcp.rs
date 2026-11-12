//! The MCP servers of `mcp.toml`, written through its revision-gated
//! calls, and their connections' actions. The transport's public
//! environment or headers and the per-tool maps are lists of `NAME=VALUE`
//! texts; the secret ones are the `mcpSecrets` collection
//! ([`super::mcp_secrets`]). An OAuth login, which waits for the browser,
//! is not an action: the host calls `mcp_oauth_login`.

use hanafubuki_agent_config::{
    SettingChoice, SettingCondition, SettingKind, SettingSpec, SettingValue, SettingsGroup,
};
use hanafubuki_agent_mcp::{
    McpApprovalPolicy, McpConnectionPhase, McpExposurePolicy, McpOAuthPolicy, McpProtocolMode,
    McpServerConfig, McpServerId, McpTransportConfig, MAX_MCP_CONCURRENT_CALLS,
    MAX_MCP_TIMEOUT_SECONDS,
};
use serde_json::{json, Map, Value};

use super::mcp_secrets::{credential_names, drop_unreferenced};
use super::record::{lists_as_maps, maps_as_lists, merged, record_of, with_defaults, PairValue};
use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, ADD,
    DELETE, SAVE,
};
use crate::service::AgentApplicationService;
use crate::{McpServerDeleteRequest, McpServerWriteRequest};

pub(super) const ID: &str = "mcp";
const CONNECT: &str = "connect";
const DISCONNECT: &str = "disconnect";
const REFRESH: &str = "refresh";
const LOGOUT: &str = "logout";
const STDIO: &str = "stdio";
const STREAMABLE_HTTP: &str = "streamable_http";
/// The maps of a server, as lists of `NAME=VALUE` texts.
const MAPS: [(&str, PairValue); 5] = [
    ("transport.publicEnvironment", PairValue::Text),
    ("transport.publicHeaders", PairValue::Text),
    ("toolMaximumConcurrentCalls", PairValue::Integer),
    ("toolApprovalPolicies", PairValue::Text),
    ("toolExposurePolicies", PairValue::Text),
];

fn fields() -> Vec<SettingSpec> {
    let choice = |values: Vec<(String, &str)>| SettingKind::Choice {
        values: values
            .into_iter()
            .map(|(value, label)| SettingChoice::new(value, label))
            .collect(),
    };
    let text = |nullable| SettingKind::Text {
        maximum_chars: None,
        secret: false,
        nullable,
    };
    let list = SettingKind::TextList {
        maximum_items: None,
        maximum_item_chars: None,
    };
    let on = |transport: &str| {
        Some(SettingCondition {
            key: "transport.type".into(),
            equals: SettingValue::Text(transport.into()),
        })
    };
    let timeout = |key: &str, label: &str| SettingSpec {
        advanced: true,
        ..SettingSpec::new(
            key,
            label,
            SettingKind::Integer {
                minimum: Some(1),
                maximum: Some(MAX_MCP_TIMEOUT_SECONDS as i64),
            },
        )
    };
    vec![
        SettingSpec::new("enabled", "Enabled", SettingKind::Boolean),
        SettingSpec {
            summary: "A step fails when its tools cannot be used".into(),
            ..SettingSpec::new("required", "Required", SettingKind::Boolean)
        },
        SettingSpec::new(
            "transport.type",
            "Transport",
            choice(vec![
                (STDIO.into(), "Standard input and output"),
                (STREAMABLE_HTTP.into(), "Streamable HTTP"),
            ]),
        ),
        SettingSpec {
            visible_if: on(STDIO),
            ..SettingSpec::new("transport.program", "Program", text(false))
        },
        SettingSpec {
            visible_if: on(STDIO),
            ..SettingSpec::new("transport.argv", "Arguments", list.clone())
        },
        SettingSpec {
            visible_if: on(STDIO),
            ..SettingSpec::new("transport.cwd", "Working directory", text(true))
        },
        SettingSpec {
            visible_if: on(STDIO),
            summary: "Each NAME=VALUE; a secret one is in MCP secrets".into(),
            ..SettingSpec::new("transport.publicEnvironment", "Environment", list.clone())
        },
        SettingSpec {
            visible_if: on(STREAMABLE_HTTP),
            ..SettingSpec::new("transport.url", "URL", text(false))
        },
        SettingSpec {
            visible_if: on(STREAMABLE_HTTP),
            ..SettingSpec::new(
                "transport.oauth",
                "OAuth",
                choice(vec![
                    (name_of(&McpOAuthPolicy::Disabled), "Off"),
                    (name_of(&McpOAuthPolicy::Optional), "Optional"),
                    (name_of(&McpOAuthPolicy::Required), "Required"),
                ]),
            )
        },
        SettingSpec {
            advanced: true,
            ..SettingSpec::new(
                "protocolMode",
                "Protocol",
                choice(vec![
                    (name_of(&McpProtocolMode::Legacy), "Legacy"),
                    (name_of(&McpProtocolMode::V20260728), "2026-07-28"),
                ]),
            )
        },
        SettingSpec::new(
            "approvalPolicy",
            "Approval",
            choice(vec![
                (name_of(&McpApprovalPolicy::Allow), "Allow"),
                (name_of(&McpApprovalPolicy::Ask), "Ask"),
                (name_of(&McpApprovalPolicy::Deny), "Deny"),
            ]),
        ),
        SettingSpec::new(
            "exposurePolicy",
            "Exposure",
            choice(vec![
                (name_of(&McpExposurePolicy::Direct), "Direct"),
                (
                    name_of(&McpExposurePolicy::DirectModelOnly),
                    "Direct, model only",
                ),
                (name_of(&McpExposurePolicy::Deferred), "Deferred"),
                (name_of(&McpExposurePolicy::Hidden), "Hidden"),
            ]),
        ),
        SettingSpec::new("enabledTools", "Only these tools", list.clone()),
        SettingSpec::new("disabledTools", "Not these tools", list.clone()),
        SettingSpec {
            advanced: true,
            summary: "Each TOOL=allow, ask or deny".into(),
            ..SettingSpec::new("toolApprovalPolicies", "Approval per tool", list.clone())
        },
        SettingSpec {
            advanced: true,
            summary: "Each TOOL=direct, direct_model_only, deferred or hidden".into(),
            ..SettingSpec::new("toolExposurePolicies", "Exposure per tool", list.clone())
        },
        SettingSpec {
            advanced: true,
            summary: "Each TOOL=NUMBER".into(),
            ..SettingSpec::new("toolMaximumConcurrentCalls", "Calls at once per tool", list)
        },
        SettingSpec {
            advanced: true,
            ..SettingSpec::new(
                "maximumConcurrentCalls",
                "Calls at once",
                SettingKind::Integer {
                    minimum: Some(1),
                    maximum: Some(MAX_MCP_CONCURRENT_CALLS as i64),
                },
            )
        },
        timeout("connectTimeoutSeconds", "Connect timeout"),
        timeout("initializeTimeoutSeconds", "Initialize timeout"),
        timeout("listTimeoutSeconds", "List timeout"),
        timeout("callTimeoutSeconds", "Call timeout"),
        timeout("shutdownTimeoutSeconds", "Shutdown timeout"),
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "MCP servers", SettingsGroup::Tools, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a server").named("Server id"),
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record(DELETE, "Delete").confirmed(),
            SettingsAction::record(CONNECT, "Connect"),
            SettingsAction::record(DISCONNECT, "Disconnect"),
            SettingsAction::record(REFRESH, "Refresh"),
            SettingsAction::record(LOGOUT, "Log out"),
        ]);
    let snapshot = match service.mcp_snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => return collection.unavailable(error),
    };
    let fields = fields();
    let mut template = with_both_transports(
        new_server("", &snapshot.server_defaults),
        &snapshot.server_defaults,
    );
    maps_as_lists(&mut template, &MAPS);
    let records = snapshot
        .settings
        .servers
        .iter()
        .map(|server| {
            let id = server.id.as_str();
            let mut record = record_of(id, id, &with_lists(server), &fields);
            let status = snapshot
                .servers
                .iter()
                .find(|status| status.server_id == server.id);
            record.summary = match status {
                Some(status)
                    if matches!(
                        status.phase,
                        McpConnectionPhase::Ready | McpConnectionPhase::Refreshing
                    ) =>
                {
                    format!("{} · {} tools", target(server), status.tool_count)
                }
                _ => target(server),
            };
            record.state = status.map(|status| name_of(&status.phase));
            record.issue = status
                .filter(|status| status.phase == McpConnectionPhase::Failed)
                .and_then(|status| status.last_error.clone());
            record
        })
        .collect();
    SettingsCollection {
        record: with_defaults(fields, &template),
        records,
        revision: Some(snapshot.settings.revision),
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let id = McpServerId::parse(request.record_id()?).map_err(|error| error.to_string())?;
    match request.action.as_str() {
        ADD | SAVE => {
            let snapshot = service.mcp_snapshot().await?;
            let current = snapshot
                .settings
                .servers
                .iter()
                .find(|server| server.id == id);
            let record = match (request.action.as_str(), current) {
                (ADD, None) => new_server(id.as_str(), &snapshot.server_defaults),
                (ADD, Some(_)) => return Err(format!("the MCP server `{}` exists", id.as_str())),
                (_, Some(server)) => serde_json::to_value(server)
                    .map_err(|_| "the MCP server cannot be encoded".to_string())?,
                (_, None) => return Err(format!("there is no MCP server `{}`", id.as_str())),
            };
            let mut record = with_both_transports(record, &snapshot.server_defaults);
            maps_as_lists(&mut record, &MAPS);
            let mut record: Value = merged(&record, &fields(), &request.values)?;
            lists_as_maps(&mut record, &MAPS)?;
            let server: McpServerConfig = serde_json::from_value(record)
                .map_err(|error| format!("the record cannot be written: {error}"))?;
            let before = current.map(credential_names).unwrap_or_default();
            let written = service
                .mcp_write_server(McpServerWriteRequest {
                    expected_revision: request.revision()?,
                    server,
                })
                .await?;
            drop_unreferenced(service, &written, before.into_values()).await;
            Ok(SettingsActionOutcome {
                record: (request.action == ADD).then(|| id.as_str().to_string()),
                message: None,
            })
        }
        DELETE => {
            let snapshot = service.mcp_snapshot().await?;
            let before = snapshot
                .settings
                .servers
                .iter()
                .find(|server| server.id == id)
                .map(credential_names)
                .unwrap_or_default();
            let written = service
                .mcp_delete_server(McpServerDeleteRequest {
                    expected_revision: request.revision()?,
                    server_id: id,
                })
                .await?;
            drop_unreferenced(service, &written, before.into_values()).await;
            Ok(SettingsActionOutcome::default())
        }
        CONNECT => service.mcp_connect(id).await.map(|_| Default::default()),
        DISCONNECT => service.mcp_disconnect(id).await.map(|_| Default::default()),
        REFRESH => service.mcp_refresh(id).await.map(|_| Default::default()),
        LOGOUT => service
            .mcp_oauth_logout(id)
            .await
            .map(|_| Default::default()),
        _ => request.unknown_action(),
    }
}

/// `server` serialized, its maps as lists of `NAME=VALUE` texts.
fn with_lists(server: &McpServerConfig) -> Value {
    let mut record = serde_json::to_value(server).unwrap_or(Value::Null);
    maps_as_lists(&mut record, &MAPS);
    record
}

/// What the server runs or reaches.
fn target(server: &McpServerConfig) -> String {
    match &server.transport {
        McpTransportConfig::Stdio(stdio) => stdio.program.clone(),
        McpTransportConfig::StreamableHttp(http) => http.url.clone(),
    }
}

/// A server the user adds, as `mcp add` starts one: the defaults, a stdio
/// transport with no program yet, no per-tool settings.
fn new_server(id: &str, defaults: &hanafubuki_agent_mcp::McpServerDefaults) -> Value {
    let mut server = match serde_json::to_value(defaults) {
        Ok(Value::Object(server)) => server,
        _ => Map::new(),
    };
    server.remove("oauth");
    server.extend([
        ("id".to_string(), json!(id)),
        ("transport".to_string(), json!({ "type": STDIO })),
        ("toolMaximumConcurrentCalls".to_string(), json!({})),
        ("enabledTools".to_string(), json!([])),
        ("disabledTools".to_string(), json!([])),
        ("toolApprovalPolicies".to_string(), json!({})),
        ("toolExposurePolicies".to_string(), json!({})),
    ]);
    Value::Object(server)
}

/// `server` whose transport holds every field of both transports, those of
/// the other one empty: a save that changes `transport.type` then reads,
/// and the transport keeps only its own.
fn with_both_transports(
    mut server: Value,
    defaults: &hanafubuki_agent_mcp::McpServerDefaults,
) -> Value {
    let empty = json!({
        "program": "",
        "argv": [],
        "cwd": null,
        "publicEnvironment": {},
        "credentialEnvironment": {},
        "url": "",
        "publicHeaders": {},
        "credentialHeaders": {},
        "oauth": name_of(&defaults.oauth),
    });
    if let (Some(Value::Object(transport)), Value::Object(empty)) =
        (server.get_mut("transport"), empty)
    {
        for (key, value) in empty {
            transport.entry(key).or_insert(value);
        }
    }
    server
}
