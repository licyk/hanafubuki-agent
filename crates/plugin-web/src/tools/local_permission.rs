// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, AuthorizedAction,
    PermissionRequirement, SandboxEscalation,
};
use hanafubuki_agent_contracts::{
    verify_tool_approval, AgentToolError, ToolFuture, ToolInvocation,
};
use hanafubuki_agent_web_search::WebSearchService;

fn action(
    service: &WebSearchService,
    tool: &str,
    invocation: &ToolInvocation,
) -> Result<Option<ApprovalAction>, AgentToolError> {
    if !service.local_helpers_enabled() {
        return Ok(None);
    }
    if matches!(
        tool,
        "weather" | "cache_search" | "domain_search" | "video_search"
    ) || (tool == "platform_search"
        && invocation
            .arguments
            .get("platform")
            .and_then(|v| v.as_str())
            != Some("x"))
    {
        return Ok(None);
    }
    let bytes = serde_json::to_vec(&(
        "web-local-helper-outside-sandbox-v1",
        tool,
        &invocation.arguments,
    ))
    .map_err(|e| AgentToolError::Failed(e.to_string()))?;
    let action=ApprovalAction::from_canonical_bytes(ApprovalActionKind::CommandExecution,ApprovalRiskClass::Critical,ApprovalOrigin::Model,format!("{tool}: allow configured local search/browser helpers to run outside the Agent sandbox in an isolated temporary directory; CLI helpers can access the signed-in account and local files"),&bytes).map_err(|e|AgentToolError::Failed(e.to_string()))?.with_sandbox_escalation(SandboxEscalation {justification:Some("Run the enabled local web helper for this call outside the Agent sandbox".into())});
    Ok(Some(action))
}
pub(super) fn permission<'a>(
    service: &'a WebSearchService,
    tool: &'a str,
    invocation: &'a ToolInvocation,
) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
    Box::pin(async move {
        if invocation.cancellation.is_cancelled() {
            return Err(AgentToolError::Cancelled);
        }
        Ok(action(service, tool, invocation)?
            .map(PermissionRequirement::ReviewRequired)
            .unwrap_or(PermissionRequirement::NotRequired))
    })
}
pub(super) fn verify(
    service: &WebSearchService,
    tool: &str,
    invocation: &ToolInvocation,
    approval: Option<&AuthorizedAction>,
) -> Result<(), AgentToolError> {
    verify_tool_approval(action(service, tool, invocation)?.as_ref(), approval)
        .map_err(AgentToolError::Failed)
}

#[cfg(test)]
mod tests;
