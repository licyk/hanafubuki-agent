use super::*;

mod approval;
pub(in crate::service) use approval::{ApprovalRoute, SessionAllowance};
mod plans;
mod review;
mod user_input;

fn approval_context(
    tool_context: &AgentToolInvocationContext,
) -> Result<ApprovalContext, AgentError> {
    let identity = TurnIdentity {
        session_id: tool_context.session_id,
        turn_id: tool_context.turn_id,
        generation: tool_context.generation,
    };
    let call_id = tool_context.call_id.as_str();
    let canonical = serde_json::to_vec(&(
        "agent-turn-approval-v2",
        identity.session_id.value(),
        identity.turn_id.value(),
        identity.generation.value(),
        call_id,
        &tool_context.catalog_revision.0,
        tool_context.tool_id.as_str(),
        &tool_context.tool_generation.0,
    ))
    .map_err(|error| AgentError::Approval(error.to_string()))?;
    ApprovalContext::from_canonical_bytes(call_id.to_string(), &canonical)
        .and_then(|context| {
            context.with_scope(ApprovalContextScope {
                authority: "agent".to_string(),
                session_id: identity.session_id.value().to_string(),
                operation_group_id: identity.turn_id.value().to_string(),
                generation: identity.generation.value(),
            })
        })
        .map_err(map_approval_error)
}

fn map_approval_error(error: ApprovalError) -> AgentError {
    match error {
        ApprovalError::RequestNotFound => AgentError::ApprovalNotFound,
        ApprovalError::StaleContext => AgentError::StaleApproval,
        other => AgentError::Approval(other.to_string()),
    }
}
