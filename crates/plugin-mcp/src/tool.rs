use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalActionKind, ApprovalOrigin, ApprovalRiskClass, AuthorizedAction,
    PermissionActionDetails, PermissionRequirement,
};
use hanafubuki_agent_contracts::{
    AgentActivityAction, AgentMemoryContextEffect, AgentToolError, AgentToolHandler, SourcedTool,
    ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolResult, ToolSourceTrust,
};
use hanafubuki_agent_foundation::panic::catch_panic_async;
use hanafubuki_agent_mcp::{McpApprovalPolicy, McpExposurePolicy, McpManager, McpToolLease};
use hanafubuki_agent_tool_contracts::{ToolExecutionClass, ToolExposure, ToolProviderId};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Plans one MCP tool lease as a tool bound to that exact lease. Its
/// identity is the server's provider id and the catalog's tool id and
/// generation, so a persisted deferred selection keeps naming it while the
/// server's catalog does not change it.
pub(super) fn mcp_tool(
    mcp: &Arc<McpManager>,
    lease: McpToolLease,
    instructions: Option<String>,
    required: bool,
) -> Result<SourcedTool, AgentToolError> {
    let binding = &lease.binding;
    let server_id = binding.server_id.as_str();
    let provider_id = ToolProviderId::parse(format!("mcp.{server_id}"))
        .map_err(|_| AgentToolError::Failed("tool catalog contains an invalid tool name".into()))?;
    let namespace_description = Some(match instructions {
        Some(instructions) if !instructions.trim().is_empty() => {
            format!("MCP server {server_id}. {instructions}")
        }
        _ => format!("MCP server {server_id}."),
    });
    // A read-only hint relaxes scheduling and Plan mode only; approval stays
    // with the server's policy and the destructive hint (`permission`), and
    // the output is external content either way, as a web fetch's is.
    let (execution_class, effect) = if binding.hints.read_only {
        (ToolExecutionClass::ConcurrentRead, ToolEffect::ReadOnly)
    } else {
        (ToolExecutionClass::Exclusive, ToolEffect::Mutating)
    };
    let descriptor = ToolDescriptor {
        name: binding.specification.name.clone(),
        description: binding.specification.description.clone(),
        input_schema: binding.specification.input_schema.clone(),
        exposure: match binding.exposure {
            McpExposurePolicy::Direct => ToolExposure::Direct,
            McpExposurePolicy::DirectModelOnly => ToolExposure::DirectModelOnly,
            McpExposurePolicy::Deferred => ToolExposure::Deferred,
            McpExposurePolicy::Hidden => ToolExposure::Hidden,
        },
        execution_class,
        effect,
        action: AgentActivityAction::USE_TOOL,
        memory_effect: AgentMemoryContextEffect::PollutesContribution,
        // The server's own timeout pauses while an elicitation waits for the
        // user, so the kernel's bound leaves room for one such wait and the
        // server's timeout stays the one that fires.
        timeout: Some(
            lease
                .call_timeout()
                .saturating_add(hanafubuki_agent_mcp::MAXIMUM_MCP_ELICITATION_WAIT),
        ),
    };
    Ok(SourcedTool {
        provider_id,
        tool_id: binding.tool_id.clone(),
        generation: binding.tool_generation,
        namespace_description,
        subject: Some(format!("MCP {server_id}")),
        trust: if required {
            ToolSourceTrust::Required
        } else {
            ToolSourceTrust::Optional
        },
        handler: Arc::new(McpTool {
            descriptor,
            lease,
            mcp: mcp.clone(),
        }),
    })
}

/// One MCP tool, frozen to the lease it was planned from: a call never
/// reopens the server's catalog by name.
struct McpTool {
    descriptor: ToolDescriptor,
    lease: McpToolLease,
    /// Told when the call panics, so the fault stays the server's.
    mcp: Arc<McpManager>,
}

impl AgentToolHandler for McpTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    fn permission<'a>(
        &'a self,
        invocation: &'a ToolInvocation,
    ) -> ToolFuture<'a, Result<PermissionRequirement<ApprovalAction>, AgentToolError>> {
        Box::pin(async move {
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            // Not worth a review: the call could not reach the tool.
            if self.lease.is_stale() {
                return Err(AgentToolError::Failed(
                    hanafubuki_agent_mcp::McpError::CatalogChanged.to_string(),
                ));
            }
            match self.lease.approval_policy {
                McpApprovalPolicy::Allow if !self.lease.binding.hints.destructive => {
                    Ok(PermissionRequirement::NotRequired)
                }
                McpApprovalPolicy::Allow | McpApprovalPolicy::Ask => {
                    Ok(PermissionRequirement::ReviewRequired(mcp_approval_action(
                        &self.lease,
                        &invocation.arguments,
                    )?))
                }
                McpApprovalPolicy::Deny => Ok(PermissionRequirement::Forbidden {
                    reason: "MCP server policy denies this tool".to_string(),
                }),
            }
        })
    }

    fn invoke(
        &self,
        invocation: ToolInvocation,
        approval: Option<AuthorizedAction>,
    ) -> ToolFuture<'_, Result<ToolResult, AgentToolError>> {
        Box::pin(async move {
            if invocation.cancellation.is_cancelled() {
                return Err(AgentToolError::Cancelled);
            }
            let lease = &self.lease;
            match lease.approval_policy {
                McpApprovalPolicy::Deny => {
                    return Err(AgentToolError::Failed(
                        "MCP server policy denies this tool".to_string(),
                    ));
                }
                McpApprovalPolicy::Allow if !lease.binding.hints.destructive => {}
                McpApprovalPolicy::Allow | McpApprovalPolicy::Ask => {
                    validate_mcp_approval(lease, &invocation.arguments, approval.as_ref())?;
                }
            }
            let arguments = invocation.arguments.as_object().cloned().ok_or_else(|| {
                AgentToolError::Failed("MCP tool arguments must be an object".to_string())
            })?;
            let context = &invocation.context;
            let mcp_context = hanafubuki_agent_mcp::McpCallContext {
                session_id: context.session_id.to_string(),
                turn_id: context.turn_id.value(),
                call_id: context.call_id.clone(),
                tool_generation: hex_digest(&lease.binding.tool_generation.0),
            };
            // Each server is its own fault owner: a panic in its call fails
            // that call and that server, which the manager reconnects; it
            // never reaches the kernel, which would stop the whole plugin.
            let called = catch_panic_async(lease.call(
                arguments,
                mcp_context,
                invocation.cancellation.clone(),
            ))
            .await;
            let output = match called {
                Ok(result) => result.map_err(|error| {
                    if error == hanafubuki_agent_mcp::McpError::Cancelled {
                        AgentToolError::Cancelled
                    } else {
                        AgentToolError::Failed(error.to_string())
                    }
                })?,
                Err(message) => {
                    self.mcp.fail_after_panic(lease, message);
                    return Err(AgentToolError::Failed(
                        "the tool failed with an internal error".to_string(),
                    ));
                }
            };
            Ok(ToolResult::from(output))
        })
    }
}

/// The consumed approval must be the one for this exact call.
fn validate_mcp_approval(
    lease: &McpToolLease,
    arguments: &Value,
    approval: Option<&AuthorizedAction>,
) -> Result<(), AgentToolError> {
    let expected = mcp_approval_action(lease, arguments)?;
    let Some(approval) = approval else {
        return Err(AgentToolError::Failed(
            "MCP tool approval is required".to_string(),
        ));
    };
    if approval.action() != &expected {
        return Err(AgentToolError::Failed(
            "MCP tool approval does not match the frozen invocation".to_string(),
        ));
    }
    Ok(())
}

fn mcp_approval_action(
    lease: &McpToolLease,
    arguments: &Value,
) -> Result<ApprovalAction, AgentToolError> {
    let binding = &lease.binding;
    let canonical = serde_json::to_vec(&(binding, arguments))
        .map_err(|error| AgentToolError::Failed(error.to_string()))?;
    let arguments_bytes =
        serde_json::to_vec(arguments).map_err(|error| AgentToolError::Failed(error.to_string()))?;
    let arguments_digest = hex_digest(Sha256::digest(arguments_bytes).as_slice());
    ApprovalAction::from_canonical_bytes(
        ApprovalActionKind::McpToolCall,
        if binding.hints.destructive {
            ApprovalRiskClass::Critical
        } else {
            ApprovalRiskClass::High
        },
        ApprovalOrigin::Model,
        format!(
            "Call MCP tool {} on {}",
            binding.tool_id.as_str(),
            binding.server_id.as_str()
        ),
        &canonical,
    )
    .and_then(|action| {
        action.with_details(PermissionActionDetails::McpTool {
            server_id: binding.server_id.as_str().to_string(),
            connection_generation: hex_digest(&binding.connection_generation.0),
            catalog_generation: hex_digest(&binding.catalog_generation.0),
            tool_id: binding.tool_id.as_str().to_string(),
            tool_generation: hex_digest(&binding.tool_generation.0),
            arguments_digest,
        })
    })
    .map_err(|error| AgentToolError::Failed(error.to_string()))
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
