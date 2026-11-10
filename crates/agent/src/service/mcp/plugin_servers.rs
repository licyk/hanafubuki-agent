//! The installed plugins' MCP servers: owned by their packages, never in
//! `mcp.toml`, and run under the same supervision as the user's. The
//! package loading decides which run (the user's server of a name comes
//! first, then the first plugin in key order); a change to that set
//! connects what was added or changed and disconnects what was removed.

use std::collections::BTreeMap;

use hanafubuki_agent_mcp::{McpServerConfig, McpServerId, McpSettings};
use serde::Serialize;

use super::AgentMcpApplication;

/// A server an installed plugin runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPluginServer {
    /// The plugin, `<plugin>@<marketplace>`.
    pub plugin: String,
    pub server: McpServerConfig,
}

/// A plugin's server as the MCP manager shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPluginServerSnapshot {
    pub plugin: String,
    pub server: McpServerConfig,
    pub status: hanafubuki_agent_mcp::McpServerStatus,
}

impl AgentMcpApplication {
    /// The plugins' servers that run now.
    pub(crate) fn plugin_servers(&self) -> Vec<McpPluginServer> {
        self.plugin_servers.read().clone()
    }

    /// The server `id`: the user's, else a plugin's.
    pub(super) fn server_config(
        &self,
        settings: &McpSettings,
        id: &McpServerId,
    ) -> Option<McpServerConfig> {
        settings.server(id).cloned().or_else(|| {
            self.plugin_servers
                .read()
                .iter()
                .find(|plugin| &plugin.server.id == id)
                .map(|plugin| plugin.server.clone())
        })
    }

    /// Runs `servers` as the plugins' servers from now on: a server that is
    /// gone or changed is disconnected (unless the user's server of its
    /// name took it over), and one that is new or changed connects, all
    /// before this returns. A connect that fails is that server's status.
    pub(crate) async fn set_plugin_servers(&self, servers: Vec<McpPluginServer>) {
        if servers.is_empty() && self.plugin_servers.read().is_empty() {
            return;
        }
        let _gate = self.mcp_configuration_gate.lock().await;
        let settings = match self.load_settings().await {
            Ok(settings) => settings,
            Err(error) => {
                log::warn!("the plugins' MCP servers were not applied: {error}");
                return;
            }
        };
        let previous = std::mem::replace(&mut *self.plugin_servers.write(), servers.clone())
            .into_iter()
            .map(|plugin| (plugin.server.id.clone(), plugin.server))
            .collect::<BTreeMap<_, _>>();
        let current = servers
            .into_iter()
            .map(|plugin| (plugin.server.id.clone(), plugin.server))
            .collect::<BTreeMap<_, _>>();
        for (id, server) in &previous {
            let unchanged = current.get(id) == Some(server) && server.enabled;
            if !unchanged && settings.server(id).is_none() {
                if let Err(error) = self.mcp.disconnect(id).await {
                    log::warn!(
                        "plugin MCP server {} did not disconnect: {error}",
                        id.as_str()
                    );
                }
            }
        }
        let connects = current
            .into_iter()
            .filter(|(id, server)| server.enabled && previous.get(id) != Some(server))
            .map(|(_, server)| server)
            .collect();
        for (server, outcome) in self
            .connect_concurrently(connects, std::sync::Arc::new(settings))
            .await
        {
            if let Err(error) = outcome {
                log::warn!(
                    "plugin MCP server {} did not start: {error}",
                    server.id.as_str()
                );
                self.mcp.report_connection_failure(server.id, error);
            }
        }
    }

    /// The plugins' servers with their status.
    pub(super) fn plugin_server_snapshots(&self) -> Vec<McpPluginServerSnapshot> {
        let mut statuses = self
            .mcp
            .statuses()
            .into_iter()
            .map(|status| (status.server_id.clone(), status))
            .collect::<std::collections::HashMap<_, _>>();
        self.plugin_servers()
            .into_iter()
            .map(|plugin| McpPluginServerSnapshot {
                status: statuses.remove(&plugin.server.id).unwrap_or_else(|| {
                    hanafubuki_agent_mcp::McpServerStatus::disabled(plugin.server.id.clone())
                }),
                plugin: plugin.plugin,
                server: plugin.server,
            })
            .collect()
    }
}
