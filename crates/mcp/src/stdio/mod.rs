use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_process::executor::manager::{
    dispose_reserved_executor, read_reserved_executor_output, release_executor_reservation,
    reserve_executor_id, start_reserved_executor, write_reserved_executor_stdin,
    ReservedExecutorReceipt,
};
use hanafubuki_agent_process::executor::{
    EnvironmentInheritance, ExecutorIoMode, ExecutorManager, ExecutorStartRequest, OutputStream,
};
use hanafubuki_agent_process::ownership::ProcessOwnerId;
use hanafubuki_agent_tool_contracts::{canonical_digest, ToolOutput};
use parking_lot::Mutex;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, RequestMetaObject, Resource, ResourceTemplate,
    ServerJsonRpcMessage,
};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient};
use tokio::sync::{mpsc as tokio_mpsc, Mutex as AsyncMutex};

use crate::listing::{McpListedPage, McpListing};
use crate::{
    McpClientHandler, McpConnectionGeneration, McpError, McpInteractionHub, McpServerConfig,
    McpStdioConfig, McpToolBinding, McpToolCatalog, McpTransportConfig,
};

const MCP_EXECUTOR_OUTPUT_CAP: usize = 16 * 1024 * 1024;
const MCP_PROTOCOL_READ_BYTES: usize = 256 * 1024;
const MCP_DIAGNOSTIC_BYTES: usize = 32 * 1024;
const MCP_TRANSPORT_CHANNEL_CAPACITY: usize = 64;
const MCP_POLL_INTERVAL: Duration = Duration::from_millis(10);
static NEXT_MCP_EXECUTOR: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct McpResolvedSecretEnvironment(BTreeMap<String, String>);

impl std::fmt::Debug for McpResolvedSecretEnvironment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpResolvedSecretEnvironment")
            .field("count", &self.0.len())
            .finish()
    }
}

impl McpResolvedSecretEnvironment {
    pub fn new(values: BTreeMap<String, String>) -> Result<Self, McpError> {
        for (name, value) in &values {
            if name.is_empty() || name.contains('=') || name.contains('\0') || value.contains('\0')
            {
                return Err(McpError::InvalidConfig(
                    "secret environment entry is invalid".into(),
                ));
            }
        }
        Ok(Self(values))
    }
}

mod connection;
mod process;
mod transport;
mod worker;

use process::McpStdioProcess;
use transport::WorkerCommand;
use worker::{bounded_error, run_stdio_worker};

pub use connection::McpStdioConnection;
pub use transport::{McpExecutorTransport, McpExecutorTransportError};
