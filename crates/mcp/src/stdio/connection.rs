use super::*;
use rmcp::service::ClientServiceExt;

pub struct McpStdioConnection {
    generation: McpConnectionGeneration,
    capabilities: crate::McpCapabilitySnapshot,
    request_meta: Option<RequestMetaObject>,
    peer: rmcp::Peer<RoleClient>,
    running: AsyncMutex<Option<RunningService<RoleClient, McpClientHandler>>>,
    process: Arc<McpStdioProcess>,
    handler: McpClientHandler,
    closed: tokio::sync::watch::Receiver<()>,
}

impl McpStdioConnection {
    pub async fn connect(
        server: &McpServerConfig,
        secrets: McpResolvedSecretEnvironment,
        credential_reference_generation: u64,
        manager: Arc<ExecutorManager>,
        workers: BackgroundWorkerGroup,
        interactions: McpInteractionHub,
        cancellation: CancellationToken,
    ) -> Result<Arc<Self>, McpError> {
        let McpTransportConfig::Stdio(config) = &server.transport else {
            return Err(McpError::InvalidConfig(
                "stdio connection requires stdio transport config".into(),
            ));
        };
        if cancellation.is_cancelled() {
            return Err(McpError::Cancelled);
        }
        let generation = McpConnectionGeneration(
            canonical_digest(&(server, credential_reference_generation))
                .map_err(|_| McpError::LimitExceeded)?,
        );
        let (process, transport) =
            McpStdioProcess::spawn(server, config, secrets, manager, workers).await?;
        let (transport, closed) = crate::liveness::closing_transport(transport);
        let handler = McpClientHandler::new(
            server.id.clone(),
            generation,
            crate::MAXIMUM_MCP_ELICITATION_WAIT,
            interactions,
            server.protocol_mode,
        );
        // rmcp establishes the transport and sends initialize in one future, so the
        // outer bound is the sum of the independently configured stage budgets.
        let startup = tokio::time::timeout(
            server
                .connect_timeout()
                .saturating_add(server.initialize_timeout()),
            handler.clone().serve_with_lifecycle(
                transport,
                crate::client_adapter::lifecycle_mode(server.protocol_mode),
            ),
        );
        let startup = tokio::select! {
            result = startup => result,
            _ = crate::request::wait_cancelled(&cancellation) => {
                process.dispose_confirmed().await?;
                return Err(McpError::Cancelled);
            }
        };
        let running = match startup {
            Ok(Ok(running)) => running,
            Ok(Err(error)) => {
                let diagnostic = process.diagnostic();
                let cleanup = process.dispose_confirmed().await;
                if let Err(cleanup) = cleanup {
                    return Err(cleanup);
                }
                let diagnostic = crate::sanitize_mcp_diagnostic(&diagnostic, MCP_DIAGNOSTIC_BYTES);
                return Err(McpError::Protocol(bounded_error(format!(
                    "{error}; sanitized stderr tail={diagnostic}"
                ))));
            }
            Err(_) => {
                process.dispose_confirmed().await?;
                return Err(McpError::Timeout);
            }
        };
        if cancellation.is_cancelled() {
            let mut running = running;
            let _ = running.close_with_timeout(Duration::from_secs(5)).await;
            process.dispose_confirmed().await?;
            return Err(McpError::Cancelled);
        }
        let peer = running.peer().clone();
        let peer_info = peer
            .peer_info()
            .ok_or_else(|| McpError::Protocol("MCP initialize result is unavailable".into()))?;
        let request_meta =
            crate::client_adapter::request_meta(&peer_info.protocol_version, &handler.get_info());
        let capabilities = crate::McpCapabilitySnapshot::from_peer(generation, &peer_info);
        Ok(Arc::new(Self {
            generation,
            capabilities,
            request_meta,
            peer,
            running: AsyncMutex::new(Some(running)),
            process,
            handler,
            closed,
        }))
    }

    pub(crate) fn closed(&self) -> tokio::sync::watch::Receiver<()> {
        self.closed.clone()
    }

    /// Why the transport closed, once it has.
    pub(crate) fn closed_reason(&self) -> String {
        self.process
            .exit_reason()
            .unwrap_or_else(|| "the MCP server's stdio transport closed".to_string())
    }

    pub fn generation(&self) -> McpConnectionGeneration {
        self.generation
    }

    pub fn resolved_program(&self) -> crate::McpResolvedProgram {
        self.process.resolved_program()
    }

    pub fn capabilities(&self) -> crate::McpCapabilitySnapshot {
        self.capabilities.clone()
    }

    pub fn diagnostic_tail(&self) -> String {
        crate::sanitize_mcp_diagnostic(&self.process.diagnostic(), MCP_DIAGNOSTIC_BYTES)
    }

    pub fn tools_changed(&self) -> bool {
        self.handler.tools_changed()
    }

    pub fn clear_tools_changed(&self) {
        self.handler.clear_tools_changed();
    }

    pub fn supports_resources(&self) -> bool {
        self.peer
            .peer_info()
            .is_some_and(|info| info.capabilities.resources.is_some())
    }

    pub async fn discover_tools(
        &self,
        server: &McpServerConfig,
    ) -> Result<McpToolCatalog, McpError> {
        let tools = crate::listing::list_all::<crate::listing::ListTools>(
            &self.peer,
            server.list_timeout(),
            self.request_meta.as_ref(),
            crate::MAX_MCP_TOOLS_PER_SERVER,
        )
        .await?;
        crate::catalog::build_tool_catalog(server, self.generation, tools)
    }

    pub(crate) async fn call_tool(
        &self,
        binding: &McpToolBinding,
        arguments: serde_json::Map<String, serde_json::Value>,
        timeout: Duration,
        pause_state: crate::interaction::ElicitationPauseState,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, McpError> {
        if binding.connection_generation != self.generation {
            return Err(McpError::StaleGeneration);
        }
        if cancellation.is_cancelled() {
            return Err(McpError::Cancelled);
        }
        let request = crate::client_adapter::apply_request_meta(
            CallToolRequestParams::new(binding.remote_name.clone()).with_arguments(arguments),
            self.request_meta.as_ref(),
        );
        crate::request::call_tool_cancellable(
            &self.peer,
            request,
            timeout,
            pause_state,
            cancellation,
        )
        .await
    }

    pub(crate) async fn list_resources(
        &self,
        timeout: Duration,
    ) -> Result<McpListing<Resource>, McpError> {
        crate::listing::list_all::<crate::listing::ListResources>(
            &self.peer,
            timeout,
            self.request_meta.as_ref(),
            crate::listing::MAXIMUM_MCP_LIST_ITEMS,
        )
        .await
    }

    pub(crate) async fn list_resources_page(
        &self,
        cursor: Option<String>,
        timeout: Duration,
    ) -> Result<McpListedPage<Resource>, McpError> {
        crate::listing::list_page::<crate::listing::ListResources>(
            &self.peer,
            cursor,
            timeout,
            self.request_meta.as_ref(),
        )
        .await
    }

    pub(crate) async fn list_resource_templates(
        &self,
        timeout: Duration,
    ) -> Result<McpListing<ResourceTemplate>, McpError> {
        crate::listing::list_all::<crate::listing::ListResourceTemplates>(
            &self.peer,
            timeout,
            self.request_meta.as_ref(),
            crate::listing::MAXIMUM_MCP_LIST_ITEMS,
        )
        .await
    }

    pub(crate) async fn list_resource_templates_page(
        &self,
        cursor: Option<String>,
        timeout: Duration,
    ) -> Result<McpListedPage<ResourceTemplate>, McpError> {
        crate::listing::list_page::<crate::listing::ListResourceTemplates>(
            &self.peer,
            cursor,
            timeout,
            self.request_meta.as_ref(),
        )
        .await
    }

    pub async fn read_resource(
        &self,
        uri: String,
        timeout: Duration,
    ) -> Result<ToolOutput, McpError> {
        crate::request::read_resource(&self.peer, uri, timeout, self.request_meta.as_ref()).await
    }

    pub async fn shutdown(&self, timeout: Duration) -> Result<(), McpError> {
        let mut running = self.running.lock().await;
        if let Some(service) = running.as_mut() {
            let closed = service
                .close_with_timeout(timeout)
                .await
                .map_err(|error| McpError::CleanupUnconfirmed(bounded_error(error.to_string())))?;
            if closed.is_none() {
                return Err(McpError::CleanupUnconfirmed(
                    "stdio RMCP service did not close before its deadline".into(),
                ));
            }
            running.take();
        }
        self.process.dispose_confirmed().await
    }
}
