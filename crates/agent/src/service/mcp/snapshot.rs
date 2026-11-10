use super::{
    bounded_tool_description, dependency_transport_matches, digest_hex, AgentMcpApplication,
};
use crate::{
    McpApplicationSnapshot, McpToolSnapshot, SkillAvailabilityState, SkillMcpDependencyStatus,
    SkillMcpTransport, SkillToolDependency,
};
use std::collections::HashMap;

impl AgentMcpApplication {
    pub(crate) fn skill_dependency_statuses(
        &self,
        dependencies: &[SkillToolDependency],
    ) -> Result<Vec<SkillMcpDependencyStatus>, String> {
        let settings = self
            .mcp_settings
            .load()
            .map_err(|error| error.to_string())?;
        let statuses = self
            .mcp
            .statuses()
            .into_iter()
            .map(|status| (status.server_id.clone(), status))
            .collect::<HashMap<_, _>>();
        dependencies
            .iter()
            .map(|dependency| {
                let server = hanafubuki_agent_mcp::McpServerId::parse(&dependency.server_id)
                    .ok()
                    .and_then(|id| self.server_config(&settings, &id));
                let Some(server) = server.as_ref() else {
                    return Ok(SkillMcpDependencyStatus {
                        dependency: dependency.clone(),
                        state: SkillAvailabilityState::Missing,
                        reason: Some("required MCP server is not configured".to_string()),
                        server_generation: None,
                        connection_generation: None,
                        action: match dependency.transport {
                            Some(SkillMcpTransport::StreamableHttp) => "register_http",
                            _ => "install_and_register_explicit_source",
                        }
                        .to_string(),
                    });
                };
                let server_generation = Some(digest_hex(
                    hanafubuki_agent_tool_contracts::canonical_digest(server).unwrap_or([0; 32]),
                ));
                let status = statuses.get(&server.id).cloned().unwrap_or_else(|| {
                    hanafubuki_agent_mcp::McpServerStatus::disabled(server.id.clone())
                });
                let connection_generation = status
                    .connection_generation
                    .map(|generation| digest_hex(generation.0));
                if !dependency_transport_matches(dependency, server) {
                    return Ok(SkillMcpDependencyStatus {
                        dependency: dependency.clone(),
                        state: SkillAvailabilityState::IncompatibleTransport,
                        reason: Some(
                            "configured MCP transport or canonical target differs".to_string(),
                        ),
                        server_generation,
                        connection_generation,
                        action: "update_or_replace_configuration".to_string(),
                    });
                }
                if !server.enabled {
                    return Ok(SkillMcpDependencyStatus {
                        dependency: dependency.clone(),
                        state: SkillAvailabilityState::InstalledDisabled,
                        reason: Some("required MCP server is configured but disabled".to_string()),
                        server_generation,
                        connection_generation,
                        action: "enable_and_connect".to_string(),
                    });
                }
                let oauth = self.mcp_oauth.status(server);
                if matches!(
                    &server.transport,
                    hanafubuki_agent_mcp::McpTransportConfig::StreamableHttp(http)
                        if http.oauth == hanafubuki_agent_mcp::McpOAuthPolicy::Required
                ) && oauth.phase != hanafubuki_agent_mcp::McpOAuthPhase::Authorized
                {
                    return Ok(SkillMcpDependencyStatus {
                        dependency: dependency.clone(),
                        state: SkillAvailabilityState::NeedsAuthentication,
                        reason: Some("required MCP OAuth login is incomplete".to_string()),
                        server_generation,
                        connection_generation,
                        action: "start_oauth".to_string(),
                    });
                }
                if status.phase == hanafubuki_agent_mcp::McpConnectionPhase::Ready {
                    Ok(SkillMcpDependencyStatus {
                        dependency: dependency.clone(),
                        state: SkillAvailabilityState::Ready,
                        reason: None,
                        server_generation,
                        connection_generation,
                        action: "none".to_string(),
                    })
                } else {
                    Ok(SkillMcpDependencyStatus {
                        dependency: dependency.clone(),
                        state: SkillAvailabilityState::InvalidConfiguration,
                        reason: status.last_error.or_else(|| {
                            Some("required MCP server is enabled but not ready".to_string())
                        }),
                        server_generation,
                        connection_generation,
                        action: "connect_or_inspect_diagnostics".to_string(),
                    })
                }
            })
            .collect()
    }

    pub async fn mcp_snapshot(&self) -> Result<McpApplicationSnapshot, String> {
        let settings = self.load_settings().await?;
        self.mcp_snapshot_with_settings(settings).await
    }

    pub(super) async fn mcp_snapshot_with_settings(
        &self,
        settings: hanafubuki_agent_mcp::McpSettings,
    ) -> Result<McpApplicationSnapshot, String> {
        let mut statuses = self
            .mcp
            .statuses()
            .into_iter()
            .map(|status| (status.server_id.clone(), status))
            .collect::<HashMap<_, _>>();
        let servers = settings
            .servers
            .iter()
            .map(|server| {
                statuses.remove(&server.id).unwrap_or_else(|| {
                    hanafubuki_agent_mcp::McpServerStatus::disabled(server.id.clone())
                })
            })
            .collect();
        let oauth = settings
            .servers
            .iter()
            .map(|server| self.mcp_oauth.status(server))
            .collect();
        let leases = self.mcp.tool_leases();
        let hidden_tool_count = leases
            .iter()
            .filter(|lease| {
                lease.binding.exposure == hanafubuki_agent_mcp::McpExposurePolicy::Hidden
                    || !lease.binding.app_visible
            })
            .count();
        let tools = leases
            .into_iter()
            .filter(|lease| {
                lease.binding.exposure != hanafubuki_agent_mcp::McpExposurePolicy::Hidden
                    && lease.binding.app_visible
            })
            .take(4096)
            .map(|lease| McpToolSnapshot {
                server_id: lease.binding.server_id,
                remote_name: lease.binding.remote_name,
                model_name: lease.binding.specification.name.qualified(),
                title: lease.binding.title,
                description: bounded_tool_description(lease.binding.specification.description),
                model_visible: lease.binding.model_visible,
                app_visible: lease.binding.app_visible,
                hints: lease.binding.hints,
                exposure: lease.binding.exposure,
                approval: lease.approval_policy,
                connection_generation: digest_hex(lease.binding.connection_generation.0),
                catalog_generation: digest_hex(lease.binding.catalog_generation.0),
                tool_generation: digest_hex(lease.binding.tool_generation.0),
                schema_digest: digest_hex(
                    hanafubuki_agent_tool_contracts::canonical_digest(
                        &lease.binding.specification.input_schema,
                    )
                    .unwrap_or([0; 32]),
                ),
                input_schema: lease.binding.specification.input_schema,
                output_schema: lease.binding.output_schema,
            })
            .collect();
        let diagnostics = settings
            .servers
            .iter()
            .map(|server| self.mcp.diagnostic_summary(&server.id))
            .collect();
        Ok(McpApplicationSnapshot {
            plugin_servers: self.plugin_server_snapshots(),
            settings,
            servers,
            oauth,
            tools,
            hidden_tool_count,
            pending_elicitations: self.mcp.pending_elicitations(),
            diagnostics,
            settings_repairs: self.mcp_settings.repairs(),
            server_defaults: hanafubuki_agent_mcp::McpServerDefaults::default(),
        })
    }
}
