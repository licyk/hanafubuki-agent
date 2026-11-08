use super::*;

#[derive(Clone)]
pub struct McpConnectionSecrets {
    pub credential_reference_generation: u64,
    pub stdio_environment: McpResolvedSecretEnvironment,
    pub http_headers: McpResolvedSecretHeaders,
}

impl std::fmt::Debug for McpConnectionSecrets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpConnectionSecrets")
            .field(
                "credential_reference_generation",
                &self.credential_reference_generation,
            )
            .field("stdio_environment", &self.stdio_environment)
            .field("http_headers", &self.http_headers)
            .finish()
    }
}

impl McpConnectionSecrets {
    pub fn empty(credential_reference_generation: u64) -> Self {
        Self {
            credential_reference_generation,
            stdio_environment: McpResolvedSecretEnvironment::new(BTreeMap::new())
                .expect("empty environment is valid"),
            http_headers: McpResolvedSecretHeaders::new(BTreeMap::new())
                .expect("empty headers are valid"),
        }
    }

    pub fn with_oauth_bearer(
        mut self,
        token: String,
        oauth_generation: u64,
    ) -> Result<Self, McpError> {
        self.http_headers.insert_bearer_token(token)?;
        let digest = hanafubuki_agent_tool_contracts::canonical_digest(&(
            self.credential_reference_generation,
            oauth_generation,
        ))
        .map_err(|_| McpError::LimitExceeded)?;
        self.credential_reference_generation = u64::from_be_bytes(
            digest[..8]
                .try_into()
                .expect("credential generation digest prefix is eight bytes"),
        );
        Ok(self)
    }
}

pub(super) enum McpLiveConnection {
    Stdio(Arc<McpStdioConnection>),
    StreamableHttp(Arc<McpStreamableHttpConnection>),
}

impl McpLiveConnection {
    pub(super) fn generation(&self) -> McpConnectionGeneration {
        match self {
            Self::Stdio(connection) => connection.generation(),
            Self::StreamableHttp(connection) => connection.generation(),
        }
    }

    pub(super) fn closed(&self) -> tokio::sync::watch::Receiver<()> {
        match self {
            Self::Stdio(connection) => connection.closed(),
            Self::StreamableHttp(connection) => connection.closed(),
        }
    }

    /// Whether the transport ended; the connection cannot carry a request.
    pub(super) fn is_closed(&self) -> bool {
        self.closed().has_changed().is_err()
    }

    /// Why the transport ended, for the server's status.
    pub(super) fn closed_reason(&self) -> String {
        match self {
            Self::Stdio(connection) => connection.closed_reason(),
            Self::StreamableHttp(_) => {
                "the MCP server's Streamable HTTP transport closed".to_string()
            }
        }
    }

    pub(super) fn resolved_program(&self) -> Option<crate::McpResolvedProgram> {
        match self {
            Self::Stdio(connection) => Some(connection.resolved_program()),
            Self::StreamableHttp(_) => None,
        }
    }

    pub(super) fn capabilities(&self) -> crate::McpCapabilitySnapshot {
        match self {
            Self::Stdio(connection) => connection.capabilities(),
            Self::StreamableHttp(connection) => connection.capabilities(),
        }
    }

    pub(super) fn diagnostic_tail(&self) -> Option<String> {
        match self {
            Self::Stdio(connection) => Some(connection.diagnostic_tail()),
            Self::StreamableHttp(_) => None,
        }
    }

    pub(super) async fn call_tool(
        &self,
        binding: &McpToolBinding,
        arguments: serde_json::Map<String, serde_json::Value>,
        timeout: Duration,
        pause_state: crate::interaction::ElicitationPauseState,
        cancellation: CancellationToken,
    ) -> Result<ToolOutput, McpError> {
        let result = match self {
            Self::Stdio(connection) => {
                connection
                    .call_tool(binding, arguments, timeout, pause_state, cancellation)
                    .await?
            }
            Self::StreamableHttp(connection) => {
                connection
                    .call_tool(binding, arguments, timeout, pause_state, cancellation)
                    .await?
            }
        };
        convert_tool_output(result)
    }

    pub(super) async fn discover_tools(
        &self,
        config: &McpServerConfig,
    ) -> Result<McpToolCatalog, McpError> {
        match self {
            Self::Stdio(connection) => connection.discover_tools(config).await,
            Self::StreamableHttp(connection) => connection.discover_tools(config).await,
        }
    }

    pub(super) fn tools_changed(&self) -> bool {
        match self {
            Self::Stdio(connection) => connection.tools_changed(),
            Self::StreamableHttp(connection) => connection.tools_changed(),
        }
    }

    pub(super) fn clear_tools_changed(&self) {
        match self {
            Self::Stdio(connection) => connection.clear_tools_changed(),
            Self::StreamableHttp(connection) => connection.clear_tools_changed(),
        }
    }

    pub(super) fn supports_resources(&self) -> bool {
        match self {
            Self::Stdio(connection) => connection.supports_resources(),
            Self::StreamableHttp(connection) => connection.supports_resources(),
        }
    }

    pub(super) async fn list_resources(
        &self,
        timeout: Duration,
    ) -> Result<McpListing<Resource>, McpError> {
        match self {
            Self::Stdio(connection) => connection.list_resources(timeout).await,
            Self::StreamableHttp(connection) => connection.list_resources(timeout).await,
        }
    }

    pub(super) async fn list_resources_page(
        &self,
        cursor: Option<String>,
        timeout: Duration,
    ) -> Result<McpListedPage<Resource>, McpError> {
        match self {
            Self::Stdio(connection) => connection.list_resources_page(cursor, timeout).await,
            Self::StreamableHttp(connection) => {
                connection.list_resources_page(cursor, timeout).await
            }
        }
    }

    pub(super) async fn list_resource_templates(
        &self,
        timeout: Duration,
    ) -> Result<McpListing<ResourceTemplate>, McpError> {
        match self {
            Self::Stdio(connection) => connection.list_resource_templates(timeout).await,
            Self::StreamableHttp(connection) => connection.list_resource_templates(timeout).await,
        }
    }

    pub(super) async fn list_resource_templates_page(
        &self,
        cursor: Option<String>,
        timeout: Duration,
    ) -> Result<McpListedPage<ResourceTemplate>, McpError> {
        match self {
            Self::Stdio(connection) => {
                connection
                    .list_resource_templates_page(cursor, timeout)
                    .await
            }
            Self::StreamableHttp(connection) => {
                connection
                    .list_resource_templates_page(cursor, timeout)
                    .await
            }
        }
    }

    pub(super) async fn read_resource(
        &self,
        uri: String,
        timeout: Duration,
    ) -> Result<ToolOutput, McpError> {
        match self {
            Self::Stdio(connection) => connection.read_resource(uri, timeout).await,
            Self::StreamableHttp(connection) => connection.read_resource(uri, timeout).await,
        }
    }

    pub(super) async fn shutdown(&self, timeout: Duration) -> Result<(), McpError> {
        match self {
            Self::Stdio(connection) => connection.shutdown(timeout).await,
            Self::StreamableHttp(connection) => connection.shutdown(timeout).await,
        }
    }
}

pub(super) struct McpConnectionEntry {
    pub(super) config: McpServerConfig,
    /// What the connection was opened with, for an automatic reconnect.
    pub(super) secrets: McpConnectionSecrets,
    pub(super) connection: Arc<McpLiveConnection>,
    pub(super) tools: Vec<McpToolBinding>,
    pub(super) resource_count: usize,
    pub(super) resource_template_count: usize,
    pub(super) call_gate: Arc<Semaphore>,
    pub(super) tool_call_gates: HashMap<String, Arc<Semaphore>>,
}
