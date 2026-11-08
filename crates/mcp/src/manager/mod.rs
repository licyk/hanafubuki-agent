use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_host::{RestartPolicy, RestartWindow};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_tool_contracts::{canonical_digest, ToolOutput};
use parking_lot::Mutex;
use rmcp::model::{Resource, ResourceTemplate};
use tokio::sync::{Notify, Semaphore};

use crate::listing::{McpListedPage, McpListing};
use crate::{
    convert_tool_output, McpApprovalPolicy, McpCatalogIssue, McpConnectionGeneration,
    McpConnectionPhase, McpDiagnosticOutcome, McpDiagnosticStage, McpDiagnosticStore,
    McpElicitationDecision, McpElicitationRequest, McpElicitationRequestId, McpError, McpEventKind,
    McpEventNotice, McpEventSink, McpInteractionHub, McpResolvedSecretEnvironment,
    McpResolvedSecretHeaders, McpServerConfig, McpServerId, McpServerStatus, McpStdioConnection,
    McpStreamableHttpConnection, McpToolBinding, McpToolCatalog, McpTransportConfig,
    NoopMcpEventSink, MAX_MCP_CATALOG_ISSUE_NAME_BYTES,
};

pub(crate) mod binding;
mod catalog;
mod connect;
mod diagnostics;
mod lease;
mod lifecycle;
mod model;
mod reconnect;
mod resources;
mod status;

pub use binding::{McpResourcePage, McpStepBinding, McpStepServerBinding};
use catalog::tool_call_gates;
use diagnostics::*;
use model::{McpConnectionEntry, McpLiveConnection};
use resources::{
    discover_resources, McpResourceDiagnosticScope, McpResourceDiscovery, McpResourceEntryKind,
};
use status::entry_status;

pub use lease::McpToolLease;
pub use model::McpConnectionSecrets;

#[derive(Default)]
struct McpManagerRegistry {
    active: HashMap<McpServerId, Arc<McpConnectionEntry>>,
    retired: Vec<Arc<McpConnectionEntry>>,
    statuses: HashMap<McpServerId, McpServerStatus>,
    operations: HashSet<McpServerId>,
    connect_operations: HashMap<McpServerId, Arc<McpConnectOperation>>,
    /// The automatic reconnects each server made in the current window.
    reconnects: HashMap<McpServerId, RestartWindow>,
}

struct McpConnectOperation {
    config_generation: [u8; 32],
    completed: AtomicBool,
    completion: Notify,
    result: Mutex<Option<Result<McpServerStatus, McpError>>>,
}

#[derive(Clone)]
pub struct McpManager {
    executor: Arc<ExecutorManager>,
    workers: BackgroundWorkerGroup,
    registry: Arc<Mutex<McpManagerRegistry>>,
    interactions: McpInteractionHub,
    events: Arc<dyn McpEventSink>,
    diagnostics: McpDiagnosticStore,
    /// Replaceable so a proxy change reaches new connections without a
    /// restart; read under a brief lock, never across an await.
    http_client: Arc<parking_lot::RwLock<Option<reqwest::Client>>>,
    /// When a server whose connection closed is connected again.
    reconnect_policy: Arc<RestartPolicy>,
    /// Cancelled by shutdown: no reconnect starts after it.
    lifetime: CancellationToken,
}

impl McpManager {
    pub fn new(executor: Arc<ExecutorManager>, workers: BackgroundWorkerGroup) -> Self {
        Self::new_with_event_sink(executor, workers, Arc::new(NoopMcpEventSink))
    }

    pub fn new_with_event_sink(
        executor: Arc<ExecutorManager>,
        workers: BackgroundWorkerGroup,
        events: Arc<dyn McpEventSink>,
    ) -> Self {
        Self {
            executor,
            workers,
            registry: Arc::new(Mutex::new(McpManagerRegistry::default())),
            interactions: McpInteractionHub::new(events.clone()),
            events,
            diagnostics: McpDiagnosticStore::default(),
            http_client: Arc::new(parking_lot::RwLock::new(None)),
            reconnect_policy: Arc::new(RestartPolicy::default()),
            lifetime: CancellationToken::default(),
        }
    }

    pub fn with_reconnect_policy(mut self, policy: RestartPolicy) -> Self {
        self.reconnect_policy = Arc::new(policy);
        self
    }

    pub fn with_http_client(self, http_client: reqwest::Client) -> Self {
        self.set_http_client(http_client);
        self
    }

    /// The client later Streamable HTTP connections use. Connections already
    /// open keep theirs until they are reconnected.
    pub fn set_http_client(&self, http_client: reqwest::Client) {
        *self.http_client.write() = Some(http_client);
    }

    pub(super) fn http_client(&self) -> Option<reqwest::Client> {
        self.http_client.read().clone()
    }
}
