use super::{credential_presence, credential_references, digest_hex, safe_mcp_transport_summary};
use crate::service::AgentApplicationService;
use crate::{McpLocalPolicySummary, McpLocalServerStatus, McpLocalStatusSnapshot};
use std::collections::{BTreeSet, HashMap};

impl AgentApplicationService {
    pub async fn mcp_local_status(&self, verbose: bool) -> Result<McpLocalStatusSnapshot, String> {
        let snapshot = self.mcp_application.mcp_snapshot().await?;
        let runtime_names = snapshot
            .tools
            .iter()
            .map(|tool| tool.model_name.clone())
            .collect::<BTreeSet<_>>();
        let referenced_credentials = snapshot
            .settings
            .servers
            .iter()
            .flat_map(credential_references)
            .collect::<BTreeSet<_>>();
        let hidden_by_server = self
            .mcp
            .tool_leases()
            .into_iter()
            .filter(|lease| {
                lease.binding.exposure == hanafubuki_agent_mcp::McpExposurePolicy::Hidden
            })
            .fold(
                HashMap::<hanafubuki_agent_mcp::McpServerId, usize>::new(),
                |mut counts, lease| {
                    *counts.entry(lease.binding.server_id).or_default() += 1;
                    counts
                },
            );
        let mut connected_count = 0;
        let mut connecting_count = 0;
        let mut error_count = 0;
        let mut disabled_count = 0;
        let mut servers = Vec::with_capacity(snapshot.settings.servers.len());
        for config in &snapshot.settings.servers {
            let status = snapshot
                .servers
                .iter()
                .find(|status| status.server_id == config.id)
                .cloned()
                .unwrap_or_else(|| {
                    hanafubuki_agent_mcp::McpServerStatus::disabled(config.id.clone())
                });
            match status.phase {
                hanafubuki_agent_mcp::McpConnectionPhase::Ready => connected_count += 1,
                hanafubuki_agent_mcp::McpConnectionPhase::Starting
                | hanafubuki_agent_mcp::McpConnectionPhase::Refreshing
                | hanafubuki_agent_mcp::McpConnectionPhase::Stopping => connecting_count += 1,
                hanafubuki_agent_mcp::McpConnectionPhase::Failed => error_count += 1,
                hanafubuki_agent_mcp::McpConnectionPhase::Disabled => disabled_count += 1,
            }
            let tools = snapshot
                .tools
                .iter()
                .filter(|tool| tool.server_id == config.id)
                .cloned()
                .collect::<Vec<_>>();
            let runtime_tool_count = tools.len();
            let credential_presence = credential_references(config)
                .into_iter()
                .map(|key| credential_presence(&snapshot.settings, key))
                .collect::<Vec<_>>();
            let oauth = snapshot
                .oauth
                .iter()
                .find(|oauth| oauth.server_id == config.id)
                .cloned()
                .unwrap_or_else(|| self.mcp_application.mcp_oauth.status(config));
            let diagnostics = self.mcp.diagnostics(&config.id);
            let diagnostic_summary = self.mcp.diagnostic_summary(&config.id);
            let settings_generation = digest_hex(
                hanafubuki_agent_tool_contracts::canonical_digest(config).unwrap_or([0; 32]),
            );
            let (transport, configured_target) = safe_mcp_transport_summary(config);
            servers.push(McpLocalServerStatus {
                server_id: config.id.clone(),
                transport,
                configured_target,
                phase: status.phase,
                enabled: config.enabled,
                required: config.required,
                settings_generation,
                connection_generation: status
                    .connection_generation
                    .map(|generation| digest_hex(generation.0)),
                catalog_generation: status
                    .catalog_generation
                    .map(|generation| digest_hex(generation.0)),
                runtime_tool_count,
                hidden_tool_count: hidden_by_server.get(&config.id).copied().unwrap_or(0),
                credential_presence,
                oauth,
                cleanup_pending: status.cleanup_pending,
                last_error: status.last_error,
                reconnect: status.reconnect,
                diagnostic_summary,
                resolved_program: verbose.then_some(status.resolved_program).flatten(),
                capabilities: verbose.then_some(status.capabilities).flatten(),
                policy: verbose.then(|| McpLocalPolicySummary {
                    connect_timeout_seconds: config.connect_timeout_seconds,
                    initialize_timeout_seconds: config.initialize_timeout_seconds,
                    list_timeout_seconds: config.list_timeout_seconds,
                    call_timeout_seconds: config.call_timeout_seconds,
                    shutdown_timeout_seconds: config.shutdown_timeout_seconds,
                    maximum_concurrent_calls: config.maximum_concurrent_calls,
                    tool_maximum_concurrent_calls: config.tool_maximum_concurrent_calls.clone(),
                    approval_policy: config.approval_policy,
                    tool_approval_policies: config.tool_approval_policies.clone(),
                    exposure_policy: config.exposure_policy,
                    tool_exposure_policies: config.tool_exposure_policies.clone(),
                }),
                tools: if verbose { tools } else { Vec::new() },
                diagnostics: verbose.then_some(diagnostics),
            });
        }
        Ok(McpLocalStatusSnapshot {
            verbose,
            settings_revision: snapshot.settings.revision,
            server_count: servers.len(),
            connected_count,
            connecting_count,
            error_count,
            disabled_count,
            runtime_nonhidden_tool_count: runtime_names.len(),
            hidden_tool_count: snapshot.hidden_tool_count,
            credential_key_count: referenced_credentials.len(),
            credential_present_count: referenced_credentials
                .iter()
                .filter(|key| snapshot.settings.credentials.contains_key(key))
                .count(),
            diagnostics_volatile_until_restart: true,
            servers,
        })
    }
}
