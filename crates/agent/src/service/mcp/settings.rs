use super::{credential_references, require_revision, write_credential, AgentMcpApplication};
use crate::{
    McpApplicationSnapshot, McpCredentialsWriteRequest, McpServerDeleteRequest,
    McpServerWriteRequest,
};
use std::collections::BTreeSet;

impl AgentMcpApplication {
    pub async fn mcp_write_server(
        &self,
        request: McpServerWriteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        let _gate = self.mcp_configuration_gate.lock().await;
        let current = self.load_settings().await?;
        require_revision(&current, request.expected_revision)?;
        let server = request.server.clone();
        let settings = self
            .update_settings(request.expected_revision, move |settings| {
                if let Some(existing) = settings
                    .servers
                    .iter_mut()
                    .find(|existing| existing.id == server.id)
                {
                    *existing = server.clone();
                } else {
                    settings.servers.push(server.clone());
                }
                settings
                    .servers
                    .sort_by(|left, right| left.id.cmp(&right.id));
                Ok(())
            })
            .await?;
        if let Err(error) = self.reconcile_mcp_server(&request.server, &settings).await {
            self.mcp
                .report_connection_failure(request.server.id.clone(), error);
        }
        self.mcp_snapshot_with_settings(settings).await
    }

    pub async fn mcp_delete_server(
        &self,
        request: McpServerDeleteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        let _gate = self.mcp_configuration_gate.lock().await;
        let current = self.load_settings().await?;
        require_revision(&current, request.expected_revision)?;
        current
            .server(&request.server_id)
            .ok_or_else(|| "MCP server does not exist".to_string())?;
        self.mcp
            .disconnect(&request.server_id)
            .await
            .map_err(|error| error.to_string())?;
        let server_id = request.server_id.clone();
        let settings = self
            .update_settings(request.expected_revision, move |settings| {
                let before = settings.servers.len();
                settings.servers.retain(|server| server.id != server_id);
                if settings.servers.len() == before {
                    return Err(hanafubuki_agent_mcp::McpError::InvalidConfig(
                        "MCP server does not exist".into(),
                    ));
                }
                Ok(())
            })
            .await?;
        self.mcp_snapshot_with_settings(settings).await
    }

    pub async fn mcp_write_credentials(
        &self,
        request: McpCredentialsWriteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        let _gate = self.mcp_configuration_gate.lock().await;
        let current = self.load_settings().await?;
        require_revision(&current, request.expected_revision)?;
        for value in request.credentials.values() {
            hanafubuki_agent_mcp::validate_credential_value(value)
                .map_err(|error| error.to_string())?;
        }
        // Removing a credential an enabled server still depends on would break
        // it on its next connection, so refuse rather than let the write land.
        let mut broken = current
            .servers
            .iter()
            .filter(|server| server.enabled)
            .filter(|server| {
                credential_references(server)
                    .iter()
                    .any(|key| !request.credentials.contains_key(key))
            })
            .map(|server| server.id.as_str().to_string())
            .collect::<Vec<_>>();
        broken.sort();
        if !broken.is_empty() {
            return Err(format!(
                "credential is referenced by enabled MCP servers: {}; disable them before deletion",
                broken.join(", ")
            ));
        }
        let changed = request
            .credentials
            .iter()
            .filter(|(key, value)| {
                current
                    .credentials
                    .get(*key)
                    .is_none_or(|existing| existing.value != **value)
            })
            .map(|(key, _)| key.clone())
            .chain(
                current
                    .credentials
                    .keys()
                    .filter(|key| !request.credentials.contains_key(*key))
                    .cloned(),
            )
            .collect::<BTreeSet<_>>();
        let desired = request.credentials;
        let settings = self
            .update_settings(request.expected_revision, move |settings| {
                let keys = settings
                    .credentials
                    .keys()
                    .cloned()
                    .chain(desired.keys().cloned())
                    .collect::<BTreeSet<_>>();
                for key in keys {
                    let value = desired.get(&key).cloned();
                    let unchanged = match (&value, settings.credentials.get(&key)) {
                        (Some(value), Some(existing)) => existing.value == *value,
                        (None, None) => true,
                        _ => false,
                    };
                    if unchanged {
                        continue;
                    }
                    write_credential(settings, key, value)?;
                }
                Ok(())
            })
            .await?;
        for key in changed {
            self.reconnect_credential_consumers(&key, &settings).await;
        }
        self.mcp_snapshot_with_settings(settings).await
    }
}
