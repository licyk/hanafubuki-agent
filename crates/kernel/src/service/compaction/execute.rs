use super::*;

use super::replacement::{local_summary_replacement, remote_replacement, remote_v2_replacement};
use super::source::canonical_context_digest;
use super::status::compaction_usage;

impl AgentService {
    pub(super) async fn execute_compaction(
        &self,
        plan: CompactionPlan,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentCompactionOutcome, AgentError> {
        let identity = plan.identity;
        let mut guard = CompactionExecutionGuard {
            service: self.clone(),
            identity,
            armed: true,
        };
        let result = self.execute_compaction_inner(&plan).await;
        let result = match result {
            Ok(sample) => {
                self.set_compaction_status(
                    identity,
                    AgentCompactionStatus::ReadyToInstall,
                    sample.summary.clone(),
                    Some(compaction_usage(&sample)),
                )
                .await?;
                match self.commit_compaction(plan, sample).await {
                    Ok((outcome, event)) => {
                        event_sink.emit(event);
                        Ok(outcome)
                    }
                    Err(error) => {
                        let event = self
                            .finish_compaction_terminal(
                                identity,
                                AgentCompactionStatus::Failed,
                                Some(error.to_string()),
                            )
                            .await?;
                        if let Some(event) = event {
                            event_sink.emit(event);
                        }
                        Err(error)
                    }
                }
            }
            Err(AgentError::Cancelled) => {
                let event = self
                    .finish_compaction_terminal(identity, AgentCompactionStatus::Cancelled, None)
                    .await?;
                if let Some(event) = event {
                    event_sink.emit(event);
                }
                Err(AgentError::Cancelled)
            }
            Err(error) => {
                let event = self
                    .finish_compaction_terminal(
                        identity,
                        AgentCompactionStatus::Failed,
                        Some(error.to_string()),
                    )
                    .await?;
                if let Some(event) = event {
                    event_sink.emit(event);
                }
                Err(error)
            }
        };
        guard.armed = false;
        result
    }

    async fn execute_compaction_inner(
        &self,
        plan: &CompactionPlan,
    ) -> Result<CompactionSample, AgentError> {
        let estimated_input_tokens = estimate_model_request_tokens(&plan.request)?;
        let initial_context_digest = canonical_context_digest(&plan.canonical_initial_context);
        match plan.implementation {
            AgentCompactionImplementation::LocalSummary => {
                let (summary, usage, estimated_input_tokens) =
                    self.collect_local_summary(plan).await?;
                if summary.trim().is_empty() {
                    return Err(AgentError::Protocol(
                        "local compaction returned an empty assistant summary".to_string(),
                    ));
                }
                let summary = format!("{SUMMARY_PREFIX}\n\n{summary}");
                validate_compaction_summary(&summary, self.inner.limits)?;
                Ok(CompactionSample {
                    estimated_output_tokens: hanafubuki_agent_contracts::estimate_tokens(
                        summary.len(),
                    ),
                    replacement_history: local_summary_replacement(plan, &summary),
                    summary: Some(summary),
                    initial_context_digest,
                    reported_usage: usage,
                    estimated_input_tokens,
                })
            }
            AgentCompactionImplementation::ResponsesCompact => {
                let result = plan
                    .model
                    .compact(plan.request.clone(), plan.cancellation.clone())
                    .await
                    .map_err(map_model_error)?;
                let replacement_history = remote_replacement(plan, &result.replacement_history)?;
                Ok(CompactionSample {
                    estimated_output_tokens: estimate_conversation_items_tokens(
                        &replacement_history,
                    )?,
                    replacement_history,
                    summary: None,
                    initial_context_digest,
                    reported_usage: result.usage,
                    estimated_input_tokens,
                })
            }
            AgentCompactionImplementation::ResponsesCompactV2 => {
                let (payload, usage) = collect_v2_compaction(plan).await?;
                let replacement_history = remote_v2_replacement(plan, payload);
                Ok(CompactionSample {
                    estimated_output_tokens: estimate_conversation_items_tokens(
                        &replacement_history,
                    )?,
                    replacement_history,
                    summary: None,
                    initial_context_digest,
                    reported_usage: usage,
                    estimated_input_tokens,
                })
            }
        }
    }

    async fn collect_local_summary(
        &self,
        plan: &CompactionPlan,
    ) -> Result<(String, Option<hanafubuki_agent_model_api::ModelUsage>, u64), AgentError> {
        let mut retries = 0u32;
        let mut trim_index = 0usize;
        let mut removed_inputs = 0usize;
        let mut request = plan.request.clone();
        loop {
            self.check_compaction_cancelled(plan.identity)?;
            let mut stream = match plan
                .model
                .stream(request.clone(), plan.cancellation.clone())
                .await
            {
                Ok(stream) => stream,
                Err(error) if is_context_window_exceeded(&error) => {
                    trim_local_compaction_request(
                        &mut request,
                        &plan.local_trim_prefix_ends,
                        &mut trim_index,
                        &mut removed_inputs,
                    )?;
                    retries = 0;
                    continue;
                }
                Err(error) => return Err(map_model_error(error)),
            };
            let mut started_response_id = None;
            let mut last_summary = None;
            let mut saw_output = false;
            let attempt = loop {
                let Some(event) = stream.next().await else {
                    break Err(AgentError::Protocol(
                        "compaction stream ended before response completion".to_string(),
                    ));
                };
                match event {
                    Err(ModelError::StreamInterrupted(_) | ModelError::IdleTimeout { .. })
                        if !saw_output && retries < plan_retry_limit(plan) =>
                    {
                        break Err(AgentError::CompactionUnavailable);
                    }
                    Err(error) if is_context_window_exceeded(&error) && !saw_output => {
                        break Err(AgentError::ContextBudgetExceeded {
                            source: AgentUsageSource::Estimated,
                            consumed: 0,
                            limit: 0,
                        });
                    }
                    Err(error) => break Err(map_model_error(error)),
                    Ok(ModelEvent::ResponseStarted { response_id, .. }) => {
                        if started_response_id.replace(response_id).is_some() {
                            break Err(AgentError::Protocol(
                                "compaction stream started more than one response".to_string(),
                            ));
                        }
                    }
                    Ok(
                        ModelEvent::OutputItemAdded { .. } | ModelEvent::AssistantTextDelta { .. },
                    ) => {
                        require_response_started(&started_response_id)?;
                        saw_output = true;
                    }
                    Ok(ModelEvent::ReasoningDelta { .. }) => {
                        require_response_started(&started_response_id)?;
                    }
                    Ok(ModelEvent::OutputItemCompleted { item }) => {
                        require_response_started(&started_response_id)?;
                        saw_output = true;
                        match item {
                            ModelOutputItem::AssistantMessage { text, .. } => {
                                last_summary = Some(text)
                            }
                            ModelOutputItem::ToolCall { .. }
                            | ModelOutputItem::Compaction { .. } => {
                                break Err(AgentError::Protocol(
                                    "local compaction returned a forbidden output item".to_string(),
                                ));
                            }
                            ModelOutputItem::Reasoning { .. } | ModelOutputItem::Other { .. } => {}
                        }
                    }
                    Ok(ModelEvent::ResponseIncomplete { reason, .. }) => {
                        break Err(AgentError::Protocol(format!(
                            "local compaction response was incomplete: {reason}"
                        )));
                    }
                    Ok(ModelEvent::ResponseCompleted {
                        response_id,
                        usage,
                        end_turn,
                    }) => {
                        if require_response_started(&started_response_id)? != &response_id
                            || end_turn == Some(false)
                        {
                            break Err(AgentError::Protocol(
                                "local compaction response completion is inconsistent".to_string(),
                            ));
                        }
                        let summary = last_summary.ok_or_else(|| {
                            AgentError::Protocol(
                                "compaction completed without an assistant summary".to_string(),
                            )
                        })?;
                        break Ok((summary, usage, estimate_model_request_tokens(&request)?));
                    }
                }
            };
            match attempt {
                Err(AgentError::CompactionUnavailable) => {
                    retries = retries.saturating_add(1);
                    wait_for_compaction_retry(plan, retries).await?;
                    continue;
                }
                Err(AgentError::ContextBudgetExceeded { .. })
                    if trim_index < plan.local_trim_prefix_ends.len() =>
                {
                    trim_local_compaction_request(
                        &mut request,
                        &plan.local_trim_prefix_ends,
                        &mut trim_index,
                        &mut removed_inputs,
                    )?;
                    retries = 0;
                    continue;
                }
                result => return result,
            }
        }
    }
}

async fn collect_v2_compaction(
    plan: &CompactionPlan,
) -> Result<
    (
        hanafubuki_agent_model_api::ModelOpaqueCompactionPayload,
        Option<hanafubuki_agent_model_api::ModelUsage>,
    ),
    AgentError,
> {
    let mut stream = plan
        .model
        .stream(plan.request.clone(), plan.cancellation.clone())
        .await
        .map_err(map_model_error)?;
    let mut response_id = None;
    let mut payload = None;
    while let Some(event) = stream.next().await {
        match event.map_err(map_model_error)? {
            ModelEvent::ResponseStarted {
                response_id: id, ..
            } => {
                if response_id.replace(id).is_some() {
                    return Err(AgentError::Protocol(
                        "v2 compaction started multiple responses".to_string(),
                    ));
                }
            }
            ModelEvent::OutputItemAdded { .. } | ModelEvent::ReasoningDelta { .. } => {
                require_response_started(&response_id)?;
            }
            ModelEvent::AssistantTextDelta { .. } => {
                return Err(AgentError::Protocol(
                    "v2 compaction returned assistant text".to_string(),
                ));
            }
            ModelEvent::OutputItemCompleted { item } => match item {
                ModelOutputItem::Compaction { payload: value, .. } if payload.is_none() => {
                    payload = Some(value);
                }
                ModelOutputItem::Compaction { .. } => {
                    return Err(AgentError::Protocol(
                        "v2 compaction returned multiple compaction items".to_string(),
                    ));
                }
                _ => {
                    return Err(AgentError::Protocol(
                        "v2 compaction returned a non-compaction output item".to_string(),
                    ));
                }
            },
            ModelEvent::ResponseIncomplete { reason, .. } => {
                return Err(AgentError::Protocol(format!(
                    "v2 compaction response was incomplete: {reason}"
                )));
            }
            ModelEvent::ResponseCompleted {
                response_id: id,
                usage,
                end_turn,
            } => {
                if require_response_started(&response_id)? != &id || end_turn == Some(false) {
                    return Err(AgentError::Protocol(
                        "v2 compaction response completion is inconsistent".to_string(),
                    ));
                }
                let payload = payload.ok_or_else(|| {
                    AgentError::Protocol("v2 compaction returned no compaction item".to_string())
                })?;
                if stream.next().await.is_some() {
                    return Err(AgentError::Protocol(
                        "v2 compaction returned an event after response completion".to_string(),
                    ));
                }
                return Ok((payload, usage));
            }
        }
    }
    Err(AgentError::Protocol(
        "v2 compaction stream ended before completion".to_string(),
    ))
}

fn plan_retry_limit(plan: &CompactionPlan) -> u32 {
    plan.sampling_retry_policy.maximum_retries
}

async fn wait_for_compaction_retry(
    plan: &CompactionPlan,
    current_retry: u32,
) -> Result<(), AgentError> {
    let delay = plan.sampling_retry_policy.delay_for_retry(current_retry);
    if super::retry_wait::cancellable_delay(&plan.cancellation, delay).await {
        Err(AgentError::Cancelled)
    } else {
        Ok(())
    }
}

fn is_context_window_exceeded(error: &ModelError) -> bool {
    matches!(error, ModelError::ContextWindowExceeded { .. })
}

fn trim_local_compaction_request(
    request: &mut ModelRequest,
    prefix_ends: &[usize],
    trim_index: &mut usize,
    removed_inputs: &mut usize,
) -> Result<(), AgentError> {
    let target_end = prefix_ends.get(*trim_index).copied().ok_or_else(|| {
        AgentError::Protocol(
            "local compaction exceeded the provider context window after all safe history groups were trimmed"
                .to_string(),
        )
    })?;
    let remove = target_end.saturating_sub(*removed_inputs);
    if remove == 0 || remove >= request.input.len().saturating_sub(1) {
        return Err(AgentError::Protocol(
            "local compaction could not trim another complete history group".to_string(),
        ));
    }
    request.input.drain(..remove);
    *removed_inputs = target_end;
    *trim_index = trim_index.saturating_add(1);
    Ok(())
}

fn validate_compaction_summary(summary: &str, limits: AgentHardLimits) -> Result<(), AgentError> {
    if summary.trim().is_empty() {
        return Err(AgentError::Protocol(
            "compaction summary must not be empty".to_string(),
        ));
    }
    enforce_size(
        "compaction summary bytes",
        summary.len(),
        limits.maximum_compaction_summary_bytes,
    )?;
    let tokens = hanafubuki_agent_contracts::estimate_tokens(summary.len());
    if tokens > limits.maximum_compaction_summary_tokens {
        return Err(AgentError::LimitExceeded {
            resource: "compaction summary tokens",
            limit: limits.maximum_compaction_summary_tokens as usize,
        });
    }
    Ok(())
}
