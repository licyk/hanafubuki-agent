//! PreToolUse and PostToolUse: one tool lifecycle hook that runs the hooks
//! configured for a call's tool.

use std::sync::Arc;

use hanafubuki_agent_contracts::{
    CancellationToken, PostToolUseDecision, PostToolUseFuture, PreToolUseDecision,
    PreToolUseFuture, ToolLifecycleCall, ToolLifecycleHook, ToolLifecycleHookError,
    ToolLifecyclePhase, MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK,
    MAXIMUM_CONTINUATION_FRAGMENT_BYTES,
};
use hanafubuki_agent_hooks::{tool_matcher_inputs, HookEvent, HookEventName};
use hanafubuki_agent_tool_contracts::ToolOutput;

use crate::runtime::{bounded, HooksRuntime};

pub(crate) struct ToolUseHooks(pub(crate) Arc<HooksRuntime>);

impl ToolLifecycleHook for ToolUseHooks {
    fn applies(&self, phase: ToolLifecyclePhase, call: &ToolLifecycleCall) -> bool {
        let event = match phase {
            ToolLifecyclePhase::PreToolUse => HookEventName::PreToolUse,
            ToolLifecyclePhase::PostToolUse => HookEventName::PostToolUse,
        };
        self.0
            .has_runnable(event, &tool_matcher_inputs(&call.tool_name))
    }

    fn before_tool_use<'a>(
        &'a self,
        call: ToolLifecycleCall,
        cancellation: CancellationToken,
    ) -> PreToolUseFuture<'a> {
        Box::pin(async move {
            let event = HookEvent::PreToolUse {
                tool_name: call.tool_name,
                tool_input: call.arguments,
                tool_use_id: call.call_id,
            };
            let outcome = self
                .0
                .run(event, call.session_id, Some(call.turn_id), cancellation)
                .await
                .map_err(ToolLifecycleHookError::Failed)?;
            match outcome.deny {
                Some(reason) => Ok(PreToolUseDecision::Deny {
                    reason: format!("a PreToolUse hook denied the call: {reason}"),
                }),
                None => no_failure(&outcome.failures).map(|()| PreToolUseDecision::Allow),
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
            let event = HookEvent::PostToolUse {
                tool_name: call.tool_name,
                tool_input: call.arguments,
                tool_response: serde_json::to_value(&output).unwrap_or_default(),
                tool_use_id: call.call_id,
            };
            let outcome = self
                .0
                .run(event, call.session_id, Some(call.turn_id), cancellation)
                .await
                .map_err(ToolLifecycleHookError::Failed)?;
            let fragments = fragments(outcome.context);
            if fragments.is_empty() {
                no_failure(&outcome.failures).map(|()| PostToolUseDecision::Pass)
            } else {
                Ok(PostToolUseDecision::AddContext { fragments })
            }
        })
    }
}

/// Context as the kernel takes it: non-blank, each within the fragment
/// bound, at most the fragments one hook may add.
pub(crate) fn fragments(context: Vec<String>) -> Vec<String> {
    context
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .take(MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK)
        .map(|text| bounded(text, MAXIMUM_CONTINUATION_FRAGMENT_BYTES))
        .collect()
}

/// A run of hooks that asked for nothing passes, unless one of them failed:
/// the kernel then reports the hook as failed (and the call goes on).
fn no_failure(failures: &[(String, String)]) -> Result<(), ToolLifecycleHookError> {
    match failures {
        [] => Ok(()),
        failures => Err(ToolLifecycleHookError::Failed(
            failures
                .iter()
                .map(|(key, why)| format!("{key}: {why}"))
                .collect::<Vec<_>>()
                .join("; "),
        )),
    }
}
