use super::*;
use crate::McpCatalogGeneration;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourcePage<T> {
    pub server: McpServerId,
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

/// Exact per-server resource and metadata authority captured for one sampling
/// step. Holding this lease keeps the selected connection alive across refresh.
#[derive(Clone)]
pub struct McpStepServerBinding {
    pub server_id: McpServerId,
    pub required: bool,
    pub config_generation: [u8; 32],
    pub connection_generation: McpConnectionGeneration,
    pub catalog_generation: McpCatalogGeneration,
    pub instructions_summary: Option<String>,
    pub supports_resources: bool,
    timeout: Duration,
    connection: Arc<McpLiveConnection>,
    diagnostics: McpDiagnosticStore,
    transport: &'static str,
}

impl McpStepServerBinding {
    /// One page of the server's resources. Entries that do not decode are
    /// left out of the page and counted in one diagnostic of the server.
    pub async fn list_resources(
        &self,
        cursor: Option<String>,
    ) -> Result<McpResourcePage<Resource>, McpError> {
        validate_cursor(cursor.as_deref())?;
        let page = self
            .connection
            .list_resources_page(cursor, self.timeout)
            .await?;
        self.diagnostic_scope()
            .record_page_skipped(McpResourceEntryKind::Resource, page.undecodable.len());
        Ok(McpResourcePage {
            server: self.server_id.clone(),
            items: page.items,
            next_cursor: page.next_cursor,
        })
    }

    /// One page of the server's resource templates, like
    /// [`Self::list_resources`].
    pub async fn list_resource_templates(
        &self,
        cursor: Option<String>,
    ) -> Result<McpResourcePage<ResourceTemplate>, McpError> {
        validate_cursor(cursor.as_deref())?;
        let page = self
            .connection
            .list_resource_templates_page(cursor, self.timeout)
            .await?;
        self.diagnostic_scope()
            .record_page_skipped(McpResourceEntryKind::Template, page.undecodable.len());
        Ok(McpResourcePage {
            server: self.server_id.clone(),
            items: page.items,
            next_cursor: page.next_cursor,
        })
    }

    /// The resource as tool output; an entry that does not decode is a
    /// placeholder (`output::convert_read_resource_output`).
    pub async fn read_resource(&self, uri: String) -> Result<ToolOutput, McpError> {
        self.connection.read_resource(uri, self.timeout).await
    }

    fn diagnostic_scope(&self) -> McpResourceDiagnosticScope<'_> {
        McpResourceDiagnosticScope {
            diagnostics: &self.diagnostics,
            server_id: &self.server_id,
            transport: self.transport,
            config_generation: self.config_generation,
            runtime_generation: Some(self.connection_generation),
        }
    }
}

/// Complete immutable MCP authority retained by one Agent Tool Plan.
#[derive(Clone)]
pub struct McpStepBinding {
    tools: Vec<McpToolLease>,
    servers: Vec<McpStepServerBinding>,
}

impl McpStepBinding {
    pub fn tool_leases(&self) -> &[McpToolLease] {
        &self.tools
    }

    pub fn servers(&self) -> &[McpStepServerBinding] {
        &self.servers
    }

    pub fn server(&self, id: &McpServerId) -> Option<&McpStepServerBinding> {
        self.servers.iter().find(|server| &server.server_id == id)
    }
}

impl McpManager {
    pub fn step_binding(&self) -> McpStepBinding {
        let registry = self.registry.lock();
        let mut tools = Vec::new();
        let mut servers = Vec::with_capacity(registry.active.len());
        for entry in registry.active.values() {
            let connection_generation = entry.connection.generation();
            let catalog_generation = entry.tools.first().map_or_else(
                || {
                    McpCatalogGeneration(
                        canonical_digest(&(
                            entry.config.id.as_str(),
                            connection_generation,
                            "resource-catalog",
                        ))
                        .expect("bounded MCP identities have a canonical digest"),
                    )
                },
                |tool| tool.catalog_generation,
            );
            let capabilities = entry.connection.capabilities();
            servers.push(McpStepServerBinding {
                server_id: entry.config.id.clone(),
                required: entry.config.required,
                config_generation: config_generation(&entry.config).unwrap_or([0; 32]),
                connection_generation,
                catalog_generation,
                instructions_summary: capabilities.instructions_summary,
                supports_resources: entry.connection.supports_resources(),
                timeout: entry.config.list_timeout(),
                connection: entry.connection.clone(),
                diagnostics: self.diagnostics.clone(),
                transport: transport_name(&entry.config),
            });
            tools.extend(entry.tools.iter().cloned().map(|binding| {
                let call_gate = entry
                    .tool_call_gates
                    .get(&binding.remote_name)
                    .cloned()
                    .unwrap_or_else(|| entry.call_gate.clone());
                McpToolLease {
                    approval_policy: entry.config.approval_policy_for(&binding.remote_name),
                    binding,
                    timeout: entry.config.call_timeout(),
                    connection: entry.connection.clone(),
                    call_gate,
                    interactions: self.interactions.clone(),
                    diagnostics: self.diagnostics.clone(),
                    config_generation: config_generation(&entry.config).unwrap_or([0; 32]),
                    transport: transport_name(&entry.config).to_string(),
                }
            }));
        }
        servers.sort_by(|left, right| left.server_id.cmp(&right.server_id));
        tools.sort_by(|left, right| {
            left.binding
                .specification
                .name
                .qualified()
                .cmp(&right.binding.specification.name.qualified())
        });
        McpStepBinding { tools, servers }
    }
}

fn validate_cursor(cursor: Option<&str>) -> Result<(), McpError> {
    if cursor.is_some_and(|cursor| {
        cursor.is_empty() || cursor.len() > crate::listing::MAXIMUM_MCP_CURSOR_BYTES
    }) {
        return Err(McpError::LimitExceeded);
    }
    Ok(())
}
