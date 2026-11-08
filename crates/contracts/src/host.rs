use std::sync::Arc;

use hanafubuki_agent_foundation::blocking::{run_blocking_operation, BlockingOperationPort};
use kardia_macros::Service;

use crate::AgentToolError;

/// The application's blocking-work boundary, published by the Agent host.
///
/// Tools whose domain service blocks (filesystem, processes, instance
/// operations) run that work here, never on the async runtime's workers. The
/// kernel serializes `Exclusive` tools, so this needs no per-session lane.
#[derive(Service)]
#[service(name = "agent-blocking")]
pub struct BlockingService(Arc<dyn BlockingOperationPort>);

impl BlockingService {
    pub fn new(port: Arc<dyn BlockingOperationPort>) -> Self {
        Self(port)
    }

    pub async fn run<T, F>(&self, label: &'static str, work: F) -> Result<T, AgentToolError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        run_blocking_operation(self.0.as_ref(), label, work)
            .await
            .map_err(AgentToolError::Failed)
    }
}
