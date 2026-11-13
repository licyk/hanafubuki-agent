use hanafubuki_agent_contracts::{
    ExtensionPoint, ToolCallContext, ToolEffect, ToolInvocation, ToolResult,
};

use super::*;
use crate::service::interaction::ApprovalRoute;

impl AgentService {
    /// Runs a plugin's handler (registered, or planned by a tool source)
    /// through the kernel's fixed pipeline, under the step's gate: mode
    /// policy, the arguments against the schema of a sourced tool, the
    /// handler's permission classification (within the plugin call
    /// deadline), approval when review is required, then the call itself, bounded by the tool's timeout, under a
    /// lease on its registry entry (the source's entry for a sourced tool).
    /// A wait for the reviewer or the user lets go of the gate, so the
    /// step's other calls proceed meanwhile, and the call takes it again
    /// once approved.
    pub(super) async fn execute_handler_tool_call(
        &self,
        identity: TurnIdentity,
        call: &ModelToolCall,
        registration: &crate::AgentPlannedTool,
        runtime: &crate::ToolCallRuntime,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentToolOutput, AgentError> {
        let binding = registration
            .handler
            .as_ref()
            .ok_or_else(|| AgentError::Protocol("planned handler tool has no handler".into()))?;
        let handler = binding.handler().clone();
        let mut admission = enter_gate(runtime, registration, &cancellation).await?;
        let context = self
            .planned_tool_context(
                identity,
                call,
                runtime.catalog_revision(),
                registration,
                runtime.catalog(),
            )
            .await?;
        if context.mode == crate::AgentSessionMode::Plan
            && handler.descriptor().effect != ToolEffect::ReadOnly
        {
            return Ok(AgentToolOutput::model_visible_error(json!({
                "error": "tool is unavailable in the current Agent mode"
            })));
        }
        // A registered handler validates its own arguments; a sourced tool's
        // schema comes from outside the application, so the kernel checks the
        // arguments against it first.
        if binding.is_sourced() {
            if let Err(error) = crate::validate_dynamic_tool_arguments(
                &registration.model_spec.input_schema,
                &call.arguments,
            ) {
                return Ok(AgentToolOutput::model_visible_error(json!({
                    "error": bounded_diagnostic(error.to_string())
                })));
            }
        }
        let turn = self.turn_extensions(identity)?;
        let call_cancellation = cancellation.child();
        let Some(_lease) = binding.acquire(&call_cancellation) else {
            return Ok(AgentToolOutput::model_visible_error(json!({
                "error": "tool is no longer available"
            })));
        };
        let invocation = ToolInvocation {
            context: ToolCallContext {
                session_id: context.session_id,
                turn_id: context.turn_id,
                generation: context.generation,
                mode: context.mode,
                session_revision: context.session_revision,
                call_id: context.call_id.clone(),
                image_input: context.input_capabilities.image_input,
                origin: context.origin,
            },
            arguments: call.arguments.clone(),
            cancellation: call_cancellation.clone(),
            progress: context.progress.clone(),
            plan_leases: context.tool_plan_leases.clone(),
            turn,
        };
        let deadline = self.inner.limits.plugin_call_deadline;
        let Ok(requirement) = tokio::time::timeout(
            deadline,
            binding.guard_async(ExtensionPoint::ToolPermission, |handler| {
                handler.permission(&invocation)
            }),
        )
        .await
        else {
            call_cancellation.cancel();
            if cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            return handler_failure(
                AgentToolError::Failed(format!(
                    "the tool's permission check did not finish within {} s and was stopped",
                    deadline.as_secs_f64()
                )),
                &cancellation,
            );
        };
        let requirement = match requirement.unwrap_or_else(|_| Err(internal_tool_error())) {
            Ok(requirement) => requirement,
            Err(error) => return handler_failure(error, &cancellation),
        };
        let authorization = match requirement {
            hanafubuki_agent_approval::PermissionRequirement::NotRequired => None,
            hanafubuki_agent_approval::PermissionRequirement::Forbidden { reason } => {
                return Ok(AgentToolOutput::model_visible_error(json!({
                    "error": bounded_diagnostic(reason)
                })));
            }
            hanafubuki_agent_approval::PermissionRequirement::ReviewRequired(action) => {
                let disposition = match self.route_approval(&context, action, &cancellation)? {
                    ApprovalRoute::Decided(disposition) => disposition,
                    ApprovalRoute::Waiting(pending) => {
                        drop(admission);
                        let disposition = self
                            .await_approval(
                                *pending,
                                runtime.approval_prompts(),
                                cancellation.clone(),
                                event_sink,
                            )
                            .await?;
                        admission = enter_gate(runtime, registration, &cancellation).await?;
                        disposition
                    }
                };
                match disposition {
                    ToolApprovalDisposition::Approved(approval) => Some(*approval),
                    ToolApprovalDisposition::Rejected(message) => {
                        return Ok(AgentToolOutput::model_visible_error(json!({
                            "error": message
                        })));
                    }
                }
            }
        };
        let timeout = match handler.descriptor().timeout {
            Some(timeout) => timeout,
            None => self.turn_budget_policy(identity)?.tool_idle_timeout(),
        };
        let invoked = tokio::time::timeout(
            timeout,
            binding.guard_async(ExtensionPoint::ToolInvoke, |handler| {
                handler.invoke(invocation, authorization)
            }),
        )
        .await;
        drop(admission);
        let Ok(invoked) = invoked else {
            call_cancellation.cancel();
            if cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            return Ok(timed_out_output(timeout));
        };
        match invoked.unwrap_or_else(|_| Err(internal_tool_error())) {
            Ok(result) => Ok(handler_output(result)),
            Err(error) => handler_failure(error, &cancellation),
        }
    }
}

/// Admits the call through the step's gate. A call admitted after its
/// turn was cancelled does not start.
async fn enter_gate(
    runtime: &crate::ToolCallRuntime,
    registration: &crate::AgentPlannedTool,
    cancellation: &CancellationToken,
) -> Result<crate::AgentToolExecutionPermit, AgentError> {
    let admission = runtime
        .enter(registration.execution_class)
        .await
        .map_err(|error| AgentError::Protocol(error.to_string()))?;
    if cancellation.is_cancelled() {
        return Err(AgentError::Cancelled);
    }
    Ok(admission)
}

/// What the model reads when a call ran past its timeout.
fn timed_out_output(timeout: Duration) -> AgentToolOutput {
    AgentToolOutput::model_visible_error(json!({
        "error": format!(
            "the tool did not finish within {} s and was stopped",
            timeout.as_secs_f64()
        )
    }))
}

/// What the model reads when a handler panicked: a tool error like any
/// failing tool's. The panic itself reaches only the log and the plugin's
/// status.
fn internal_tool_error() -> AgentToolError {
    AgentToolError::Failed("the tool failed with an internal error".to_string())
}

/// A handler's result as the kernel commits it. A context fragment becomes
/// the context of the Skill it names; one naming no valid Skill fails the
/// call instead of reaching history.
fn handler_output(result: ToolResult) -> AgentToolOutput {
    let skill_context = match result.context {
        None => None,
        Some(fragment) => match hanafubuki_agent_skills::SkillId::from_untrusted(fragment.source) {
            Ok(skill) => Some(crate::ActivatedSkillContext {
                skill,
                name: fragment.name,
                content: fragment.content,
            }),
            Err(_) => {
                return AgentToolOutput::model_visible_error(json!({
                    "error": "tool returned context for an invalid Skill id"
                }));
            }
        },
    };
    let mut output = AgentToolOutput::typed(result.output);
    output.prepared_images = result.images;
    output.skill_context = skill_context;
    output
}

/// A cancelled call ends the turn only when the turn itself was cancelled; a
/// call cancelled because its plugin stopped is a model-visible tool error.
fn handler_failure(
    error: AgentToolError,
    turn_cancellation: &CancellationToken,
) -> Result<AgentToolOutput, AgentError> {
    match error {
        AgentToolError::Cancelled if turn_cancellation.is_cancelled() => Err(AgentError::Cancelled),
        AgentToolError::Cancelled => Ok(AgentToolOutput::model_visible_error(json!({
            "error": "tool was stopped because its plugin stopped"
        }))),
        AgentToolError::Failed(message) => Ok(AgentToolOutput::model_visible_error(
            json!({ "error": bounded_diagnostic(message) }),
        )),
    }
}
