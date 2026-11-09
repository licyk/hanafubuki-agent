//! The tool lifecycle hooks around one call: before it, the hooks that
//! apply may deny it; after it, they may add context, which the step
//! commits after all of its outputs (a fragment between two outputs would
//! split a protocol's batch of tool results).

use hanafubuki_agent_contracts::{
    ExtensionPoint, PostToolUseDecision, PreToolUseDecision, ToolLifecycleCall,
    ToolLifecycleHookError, ToolLifecycleHookRegistration, ToolLifecyclePhase,
    MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK, MAXIMUM_CONTINUATION_FRAGMENT_BYTES,
};

use super::*;
use crate::tool_catalog::ToolLifecycleHookEntry;
use crate::{AgentHookEvent, AgentHookOutcome};

impl AgentService {
    /// Asks the hooks that apply before `call`, in order, and returns the
    /// first denial's reason. A hook that fails or times out has no say.
    pub(super) async fn run_pre_tool_use_hooks(
        &self,
        identity: TurnIdentity,
        call: &ToolLifecycleCall,
        hooks: &[ToolLifecycleHookEntry],
        cancellation: &CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<String>, AgentError> {
        for entry in applying(hooks, ToolLifecyclePhase::PreToolUse, call) {
            let ran = self
                .run_hook(
                    identity,
                    entry,
                    call,
                    AgentHookEvent::PreToolUse,
                    cancellation,
                    event_sink,
                    |registration, hook_cancellation| {
                        registration
                            .hook
                            .before_tool_use(call.clone(), hook_cancellation)
                    },
                )
                .await?;
            let (outcome, denial) = match ran {
                Ok(PreToolUseDecision::Allow) => (AgentHookOutcome::Passed, None),
                Ok(PreToolUseDecision::Deny { reason }) if !reason.trim().is_empty() => {
                    (AgentHookOutcome::Denied, Some(bounded_diagnostic(reason)))
                }
                Ok(PreToolUseDecision::Deny { .. }) => (AgentHookOutcome::Failed, None),
                Err(outcome) => (outcome, None),
            };
            self.emit_hook_completed(
                identity,
                entry.item(),
                call,
                AgentHookEvent::PreToolUse,
                outcome,
                event_sink,
            )
            .await?;
            if denial.is_some() {
                return Ok(denial);
            }
        }
        Ok(None)
    }

    /// Asks the hooks that apply after `call` produced `output`, in order,
    /// and returns the context they add, as the items to commit after the
    /// step's outputs. A hook that fails, times out or returns fragments out
    /// of bounds adds nothing.
    pub(super) async fn run_post_tool_use_hooks(
        &self,
        identity: TurnIdentity,
        call: &ToolLifecycleCall,
        output: &hanafubuki_agent_tool_contracts::ToolOutput,
        hooks: &[ToolLifecycleHookEntry],
        cancellation: &CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Vec<ConversationItem>, AgentError> {
        let mut items = Vec::new();
        for entry in applying(hooks, ToolLifecyclePhase::PostToolUse, call) {
            let ran = self
                .run_hook(
                    identity,
                    entry,
                    call,
                    AgentHookEvent::PostToolUse,
                    cancellation,
                    event_sink,
                    |registration, hook_cancellation| {
                        registration.hook.after_tool_use(
                            call.clone(),
                            output.clone(),
                            hook_cancellation,
                        )
                    },
                )
                .await?;
            let registration = entry.item();
            let outcome = match ran {
                Ok(PostToolUseDecision::Pass) => AgentHookOutcome::Passed,
                Ok(PostToolUseDecision::AddContext { fragments }) if valid(&fragments) => {
                    items.extend(fragments.into_iter().map(|content| {
                        ConversationItem::ContinuationContext {
                            provider_id: registration.provider_id.clone(),
                            hook_id: registration.hook_id.clone(),
                            generation: registration.generation,
                            content,
                        }
                    }));
                    AgentHookOutcome::AddedContext
                }
                Ok(PostToolUseDecision::AddContext { .. }) => {
                    log::warn!(
                        "agent tool lifecycle hook returned invalid fragments: provider={} hook={}",
                        registration.provider_id.as_str(),
                        registration.hook_id.as_str()
                    );
                    AgentHookOutcome::Failed
                }
                Err(outcome) => outcome,
            };
            self.emit_hook_completed(
                identity,
                registration,
                call,
                AgentHookEvent::PostToolUse,
                outcome,
                event_sink,
            )
            .await?;
        }
        Ok(items)
    }

    /// One hook call under a lease on its entry, a child of the turn's
    /// cancellation and the turn's tool idle timeout, after its start
    /// event. `Err` is the outcome of a call that had no say; the turn's
    /// own cancellation ends the turn instead.
    #[allow(
        clippy::too_many_arguments,
        reason = "one hook call needs its turn, entry, call, event, cancellation, sink and body"
    )]
    async fn run_hook<'a, T, F, Fut>(
        &self,
        identity: TurnIdentity,
        entry: &'a ToolLifecycleHookEntry,
        call: &ToolLifecycleCall,
        event: AgentHookEvent,
        cancellation: &CancellationToken,
        event_sink: &dyn AgentEventSink,
        body: F,
    ) -> Result<Result<T, AgentHookOutcome>, AgentError>
    where
        F: FnOnce(&'a ToolLifecycleHookRegistration, CancellationToken) -> Fut,
        Fut: std::future::Future<Output = Result<T, ToolLifecycleHookError>>,
    {
        self.check_cancelled(identity)?;
        let hook_cancellation = cancellation.child();
        let Some(_lease) = entry.acquire(&hook_cancellation) else {
            return Ok(Err(AgentHookOutcome::Failed));
        };
        let registration = entry.item();
        self.emit(
            identity,
            AgentEventKind::HookStarted {
                provider_id: registration.provider_id.as_str().to_string(),
                hook_id: registration.hook_id.as_str().to_string(),
                call_id: call.call_id.clone(),
                event,
            },
            event_sink,
        )
        .await?;
        let timeout = self.turn_budget_policy(identity)?.tool_idle_timeout();
        let ran = tokio::time::timeout(
            timeout,
            entry.guard_async(ExtensionPoint::ToolLifecycleHook, |registration| {
                body(registration, hook_cancellation.clone())
            }),
        )
        .await;
        let Ok(ran) = ran else {
            hook_cancellation.cancel();
            return Ok(Err(AgentHookOutcome::TimedOut));
        };
        match ran {
            Ok(Ok(decision)) => Ok(Ok(decision)),
            Ok(Err(ToolLifecycleHookError::Cancelled)) if cancellation.is_cancelled() => {
                Err(AgentError::Cancelled)
            }
            Ok(Err(ToolLifecycleHookError::Failed(message))) => {
                log::warn!(
                    "agent tool lifecycle hook failed: provider={} hook={} error={}",
                    registration.provider_id.as_str(),
                    registration.hook_id.as_str(),
                    bounded_diagnostic(message)
                );
                Ok(Err(AgentHookOutcome::Failed))
            }
            // Its plugin stopped, or it panicked (a fault of its plugin).
            Ok(Err(ToolLifecycleHookError::Cancelled)) | Err(_) => {
                Ok(Err(AgentHookOutcome::Failed))
            }
        }
    }

    async fn emit_hook_completed(
        &self,
        identity: TurnIdentity,
        registration: &ToolLifecycleHookRegistration,
        call: &ToolLifecycleCall,
        event: AgentHookEvent,
        outcome: AgentHookOutcome,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.emit(
            identity,
            AgentEventKind::HookCompleted {
                provider_id: registration.provider_id.as_str().to_string(),
                hook_id: registration.hook_id.as_str().to_string(),
                call_id: call.call_id.clone(),
                event,
                outcome,
            },
            event_sink,
        )
        .await
    }
}

/// The hooks that apply to `call` in `phase`, in order. `applies` is pure;
/// one that panics does not apply (and faults its plugin).
fn applying<'a>(
    hooks: &'a [ToolLifecycleHookEntry],
    phase: ToolLifecyclePhase,
    call: &'a ToolLifecycleCall,
) -> impl Iterator<Item = &'a ToolLifecycleHookEntry> + 'a {
    hooks.iter().filter(move |entry| {
        entry
            .guard(ExtensionPoint::ToolLifecycleHook, |registration| {
                registration.hook.applies(phase, call)
            })
            .unwrap_or(false)
    })
}

fn valid(fragments: &[String]) -> bool {
    !fragments.is_empty()
        && fragments.len() <= MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK
        && fragments.iter().all(|content| {
            !content.trim().is_empty() && content.len() <= MAXIMUM_CONTINUATION_FRAGMENT_BYTES
        })
}
