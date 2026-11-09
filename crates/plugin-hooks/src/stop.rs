//! Stop: a continuation hook that runs the Stop hooks when the model gives
//! a final answer. A hook that blocks keeps the turn going with its reason;
//! one that sets `continue: false` ends it without the remaining
//! continuation hooks.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    AgentContinuationContext, AgentContinuationHook, AgentContinuationHookDecision,
    AgentContinuationHookError, AgentContinuationHookFuture, CancellationToken,
};
use hanafubuki_agent_hooks::{HookEvent, HookEventName};

use crate::runtime::HooksRuntime;
use crate::tool_use::fragments;

pub(crate) struct StopHooks(pub(crate) Arc<HooksRuntime>);

impl AgentContinuationHook for StopHooks {
    fn applies(&self, _context: &AgentContinuationContext) -> bool {
        self.0.has_runnable(HookEventName::Stop, &[])
    }

    fn evaluate<'a>(
        &'a self,
        context: AgentContinuationContext,
        cancellation: CancellationToken,
    ) -> AgentContinuationHookFuture<'a> {
        Box::pin(async move {
            let stop_hook_active =
                self.0.continued.lock().get(&context.session_id) == Some(&context.turn_id);
            let event = HookEvent::Stop {
                stop_hook_active,
                last_assistant_message: context.candidate_final_answer,
            };
            let outcome = self
                .0
                .run(
                    event,
                    context.session_id,
                    Some(context.turn_id),
                    cancellation,
                )
                .await
                .map_err(AgentContinuationHookError::Failed)?;
            if outcome.stop {
                return Ok(AgentContinuationHookDecision::Stop);
            }
            let fragments = fragments(outcome.continue_with);
            if fragments.is_empty() {
                return Ok(AgentContinuationHookDecision::Pass);
            }
            self.0
                .continued
                .lock()
                .insert(context.session_id, context.turn_id);
            Ok(AgentContinuationHookDecision::Continue { fragments })
        })
    }
}
