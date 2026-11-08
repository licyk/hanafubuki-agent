use std::sync::Arc;

use hanafubuki_agent_contracts::{AgentToolSource, ToolSourceStep};
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_mcp::{McpManager, McpStepBinding};
use hanafubuki_agent_process::executor::ExecutorManager;
use serde_json::json;

use super::McpToolSource;

fn manager() -> Arc<McpManager> {
    Arc::new(McpManager::new(
        Arc::new(ExecutorManager::default()),
        BackgroundWorkerGroup::default(),
    ))
}

#[tokio::test]
async fn the_source_freezes_one_step_binding_as_the_plans_lease() {
    let source = McpToolSource { mcp: manager() };
    let step = ToolSourceStep {
        session_id: serde_json::from_value(json!(1)).unwrap(),
        turn_id: serde_json::from_value(json!(1)).unwrap(),
        generation: serde_json::from_value(json!(1)).unwrap(),
        session_revision: serde_json::from_value(json!(1)).unwrap(),
        memory_contribution: false,
        cancellation: CancellationToken::default(),
    };
    let contribution = source.contribute(&step).await.unwrap();
    assert!(contribution.tools.is_empty());
    let lease = contribution.lease.unwrap();
    assert!(lease.as_ref().as_any().is::<McpStepBinding>());
}
