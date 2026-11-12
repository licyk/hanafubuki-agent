//! The MCP servers of the installed plugins that are on: a record per
//! plugin and server (`PLUGIN/SERVER`), its connection's state, and how the
//! user restricts it (`[plugins."<plugin>".mcpServers.<server>]`: on or
//! off, only these tools, not these tools). How it runs stays the
//! plugin's. A save writes only the keys it names, `null` returning one to
//! the plugin's, then loads the plugins again.

use hanafubuki_agent_config::{SettingKind, SettingSpec, SettingsGroup, UserEntry};
use hanafubuki_agent_mcp::{McpConnectionPhase, McpServerConfig};
use hanafubuki_agent_packages::PluginId;
use serde_json::{Map, Value};

use super::record::{merged, record_of};
use super::{
    name_of, SettingsAction, SettingsActionOutcome, SettingsActionRequest, SettingsCollection, SAVE,
};
use crate::service::AgentApplicationService;

pub(super) const ID: &str = "pluginMcp";

fn fields() -> Vec<SettingSpec> {
    let list = SettingKind::TextList {
        maximum_items: None,
        maximum_item_chars: None,
    };
    vec![
        SettingSpec::new("enabled", "Enabled", SettingKind::Boolean),
        SettingSpec::new("enabledTools", "Only these tools", list.clone()),
        SettingSpec::new("disabledTools", "Not these tools", list),
    ]
}

pub(super) async fn collection(service: &AgentApplicationService) -> SettingsCollection {
    let collection =
        SettingsCollection::new(ID, "Plugins' MCP servers", SettingsGroup::Tools, fields())
            .with_actions(vec![SettingsAction::record(SAVE, "Save")]);
    let snapshot = match service.mcp_snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => return collection.unavailable(error),
    };
    let fields = fields();
    let records = snapshot
        .plugin_servers
        .iter()
        .map(|plugin| {
            let server = plugin.server.id.as_str();
            let id = format!("{}/{server}", plugin.plugin);
            let mut record = record_of(&id, server, &plugin.server, &fields);
            let status = &plugin.status;
            record.summary = match status.phase {
                McpConnectionPhase::Ready | McpConnectionPhase::Refreshing => {
                    format!("plugin {} · {} tools", plugin.plugin, status.tool_count)
                }
                _ => format!("plugin {}", plugin.plugin),
            };
            record.state = Some(name_of(&status.phase));
            record.issue = (status.phase == McpConnectionPhase::Failed)
                .then(|| status.last_error.clone())
                .flatten();
            record
        })
        .collect();
    SettingsCollection {
        records,
        ..collection
    }
}

pub(super) async fn run(
    service: &AgentApplicationService,
    request: SettingsActionRequest,
) -> Result<SettingsActionOutcome, String> {
    if request.action != SAVE {
        return request.unknown_action();
    }
    let record = request.record_id()?;
    let (plugin, server) = record
        .split_once('/')
        .ok_or_else(|| format!("`{record}` is not PLUGIN/SERVER"))?;
    let plugin = PluginId::parse(plugin).map_err(|error| error.to_string())?;
    let snapshot = service.mcp_snapshot().await?;
    let current = snapshot
        .plugin_servers
        .iter()
        .find(|candidate| {
            candidate.plugin == plugin.key() && candidate.server.id.as_str() == server
        })
        .ok_or_else(|| format!("the plugin {} runs no MCP server `{server}`", plugin.key()))?;
    check(&current.server, &request.values)?;
    let (config, values, server) = (
        service.config.clone(),
        request.values.clone(),
        server.to_string(),
    );
    hanafubuki_agent_foundation::blocking::run_blocking_operation(
        service.blocking.as_ref(),
        "agent plugin MCP server write worker",
        move || {
            config.write_entry_values(
                UserEntry::PluginMcpServer {
                    plugin: &plugin,
                    server: &server,
                },
                &values,
            )
        },
    )
    .await??;
    service.reload_packages().await?;
    Ok(SettingsActionOutcome::default())
}

/// Refuses values the server could not run with: each checked by its
/// field's spec, then the server with them (a key set to `null` keeps its
/// value for this check) as the plugin's loading checks it.
fn check(server: &McpServerConfig, values: &Map<String, Value>) -> Result<(), String> {
    let set = values
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    let fields = fields();
    for key in values.keys() {
        if !fields.iter().any(|spec| spec.key == *key) {
            return Err(format!("`{key}` is not a field of this record"));
        }
    }
    let server: McpServerConfig = merged(server, &fields, &set)?;
    server
        .validate()
        .map_err(|error| format!("the server cannot run with this: {error}"))
}
