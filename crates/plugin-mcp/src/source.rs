//! The MCP tool source: the tools of the connected MCP servers, planned per
//! sampling step from one step binding.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentToolError, AgentToolOmission, AgentToolSource, ToolFuture, ToolSourceContribution,
    ToolSourceStep, MAX_AGENT_TOOL_OMISSIONS,
};
use hanafubuki_agent_mcp::McpManager;

use crate::tool::mcp_tool;

const SOURCE_ID: &str = "mcp";

#[cfg(test)]
mod tests;

/// Refreshes the catalogs that changed, freezes one step binding, plans its
/// tools and returns the binding as the plan's lease, which the resource
/// tools read through.
pub(crate) struct McpToolSource {
    pub(crate) mcp: Arc<McpManager>,
}

impl AgentToolSource for McpToolSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn contribute<'a>(
        &'a self,
        step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>> {
        Box::pin(async move {
            let cancellation = &step.cancellation;
            if cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            self.mcp
                .refresh_changed_catalogs(cancellation.clone())
                .await
                .map_err(|error| AgentToolError::Failed(error.to_string()))?;
            if cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            let binding = Arc::new(self.mcp.step_binding());
            let mut contribution = ToolSourceContribution::default();
            for lease in binding.tool_leases().iter().cloned() {
                let server = binding.server(&lease.binding.server_id);
                let instructions = server.and_then(|server| server.instructions_summary.clone());
                let required = server.is_some_and(|server| server.required);
                let tool_name = lease.binding.specification.name.flattened_alias();
                match mcp_tool(&self.mcp, lease, instructions, required) {
                    Ok(tool) => contribution.tools.push(tool),
                    Err(error) if required => return Err(error),
                    Err(error) => {
                        let reason = match error {
                            AgentToolError::Failed(message) => message,
                            AgentToolError::Cancelled => return Err(error),
                        };
                        let reason = reason.chars().take(512).collect::<String>();
                        log::warn!(
                            "optional MCP tool omitted from Agent plan: {tool_name}: {reason}"
                        );
                        if contribution.omissions.len() < MAX_AGENT_TOOL_OMISSIONS {
                            contribution.omissions.push(AgentToolOmission {
                                contributor: SOURCE_ID.to_string(),
                                tool_name: Some(tool_name),
                                reason,
                            });
                        }
                    }
                }
            }
            Ok(contribution.with_lease(binding))
        })
    }
}
