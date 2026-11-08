//! A scripted MCP server in the test process: each mounted server lists the
//! tools it was given and answers every call with one text naming the call,
//! over Streamable HTTP on a loopback port.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{ErrorData, ServerHandler};
use serde_json::json;
use tokio::sync::Notify;

/// Holds the servers' calls once armed: each call signals its entry and
/// waits for a release. A call the client abandons stays held until the
/// servers stop.
#[derive(Default)]
pub struct McpCallHold {
    armed: AtomicBool,
    entered: Notify,
    released: Notify,
}

impl McpCallHold {
    pub fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    /// Waits until a held call has entered the server.
    pub async fn entered(&self) {
        self.entered.notified().await;
    }

    pub fn release(&self) {
        self.released.notify_one();
    }

    async fn pass(&self) {
        if self.armed.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.released.notified().await;
        }
    }
}

/// A read-only tool with an object schema taking one optional string: a
/// concurrent read, offered in Plan mode.
pub fn scripted_tool(name: &str, description: &str) -> Tool {
    annotated_tool(
        name,
        description,
        ToolAnnotations::new().read_only(true).destructive(false),
    )
}

/// [`scripted_tool`] with `annotations`.
pub fn annotated_tool(name: &str, description: &str, annotations: ToolAnnotations) -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {"value": {"type": "string", "description": "Any text."}},
        "additionalProperties": false
    });
    let serde_json::Value::Object(schema) = schema else {
        unreachable!()
    };
    Tool::new(name.to_string(), description.to_string(), schema).with_annotations(annotations)
}

#[derive(Clone)]
struct ScriptedServer {
    tools: Vec<Tool>,
    hold: Arc<McpCallHold>,
}

impl ServerHandler for ScriptedServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.hold.pass().await;
        let arguments = serde_json::Value::Object(request.arguments.unwrap_or_default());
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "{} called with {arguments}",
            request.name
        ))])
        .into())
    }
}

/// The servers, each at `/<id>` of one loopback listener; stopped on drop.
pub struct ScriptedMcpServers {
    pub hold: Arc<McpCallHold>,
    base_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl ScriptedMcpServers {
    pub async fn start(servers: BTreeMap<&str, Vec<Tool>>) -> Self {
        let hold = Arc::new(McpCallHold::default());
        let mut router = axum::Router::new();
        for (id, tools) in servers {
            let server = ScriptedServer {
                tools,
                hold: hold.clone(),
            };
            let service: StreamableHttpService<ScriptedServer, LocalSessionManager> =
                StreamableHttpService::new(
                    move || Ok(server.clone()),
                    Default::default(),
                    StreamableHttpServerConfig::default().with_sse_keep_alive(None),
                );
            router = router.nest_service(&format!("/{id}"), service);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self {
            hold,
            base_url: format!("http://{address}"),
            task,
        }
    }

    /// An enabled, optional Legacy-protocol server `id` that needs no
    /// approval, with `exposure` for all of its tools.
    pub fn config(
        &self,
        id: &str,
        exposure: hanafubuki_agent_mcp::McpExposurePolicy,
    ) -> hanafubuki_agent_mcp::McpServerConfig {
        hanafubuki_agent_mcp::McpServerConfig {
            id: hanafubuki_agent_mcp::McpServerId::parse(id).unwrap(),
            enabled: true,
            required: false,
            protocol_mode: hanafubuki_agent_mcp::McpProtocolMode::Legacy,
            transport: hanafubuki_agent_mcp::McpTransportConfig::StreamableHttp(
                hanafubuki_agent_mcp::McpStreamableHttpConfig {
                    url: format!("{}/{id}", self.base_url),
                    public_headers: BTreeMap::new(),
                    credential_headers: BTreeMap::new(),
                    oauth: hanafubuki_agent_mcp::McpOAuthPolicy::Disabled,
                },
            ),
            connect_timeout_seconds: 5,
            initialize_timeout_seconds: 5,
            list_timeout_seconds: 5,
            call_timeout_seconds: 5,
            shutdown_timeout_seconds: 5,
            maximum_concurrent_calls: 1,
            tool_maximum_concurrent_calls: BTreeMap::new(),
            enabled_tools: BTreeSet::new(),
            disabled_tools: BTreeSet::new(),
            approval_policy: hanafubuki_agent_mcp::McpApprovalPolicy::Allow,
            tool_approval_policies: BTreeMap::new(),
            exposure_policy: exposure,
            tool_exposure_policies: BTreeMap::new(),
        }
    }
}

impl Drop for ScriptedMcpServers {
    fn drop(&mut self) {
        self.task.abort();
    }
}
