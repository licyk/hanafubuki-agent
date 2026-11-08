use super::TurnIdentity;
use crate::tool_runtime::AgentToolOutput;
use hanafubuki_agent_model_api::ModelToolCall;

pub(in crate::service) use hanafubuki_agent_model_api::bounded_trace_content;

/// Record the model-visible tool call (name and parsed arguments) at `trace`
/// level only. Approval-sensitive fields are never emitted here.
pub(in crate::service) fn trace_tool_call(identity: TurnIdentity, call: &ModelToolCall) {
    if !log::log_enabled!(log::Level::Trace) {
        return;
    }
    let arguments = call.arguments.to_string();
    log::trace!(
        "agent_diag stage=tool.call.content session_id={} turn_id={} generation={} name={} arguments={}",
        identity.session_id,
        identity.turn_id,
        identity.generation,
        call.name,
        bounded_trace_content(&arguments)
    );
}

/// Record the committed tool result content (the JSON that becomes the model's
/// next input) at `trace` level only.
pub(in crate::service) fn trace_tool_result(identity: TurnIdentity, output: &AgentToolOutput) {
    if !log::log_enabled!(log::Level::Trace) {
        return;
    }
    let content = serde_json::to_string(&output.output).unwrap_or_default();
    log::trace!(
        "agent_diag stage=tool.result.content session_id={} turn_id={} generation={} is_error={} output={}",
        identity.session_id,
        identity.turn_id,
        identity.generation,
        output.is_error(),
        bounded_trace_content(&content)
    );
}
