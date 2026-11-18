use super::AdvisorService;
use hanafubuki_agent_contracts::{
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    CancellationToken, PostToolUseDecision, PostToolUseFuture, PreToolUseDecision,
    PreToolUseFuture, Register, ToolLifecycleCall, ToolLifecycleHook,
    ToolLifecycleHookRegistration, ToolLifecycleHookRegistry, ToolLifecyclePhase,
};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Failure {
    #[default]
    Warn,
    BlockTool,
    BlockSession,
}
#[derive(Default)]
pub(super) struct Repeated {
    hash: [u8; 32],
    count: usize,
    blocked: Option<String>,
    warning: Option<String>,
}

struct Gate(Arc<AdvisorService>);
pub(super) fn register(
    ctx: &kardia_core::Context,
    service: Arc<AdvisorService>,
) -> kardia_core::Result<()> {
    ctx.get::<ToolLifecycleHookRegistry>()?
        .register(ToolLifecycleHookRegistration {
            provider_id: AgentContinuationHookProviderId::parse("advisor".into())
                .map_err(kardia_core::Error::msg)?,
            hook_id: AgentContinuationHookId::parse("repeat".into())
                .map_err(kardia_core::Error::msg)?,
            generation: AgentContinuationHookGeneration(Sha256::digest(b"advisor/repeat/1").into()),
            priority: 0,
            hook: Arc::new(Gate(service)),
        })?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Decision {
    Proceed,
    Revise,
    Blocked,
}
#[derive(Deserialize)]
struct Verdict {
    decision: Decision,
    reason: String,
}
impl ToolLifecycleHook for Gate {
    fn applies(&self, phase: ToolLifecyclePhase, call: &ToolLifecycleCall) -> bool {
        let opts = self.0.options.read().clone();
        !opts.simple
            && opts.repeat_threshold.is_some()
            && self.0.active(call.session_id)
            && !matches!(
                call.tool_name.as_str(),
                "ask_advisor" | "record_advisor_outcome" | "advisor_config"
            )
            && (phase == ToolLifecyclePhase::PreToolUse
                || self
                    .0
                    .repeats
                    .lock()
                    .get(&call.session_id)
                    .is_some_and(|r| r.warning.is_some()))
    }
    fn before_tool_use<'a>(
        &'a self,
        call: ToolLifecycleCall,
        cancel: CancellationToken,
    ) -> PreToolUseFuture<'a> {
        Box::pin(async move {
            let opts = self.0.options.read().clone();
            let Some(limit) = opts.repeat_threshold else {
                return Ok(PreToolUseDecision::Allow);
            };
            let id = call.session_id;
            {
                let mut repeats = self.0.repeats.lock();
                let state = repeats.entry(id).or_default();
                if let Some(reason) = &state.blocked {
                    return Ok(PreToolUseDecision::Deny {
                        reason: reason.clone(),
                    });
                }
                let hash: [u8; 32] = Sha256::digest(
                    serde_json::json!([call.tool_name, call.arguments])
                        .to_string()
                        .as_bytes(),
                )
                .into();
                state.count = if state.hash == hash {
                    state.count.saturating_add(1)
                } else {
                    1
                };
                state.hash = hash;
                if state.count < limit {
                    return Ok(PreToolUseDecision::Allow);
                }
                state.count = 0;
            }
            let question=format!("The executor proposes repeating tool {} with identical arguments. Polling can produce new results. Review the available evidence and return only JSON with decision (proceed, revise or blocked) and a short reason.",call.tool_name);
            let answer = self
                .0
                .consult(
                    id,
                    super::Consultation {
                        question,
                        trigger: super::AdvisorTrigger::RepeatedToolCall,
                        ..Default::default()
                    },
                    cancel.clone(),
                )
                .await;
            if cancel.is_cancelled() {
                return Err(hanafubuki_agent_contracts::ToolLifecycleHookError::Cancelled);
            }
            let verdict = answer.map_err(|e| e.to_string()).and_then(|a| {
                serde_json::from_str::<Verdict>(a.text.trim())
                    .map_err(|e| format!("Invalid Advisor verdict: {e}"))
            });
            let reason = match verdict {
                Ok(Verdict {
                    decision: Decision::Proceed,
                    ..
                }) => return Ok(PreToolUseDecision::Allow),
                Ok(Verdict {
                    decision: Decision::Revise,
                    reason,
                }) => return Ok(PreToolUseDecision::Deny { reason }),
                Ok(Verdict { reason, .. }) | Err(reason) => reason,
            };
            let mut states = self.0.repeats.lock();
            let Some(state) = states.get_mut(&id) else {
                return Ok(PreToolUseDecision::Allow);
            };
            match opts.gate_failure {
                Failure::Warn => {
                    state.warning = Some(reason);
                    Ok(PreToolUseDecision::Allow)
                }
                Failure::BlockTool => Ok(PreToolUseDecision::Deny { reason }),
                Failure::BlockSession => {
                    let reason = format!("{reason} (Use /advisor-off to clear the session block.)");
                    state.blocked = Some(reason.clone());
                    Ok(PreToolUseDecision::Deny { reason })
                }
            }
        })
    }
    fn after_tool_use<'a>(
        &'a self,
        call: ToolLifecycleCall,
        _: ToolOutput,
        _: CancellationToken,
    ) -> PostToolUseFuture<'a> {
        Box::pin(async move {
            Ok(self
                .0
                .repeats
                .lock()
                .get_mut(&call.session_id)
                .and_then(|s| s.warning.take())
                .map(|s| PostToolUseDecision::AddContext {
                    fragments: vec![format!("Advisor review warning: {s}")],
                })
                .unwrap_or(PostToolUseDecision::Pass))
        })
    }
}
