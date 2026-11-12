//! The secret environment variables and headers of the MCP servers: a
//! record per name a server's transport fills from a credential
//! (`SERVER/NAME`), its value written with `mcp_set_credential` and never
//! returned. Adding or deleting one writes the server's reference first,
//! at the revision read; a credential no server references any more is
//! removed after it.

use std::collections::BTreeMap;

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingsGroup};
use hanafubuki_agent_mcp::{McpCredentialKey, McpServerConfig, McpServerId, McpTransportConfig};
use serde_json::Value;

use super::{
    SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection,
    SettingsRecord, ADD, DELETE, SAVE,
};
use crate::service::AgentApplicationService;
use crate::{McpApplicationSnapshot, McpServerWriteRequest};

pub(super) const ID: &str = "mcpSecrets";
const VALUE: &str = "value";

fn fields() -> Vec<SettingSpec> {
    vec![SettingSpec {
        summary: "Written only; it is never shown".into(),
        ..SettingSpec::new(
            VALUE,
            "Value",
            SettingKind::Text {
                maximum_chars: None,
                secret: true,
                nullable: false,
            },
        )
    }]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection = SettingsCollection::new(ID, "MCP secrets", SettingsGroup::Tools, fields())
        .with_actions(vec![
            SettingsAction::collection(ADD, "Add a secret").named("Server and name (SERVER/NAME)"),
            SettingsAction::record(SAVE, "Save"),
            SettingsAction::record(DELETE, "Delete").confirmed(),
        ]);
    let snapshot = match service.mcp_snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => return collection.unavailable(error),
    };
    let records = snapshot
        .settings
        .servers
        .iter()
        .flat_map(|server| {
            let kind = match server.transport {
                McpTransportConfig::Stdio(_) => "environment variable",
                McpTransportConfig::StreamableHttp(_) => "header",
            };
            let credentials = &snapshot.settings.credentials;
            credential_names(server)
                .into_iter()
                .map(move |(name, key)| SettingsRecord {
                    id: format!("{}/{name}", server.id.as_str()),
                    label: format!("{} · {name}", server.id.as_str()),
                    summary: format!("{kind} · credential {}", key.as_str()),
                    state: None,
                    issue: (!credentials.contains_key(&key)).then(|| "it has no value".into()),
                    read_only: None,
                    values: BTreeMap::new(),
                    secrets_set: credentials
                        .contains_key(&key)
                        .then(|| VALUE.to_string())
                        .into_iter()
                        .collect(),
                })
        })
        .collect();
    SettingsCollection {
        records,
        revision: Some(snapshot.settings.revision),
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    let (server_id, name) = parse(request.record_id()?)?;
    let snapshot = service.mcp_snapshot().await?;
    let server = snapshot
        .settings
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .ok_or_else(|| format!("there is no MCP server `{}`", server_id.as_str()))?;
    let referenced = credential_names(server).get(&name).cloned();
    match (request.action.as_str(), referenced) {
        (ADD, Some(_)) => Err(format!(
            "`{name}` of `{}` is a secret already",
            server_id.as_str()
        )),
        (ADD, None) => {
            let value = value(&request)?;
            if public_names(server).contains(&name) {
                return Err(format!(
                    "`{name}` of `{}` is public: remove it from the server first",
                    server_id.as_str()
                ));
            }
            let key = McpCredentialKey::for_server_secret(&server_id, &name);
            let mut server = server.clone();
            credential_map(&mut server).insert(name, key.clone());
            service
                .mcp_write_server(McpServerWriteRequest {
                    expected_revision: request.revision()?,
                    server,
                })
                .await?;
            service.mcp_set_credential(key, value).await?;
            Ok(SettingsActionOutcome {
                record: request.record.clone(),
                message: None,
            })
        }
        (SAVE, Some(key)) => {
            service.mcp_set_credential(key, value(&request)?).await?;
            Ok(SettingsActionOutcome::default())
        }
        (DELETE, Some(key)) => {
            let mut server = server.clone();
            credential_map(&mut server).remove(&name);
            let written = service
                .mcp_write_server(McpServerWriteRequest {
                    expected_revision: request.revision()?,
                    server,
                })
                .await?;
            drop_unreferenced(service, &written, [key]).await;
            Ok(SettingsActionOutcome::default())
        }
        (SAVE | DELETE, None) => Err(format!(
            "`{name}` is not a secret of `{}`",
            server_id.as_str()
        )),
        _ => request.unknown_action(),
    }
}

/// The names `server`'s transport fills from credentials, with their keys.
pub(super) fn credential_names(server: &McpServerConfig) -> BTreeMap<String, McpCredentialKey> {
    match &server.transport {
        McpTransportConfig::Stdio(stdio) => stdio.credential_environment.clone(),
        McpTransportConfig::StreamableHttp(http) => http.credential_headers.clone(),
    }
}

/// Removes each of `keys` that no server of `snapshot` references; one
/// left behind harms nothing, so a failure is not the caller's.
pub(super) async fn drop_unreferenced(
    service: &AgentApplicationService,
    snapshot: &McpApplicationSnapshot,
    keys: impl IntoIterator<Item = McpCredentialKey>,
) {
    for key in keys {
        let referenced = snapshot
            .settings
            .servers
            .iter()
            .any(|server| credential_names(server).values().any(|used| *used == key));
        if !referenced && snapshot.settings.credentials.contains_key(&key) {
            let _ = service.mcp_remove_credential(key).await;
        }
    }
}

fn public_names(server: &McpServerConfig) -> Vec<String> {
    match &server.transport {
        McpTransportConfig::Stdio(stdio) => stdio.public_environment.keys().cloned().collect(),
        McpTransportConfig::StreamableHttp(http) => http.public_headers.keys().cloned().collect(),
    }
}

fn credential_map(server: &mut McpServerConfig) -> &mut BTreeMap<String, McpCredentialKey> {
    match &mut server.transport {
        McpTransportConfig::Stdio(stdio) => &mut stdio.credential_environment,
        McpTransportConfig::StreamableHttp(http) => &mut http.credential_headers,
    }
}

/// `SERVER/NAME`.
fn parse(record: &str) -> Result<(McpServerId, String), String> {
    let (server, name) = record
        .split_once('/')
        .filter(|(_, name)| !name.trim().is_empty())
        .ok_or_else(|| format!("`{record}` is not SERVER/NAME"))?;
    let server = McpServerId::parse(server).map_err(|error| error.to_string())?;
    Ok((server, name.trim().to_string()))
}

/// The value `add` and `save` write, checked by its spec.
fn value(request: &SettingsActionRequest) -> Result<String, String> {
    let raw = request
        .values
        .get(VALUE)
        .ok_or_else(|| "enter the secret's value".to_string())?;
    let spec = &fields()[0];
    match spec.check(raw)?.to_json() {
        Value::String(value) if !value.is_empty() => Ok(value),
        _ => Err("enter the secret's value".to_string()),
    }
}
