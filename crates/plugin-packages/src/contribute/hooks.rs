//! A guest's hooks, when its manifest declares them: a tool lifecycle hook
//! (`beforeToolUse`, `afterToolUse`) and a continuation hook
//! (`continuation`), after the user's hooks and the goal's (priority 200).
//! Each call is bounded by the call deadline; a guest that fails or does
//! not answer in time has no say (a continuation hook is `Ignore`), so a
//! guest never fails a turn or stops a tool through a failure.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentContinuationContext, AgentContinuationHook, AgentContinuationHookDecision,
    AgentContinuationHookError, AgentContinuationHookFailurePolicy, AgentContinuationHookFuture,
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    AgentContinuationHookRegistration, CancellationToken, ContinuationHookRegistry,
    PostToolUseDecision, PostToolUseFuture, PreToolUseDecision, PreToolUseFuture,
    ToolLifecycleCall, ToolLifecycleHook, ToolLifecycleHookError, ToolLifecycleHookRegistration,
    ToolLifecycleHookRegistry, ToolLifecyclePhase,
};
use hanafubuki_agent_tool_contracts::ToolOutput;
use kardia_core::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::protocol::{kind, strings};
use super::{provider_id, register_in, GuestCaller, GuestMethod};
use crate::package::GuestPackage;

/// After the user's hooks (0) and the goal's (100).
const PRIORITY: i32 = 200;

/// `entry` is the guest's entry name, which the hooks' generation follows.
pub(super) fn register(
    ctx: &Context,
    guest: &GuestCaller,
    package: &GuestPackage,
    entry: &str,
    issues: &mut Vec<String>,
) {
    let contributes = &package.contributes;
    if !contributes.tool_lifecycle_hooks && !contributes.continuation_hooks {
        return;
    }
    let provider = provider_id(&package.id);
    let ids = (
        AgentContinuationHookProviderId::parse(provider.clone()),
        AgentContinuationHookId::parse("tool-use".to_string()),
        AgentContinuationHookId::parse("continuation".to_string()),
    );
    let (Ok(provider_id), Ok(tool_use), Ok(continuation)) = ids else {
        issues.push(format!(
            "its hooks were not registered: `{provider}` is not a hook provider id"
        ));
        return;
    };
    let generation = AgentContinuationHookGeneration(Sha256::digest(entry.as_bytes()).into());
    let hook = Arc::new(GuestHooks(guest.clone()));
    if contributes.tool_lifecycle_hooks {
        let registration = ToolLifecycleHookRegistration {
            provider_id: provider_id.clone(),
            hook_id: tool_use,
            generation,
            priority: PRIORITY,
            hook: hook.clone(),
        };
        register_in::<ToolLifecycleHookRegistry>(
            ctx,
            registration,
            "its tool lifecycle hook",
            issues,
        );
    }
    if contributes.continuation_hooks {
        let registration = AgentContinuationHookRegistration {
            provider_id,
            hook_id: continuation,
            generation,
            priority: PRIORITY,
            failure_policy: AgentContinuationHookFailurePolicy::Ignore,
            hook,
        };
        register_in::<ContinuationHookRegistry>(ctx, registration, "its continuation hook", issues);
    }
}

struct GuestHooks(GuestCaller);

impl GuestHooks {
    async fn ask(
        &self,
        method: GuestMethod,
        args: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, String> {
        self.0
            .call(method, args, self.0.deadline(), cancellation)
            .await
            .map_err(|error| self.0.failure(method, error))
    }
}

fn call_args(call: &ToolLifecycleCall) -> Value {
    json!({
        "sessionId": call.session_id.value(),
        "turnId": call.turn_id.value(),
        "callId": call.call_id,
        "toolName": call.tool_name,
        "arguments": call.arguments,
    })
}

fn lifecycle_error(cancellation: &CancellationToken, error: String) -> ToolLifecycleHookError {
    if cancellation.is_cancelled() {
        ToolLifecycleHookError::Cancelled
    } else {
        ToolLifecycleHookError::Failed(error)
    }
}

impl ToolLifecycleHook for GuestHooks {
    fn applies(&self, _phase: ToolLifecyclePhase, _call: &ToolLifecycleCall) -> bool {
        self.0.running()
    }

    fn before_tool_use<'a>(
        &'a self,
        call: ToolLifecycleCall,
        cancellation: CancellationToken,
    ) -> PreToolUseFuture<'a> {
        Box::pin(async move {
            let args = json!({ "call": call_args(&call) });
            let answer = self
                .ask(GuestMethod::BeforeToolUse, args, &cancellation)
                .await
                .map_err(|error| lifecycle_error(&cancellation, error))?;
            match answer {
                Value::Null => Ok(PreToolUseDecision::Allow),
                answer => match answer.get("deny").and_then(Value::as_str) {
                    Some(reason) => Ok(PreToolUseDecision::Deny {
                        reason: reason.to_string(),
                    }),
                    None => Err(ToolLifecycleHookError::Failed(format!(
                        "the guest answered {} to `beforeToolUse`, not null or {{\"deny\": …}}",
                        kind(&answer)
                    ))),
                },
            }
        })
    }

    fn after_tool_use<'a>(
        &'a self,
        call: ToolLifecycleCall,
        output: ToolOutput,
        cancellation: CancellationToken,
    ) -> PostToolUseFuture<'a> {
        Box::pin(async move {
            let args = json!({
                "call": call_args(&call),
                "output": serde_json::to_value(&output).unwrap_or_default(),
            });
            let answer = self
                .ask(GuestMethod::AfterToolUse, args, &cancellation)
                .await
                .map_err(|error| lifecycle_error(&cancellation, error))?;
            if answer.is_null() {
                return Ok(PostToolUseDecision::Pass);
            }
            match strings(&answer, "context") {
                Some(fragments) => Ok(PostToolUseDecision::AddContext { fragments }),
                None => Err(ToolLifecycleHookError::Failed(format!(
                    "the guest answered {} to `afterToolUse`, not null or {{\"context\": […]}}",
                    kind(&answer)
                ))),
            }
        })
    }
}

impl AgentContinuationHook for GuestHooks {
    fn applies(&self, _context: &AgentContinuationContext) -> bool {
        self.0.running()
    }

    fn evaluate<'a>(
        &'a self,
        context: AgentContinuationContext,
        cancellation: CancellationToken,
    ) -> AgentContinuationHookFuture<'a> {
        Box::pin(async move {
            let args = json!({
                "sessionId": context.session_id.value(),
                "turnId": context.turn_id.value(),
                "samplingStep": context.sampling_step,
                "candidateFinalAnswer": context.candidate_final_answer,
            });
            let answer = self
                .ask(GuestMethod::Continuation, args, &cancellation)
                .await
                .map_err(|error| match cancellation.is_cancelled() {
                    true => AgentContinuationHookError::Cancelled,
                    false => AgentContinuationHookError::Failed(error),
                })?;
            match &answer {
                Value::Null => Ok(AgentContinuationHookDecision::Pass),
                Value::String(decision) if decision == "pass" => {
                    Ok(AgentContinuationHookDecision::Pass)
                }
                Value::String(decision) if decision == "stop" => {
                    Ok(AgentContinuationHookDecision::Stop)
                }
                answer => match strings(answer, "continue") {
                    Some(fragments) => Ok(AgentContinuationHookDecision::Continue { fragments }),
                    None => Err(AgentContinuationHookError::Failed(format!(
                        "the guest answered {} to `continuation`, not null, \"pass\", \"stop\" \
                         or {{\"continue\": […]}}",
                        kind(answer)
                    ))),
                },
            }
        })
    }
}
