use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::{canonical_digest, ToolOutput};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, RequestMetaObject, Resource, ResourceTemplate,
};
use rmcp::service::ClientServiceExt;
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientHandler, RoleClient};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::listing::{McpListedPage, McpListing};
use crate::{
    McpClientHandler, McpConnectionGeneration, McpError, McpInteractionHub, McpServerConfig,
    McpStreamableHttpConfig, McpToolBinding, McpToolCatalog, McpTransportConfig,
};

#[derive(Clone)]
pub struct McpResolvedSecretHeaders(BTreeMap<String, String>);

impl std::fmt::Debug for McpResolvedSecretHeaders {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpResolvedSecretHeaders")
            .field("count", &self.0.len())
            .finish()
    }
}

impl McpResolvedSecretHeaders {
    pub fn new(values: BTreeMap<String, String>) -> Result<Self, McpError> {
        for (name, value) in &values {
            http::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| McpError::InvalidConfig("secret header name is invalid".into()))?;
            http::HeaderValue::from_str(value)
                .map_err(|_| McpError::InvalidConfig("secret header value is invalid".into()))?;
        }
        Ok(Self(values))
    }

    pub(crate) fn insert_bearer_token(&mut self, token: String) -> Result<(), McpError> {
        if self
            .0
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
        {
            return Err(McpError::InvalidConfig(
                "OAuth and a credential Authorization header cannot both be configured".into(),
            ));
        }
        let value = format!("Bearer {token}");
        http::HeaderValue::from_str(&value)
            .map_err(|_| McpError::InvalidConfig("OAuth access token is invalid".into()))?;
        self.0.insert("authorization".into(), value);
        Ok(())
    }
}

pub struct McpStreamableHttpConnection {
    generation: McpConnectionGeneration,
    capabilities: crate::McpCapabilitySnapshot,
    request_meta: Option<RequestMetaObject>,
    peer: rmcp::Peer<RoleClient>,
    running: Mutex<Option<RunningService<RoleClient, McpClientHandler>>>,
    handler: McpClientHandler,
    closed: tokio::sync::watch::Receiver<()>,
}

impl McpStreamableHttpConnection {
    pub async fn connect(
        server: &McpServerConfig,
        secret_headers: McpResolvedSecretHeaders,
        credential_reference_generation: u64,
        interactions: McpInteractionHub,
        http_client: Option<reqwest::Client>,
        cancellation: CancellationToken,
    ) -> Result<Arc<Self>, McpError> {
        let McpTransportConfig::StreamableHttp(config) = &server.transport else {
            return Err(McpError::InvalidConfig(
                "streamable HTTP connection requires HTTP transport config".into(),
            ));
        };
        if cancellation.is_cancelled() {
            return Err(McpError::Cancelled);
        }
        let generation = connection_generation(server, credential_reference_generation)?;
        let http_client = match http_client {
            Some(client) => client,
            None => crate::build_mcp_http_client(crate::McpHttpProxyPolicy::System)?,
        };
        let handler = McpClientHandler::new(
            server.id.clone(),
            generation,
            crate::MAXIMUM_MCP_ELICITATION_WAIT,
            interactions,
            server.protocol_mode,
        );
        let startup_deadline = Instant::now()
            + server
                .connect_timeout()
                .saturating_add(server.initialize_timeout());
        let mut attempt = 0usize;
        let (running, closed) = loop {
            let remaining = startup_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(McpError::Timeout);
            }
            let (transport, closed) =
                crate::liveness::closing_transport(StreamableHttpClientTransport::with_client(
                    http_client.clone(),
                    transport_config(config, secret_headers.clone())?,
                ));
            let startup = tokio::time::timeout(
                remaining,
                handler.clone().serve_with_lifecycle(
                    transport,
                    crate::client_adapter::lifecycle_mode(server.protocol_mode),
                ),
            );
            let result = tokio::select! {
                result = startup => result,
                _ = crate::request::wait_cancelled(&cancellation) => {
                    return Err(McpError::Cancelled);
                }
            };
            match result {
                Ok(Ok(running)) => break (running, closed),
                Ok(Err(_error)) if attempt == 0 => {
                    attempt += 1;
                    log::debug!("MCP Streamable HTTP initialize retry scheduled");
                }
                Ok(Err(error)) => {
                    return Err(McpError::Protocol(bounded_error(error.to_string())));
                }
                Err(_) => return Err(McpError::Timeout),
            }
        };
        if cancellation.is_cancelled() {
            let mut running = running;
            let _ = running.close_with_timeout(Duration::from_secs(5)).await;
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
            running: Mutex::new(Some(running)),
            handler,
            closed,
        }))
    }

    pub fn generation(&self) -> McpConnectionGeneration {
        self.generation
    }

    pub(crate) fn closed(&self) -> tokio::sync::watch::Receiver<()> {
        self.closed.clone()
    }

    pub fn capabilities(&self) -> crate::McpCapabilitySnapshot {
        self.capabilities.clone()
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
        self.verify_binding(binding)?;
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
        let Some(service) = running.as_mut() else {
            return Ok(());
        };
        match service
            .close_with_timeout(timeout)
            .await
            .map_err(|error| McpError::Protocol(bounded_error(error.to_string())))?
        {
            Some(_) => {
                running.take();
                Ok(())
            }
            None => Err(McpError::CleanupUnconfirmed(
                "streamable HTTP service did not close before its deadline".into(),
            )),
        }
    }

    fn verify_binding(&self, binding: &McpToolBinding) -> Result<(), McpError> {
        if binding.connection_generation != self.generation {
            return Err(McpError::StaleGeneration);
        }
        Ok(())
    }
}

pub(crate) fn transport_config(
    config: &McpStreamableHttpConfig,
    secret_headers: McpResolvedSecretHeaders,
) -> Result<StreamableHttpClientTransportConfig, McpError> {
    let mut headers = HashMap::new();
    for (name, value) in config.public_headers.iter().chain(secret_headers.0.iter()) {
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| McpError::InvalidConfig("HTTP header name is invalid".into()))?;
        let value = http::HeaderValue::from_str(value)
            .map_err(|_| McpError::InvalidConfig("HTTP header value is invalid".into()))?;
        if headers.insert(name, value).is_some() {
            return Err(McpError::InvalidConfig("HTTP header names overlap".into()));
        }
    }
    Ok(
        StreamableHttpClientTransportConfig::with_uri(config.url.clone())
            .custom_headers(headers)
            .max_sse_event_size(crate::client_adapter::MAXIMUM_MCP_MESSAGE_BYTES)
            .reinit_on_expired_session(true),
    )
}

fn connection_generation<T: Serialize>(
    config: &T,
    credential_reference_generation: u64,
) -> Result<McpConnectionGeneration, McpError> {
    Ok(McpConnectionGeneration(
        canonical_digest(&(config, credential_reference_generation))
            .map_err(|_| McpError::LimitExceeded)?,
    ))
}

fn bounded_error(message: String) -> String {
    crate::sanitize_mcp_diagnostic(&message, 2048)
}
