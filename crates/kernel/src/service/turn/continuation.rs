use super::*;

impl AgentService {
    pub(super) async fn run_continuation_hooks(
        &self,
        identity: TurnIdentity,
        candidate_final_answer: String,
        hooks: ContinuationHooks,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Vec<ConversationItem>, AgentError> {
        if hooks.is_empty() {
            return Ok(Vec::new());
        }
        let background = self.background_work_waking(identity.session_id)?;
        let (context, timeout) = {
            let cell = self.session(identity.session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            let turn = exact_turn(record, identity.turn_id, identity.generation)?;
            (
                AgentContinuationContext {
                    session_id: identity.session_id,
                    turn_id: identity.turn_id,
                    generation: identity.generation,
                    revision: AgentRevision::new(record.status.revision),
                    sampling_step: turn.sampling_step,
                    candidate_final_answer,
                    background,
                },
                record.settings.budget_policy.tool_idle_timeout(),
            )
        };
        let mut items = Vec::new();
        for entry in hooks {
            self.check_cancelled(identity)?;
            // A hook with nothing to decide is neither called nor reported;
            // one whose `applies` panics does not apply (and faults its
            // plugin).
            if !entry
                .guard(
                    hanafubuki_agent_contracts::ExtensionPoint::ContinuationHook,
                    |registration| registration.hook.applies(&context),
                )
                .unwrap_or(false)
            {
                continue;
            }
            // A hook whose plugin is stopping is no longer registered; one
            // that stops while it runs is cancelled through this token.
            let hook_cancellation = cancellation.child();
            let Some(_lease) = entry.acquire(&hook_cancellation) else {
                continue;
            };
            let registration = entry.item();
            self.emit(
                identity,
                AgentEventKind::ContinuationHookStarted {
                    provider_id: registration.provider_id.as_str().to_string(),
                    hook_id: registration.hook_id.as_str().to_string(),
                },
                event_sink,
            )
            .await?;
            // A panicking hook fails like a hook that returned an error: its
            // failure policy decides.
            let evaluated = tokio::time::timeout(
                timeout,
                entry.guard_async(
                    hanafubuki_agent_contracts::ExtensionPoint::ContinuationHook,
                    |registration| {
                        registration
                            .hook
                            .evaluate(context.clone(), hook_cancellation)
                    },
                ),
            )
            .await
            .map(|evaluated| {
                evaluated.unwrap_or_else(|_| {
                    Err(AgentContinuationHookError::Failed(
                        "the hook failed with an internal error".to_string(),
                    ))
                })
            });
            let decision = match evaluated {
                Ok(Ok(decision)) => decision,
                Ok(Err(AgentContinuationHookError::Cancelled)) if cancellation.is_cancelled() => {
                    return Err(AgentError::Cancelled);
                }
                Ok(Err(AgentContinuationHookError::Cancelled)) => {
                    // Its plugin stopped: the hook no longer has a say.
                    self.emit_continuation_hook_completed(
                        identity,
                        registration,
                        AgentContinuationHookOutcome::IgnoredFailure,
                        event_sink,
                    )
                    .await?;
                    continue;
                }
                Ok(Err(AgentContinuationHookError::Failed(message))) => {
                    if registration.failure_policy == AgentContinuationHookFailurePolicy::Ignore {
                        self.emit_continuation_hook_completed(
                            identity,
                            registration,
                            AgentContinuationHookOutcome::IgnoredFailure,
                            event_sink,
                        )
                        .await?;
                        log::warn!(
                            "agent continuation hook failure ignored: provider={} hook={} error={}",
                            registration.provider_id.as_str(),
                            registration.hook_id.as_str(),
                            bounded_diagnostic(message)
                        );
                        continue;
                    }
                    self.emit_continuation_hook_completed(
                        identity,
                        registration,
                        AgentContinuationHookOutcome::Failed,
                        event_sink,
                    )
                    .await?;
                    return Err(AgentError::Protocol(format!(
                        "continuation hook failed: {}",
                        bounded_diagnostic(message)
                    )));
                }
                Err(_) => {
                    if registration.failure_policy == AgentContinuationHookFailurePolicy::Ignore {
                        self.emit_continuation_hook_completed(
                            identity,
                            registration,
                            AgentContinuationHookOutcome::IgnoredFailure,
                            event_sink,
                        )
                        .await?;
                        continue;
                    }
                    self.emit_continuation_hook_completed(
                        identity,
                        registration,
                        AgentContinuationHookOutcome::Failed,
                        event_sink,
                    )
                    .await?;
                    return Err(AgentError::Protocol(
                        "continuation hook timed out".to_string(),
                    ));
                }
            };
            match decision {
                AgentContinuationHookDecision::Pass => {
                    self.emit_continuation_hook_completed(
                        identity,
                        registration,
                        AgentContinuationHookOutcome::Passed,
                        event_sink,
                    )
                    .await?;
                }
                AgentContinuationHookDecision::Stop => {
                    self.emit_continuation_hook_completed(
                        identity,
                        registration,
                        AgentContinuationHookOutcome::Stopped,
                        event_sink,
                    )
                    .await?;
                    return Ok(Vec::new());
                }
                AgentContinuationHookDecision::Continue { fragments } => {
                    if fragments.is_empty()
                        || fragments.len() > MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK
                    {
                        return Err(AgentError::Protocol(
                            "continuation hook returned an invalid fragment count".to_string(),
                        ));
                    }
                    for content in fragments {
                        if content.trim().is_empty()
                            || content.len() > MAXIMUM_CONTINUATION_FRAGMENT_BYTES
                        {
                            return Err(AgentError::Protocol(
                                "continuation hook returned an invalid fragment".to_string(),
                            ));
                        }
                        items.push(ConversationItem::ContinuationContext {
                            provider_id: registration.provider_id.clone(),
                            hook_id: registration.hook_id.clone(),
                            generation: registration.generation,
                            content,
                        });
                    }
                    self.emit_continuation_hook_completed(
                        identity,
                        registration,
                        AgentContinuationHookOutcome::Continued,
                        event_sink,
                    )
                    .await?;
                }
            }
        }
        Ok(items)
    }

    pub(super) async fn emit_continuation_hook_completed(
        &self,
        identity: TurnIdentity,
        registration: &AgentContinuationHookRegistration,
        outcome: AgentContinuationHookOutcome,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.emit(
            identity,
            AgentEventKind::ContinuationHookCompleted {
                provider_id: registration.provider_id.as_str().to_string(),
                hook_id: registration.hook_id.as_str().to_string(),
                outcome,
            },
            event_sink,
        )
        .await
    }
}
