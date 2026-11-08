use super::*;

impl AgentService {
    pub(super) async fn execute_compaction_inner(
        &self,
        plan: &CompactionPlan,
    ) -> Result<CompactionSample, AgentError> {
        let estimated_input_tokens = estimate_model_request_tokens(&plan.request)?;
        let (implementation, phase) = {
            let cell = self.session(plan.identity.session_id)?;
            let record = cell.lock()?;
            let snapshot = &exact_compaction(&record, plan.identity)?.snapshot;
            (snapshot.implementation, snapshot.phase)
        };
        match implementation {
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
                let mut replacement_history =
                    collect_real_user_messages(&plan.source_items, COMPACT_USER_MESSAGE_MAX_TOKENS);
                replacement_history.push(ConversationItem::CompactionSummary {
                    summary: summary.clone(),
                    source_revision: plan.source_revision,
                    compacted_item_count: plan.source_items.len(),
                });
                inject_initial_context(
                    &mut replacement_history,
                    phase,
                    &plan.canonical_initial_context,
                );
                let initial_context_digest =
                    canonical_context_digest(&plan.canonical_initial_context);
                Ok(CompactionSample {
                    estimated_output_tokens: hanafubuki_agent_contracts::estimate_tokens(
                        summary.len(),
                    ),
                    replacement_history,
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
                let mut replacement_history = filter_remote_replacement(
                    &result.replacement_history,
                    &plan.source_items,
                    &plan.images,
                )?;
                if replacement_history.is_empty() {
                    return Err(AgentError::Protocol(
                        "remote compaction returned an empty replacement history".to_string(),
                    ));
                }
                inject_initial_context(
                    &mut replacement_history,
                    phase,
                    &plan.canonical_initial_context,
                );
                Ok(CompactionSample {
                    estimated_output_tokens: estimate_conversation_items_tokens(
                        &replacement_history,
                    )?,
                    replacement_history,
                    summary: None,
                    initial_context_digest: canonical_context_digest(
                        &plan.canonical_initial_context,
                    ),
                    reported_usage: result.usage,
                    estimated_input_tokens,
                })
            }
            AgentCompactionImplementation::ResponsesCompactV2 => {
                let (payload, usage) = collect_v2_compaction(plan).await?;
                let mut replacement_history = collect_real_user_messages(
                    &plan.source_items,
                    REMOTE_V2_RETAINED_MESSAGE_TOKEN_BUDGET,
                );
                replacement_history.push(ConversationItem::ProviderCompaction { payload });
                inject_initial_context(
                    &mut replacement_history,
                    phase,
                    &plan.canonical_initial_context,
                );
                Ok(CompactionSample {
                    estimated_output_tokens: estimate_conversation_items_tokens(
                        &replacement_history,
                    )?,
                    replacement_history,
                    summary: None,
                    initial_context_digest: canonical_context_digest(
                        &plan.canonical_initial_context,
                    ),
                    reported_usage: usage,
                    estimated_input_tokens,
                })
            }
        }
    }

    pub(super) async fn collect_local_summary(
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
                    Ok(ModelEvent::ReasoningSummaryDelta { .. }) => {
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
