use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    validate_credential_value, McpCredentialKey, McpCredentialSettings, McpError, McpServerConfig,
    McpServerId, MAX_MCP_SERVERS,
};

mod repair;

pub use repair::{repair_settings, McpSettingsRepair, McpSettingsRepairKind};

/// Durable MCP configuration, which the Agent's configuration persists
/// (`hanafubuki-agent-config`).
///
/// Credential values are stored verbatim alongside the servers that reference
/// them, so a configured credential can be read back and edited. The
/// per-credential generation is retained purely as change detection for
/// connection invalidation; it never stands in for the value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    pub revision: u64,
    pub servers: Vec<McpServerConfig>,
    pub credentials: BTreeMap<McpCredentialKey, McpCredentialSettings>,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            revision: 1,
            servers: Vec::new(),
            credentials: BTreeMap::new(),
        }
    }
}

impl McpSettings {
    pub fn validate(&self) -> Result<(), McpError> {
        if self.revision == 0 || self.servers.len() > MAX_MCP_SERVERS {
            return Err(McpError::InvalidConfig("MCP settings are invalid".into()));
        }
        let mut ids = HashSet::with_capacity(self.servers.len());
        for server in &self.servers {
            server.validate()?;
            if !ids.insert(server.id.clone()) {
                return Err(McpError::InvalidConfig(
                    "MCP server identifiers must be unique".into(),
                ));
            }
        }
        for credential in self.credentials.values() {
            if credential.generation == 0 {
                return Err(McpError::InvalidConfig(
                    "credential generations must be positive".into(),
                ));
            }
            validate_credential_value(&credential.value)?;
        }
        Ok(())
    }

    pub fn server(&self, server_id: &McpServerId) -> Option<&McpServerConfig> {
        self.servers.iter().find(|server| server.id == *server_id)
    }
}
