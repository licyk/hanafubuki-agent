mod args;
mod fetch;
mod search;

pub(crate) use fetch::FetchWebPageTool;
pub(crate) use search::WebSearchTool;

use hanafubuki_agent_approval::{ApprovalAction, PermissionRequirement};
use hanafubuki_agent_contracts::{AgentToolError, ToolFuture, ToolInvocation};

/// Web reads need no approval: they reach only public targets the service's
/// target policy admits, and their content is marked as untrusted.
fn no_approval_required(
    invocation: &ToolInvocation,
) -> ToolFuture<'_, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
    let cancelled = invocation.cancellation.is_cancelled();
    Box::pin(async move {
        if cancelled {
            Err(AgentToolError::Cancelled)
        } else {
            Ok(PermissionRequirement::NotRequired)
        }
    })
}

#[cfg(test)]
mod tests;
