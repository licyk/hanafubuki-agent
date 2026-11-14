use super::*;

/// A sampling step as it stands before its request is built: its tool plan
/// and its prompt, prepared once and used by the step's request, by the
/// admission estimate before it and by a compaction within it, so each
/// counts the same request.
pub(in crate::service) struct SamplingStep {
    pub(in crate::service) step: u32,
    pub(in crate::service) cancellation: CancellationToken,
    pub(in crate::service) model: Arc<dyn ModelSession>,
    pub(in crate::service) tool_plan: AgentToolPlan,
    pub(in crate::service) hook_catalog: ContinuationHooks,
    pub(in crate::service) prompt: RequestContext,
    pub(in crate::service) memory: crate::memory::StepMemoryPolicy,
}

/// What plans the tools of the turn's step `sampling_step`, as the session
/// stands.
pub(in crate::service) fn tool_catalog_context(
    record: &SessionRecord,
    identity: TurnIdentity,
    sampling_step: u32,
) -> Result<AgentToolCatalogContext, AgentError> {
    let turn = exact_turn_identity(record, identity)?;
    Ok(AgentToolCatalogContext {
        session_id: identity.session_id,
        turn_id: identity.turn_id,
        generation: identity.generation,
        session_revision: crate::AgentRevision::new(record.status.revision),
        sampling_step,
        mode: record.settings.config.mode,
        input_capabilities: record.settings.effective_input_capabilities,
        provider_capabilities: turn.model_session.tool_capabilities(),
        selected_deferred_tools: selected_deferred_tools(&record.conversation.history),
        memory_contribution_enabled: record.settings.memory_contribution.mode
            == crate::AgentMemoryContributionMode::Enabled,
        compression_tools: super::super::compression::planned_tools(record),
        scope: record.settings.config.scope.clone(),
    })
}

impl AgentService {
    /// Prepares the turn's next sampling step: the tool plan, and the prompt
    /// the contributors prepared and rendered with no lock held. The turn is
    /// left as it is: [`Self::begin_sampling`] opens the step and commits its
    /// request.
    pub(in crate::service) async fn prepare_sampling_step(
        &self,
        identity: TurnIdentity,
    ) -> Result<SamplingStep, AgentError> {
        self.refresh_session_image_input_capability(identity.session_id)
            .await?;
        let (catalog_context, cancellation, model, step_prompt_inputs, scope) = {
            let cell = self.session(identity.session_id)?;
            let guard = cell.lock_after_persistence().await?;
            let record = &*guard;
            let context_window_tokens = record.settings.config.context_window_tokens;
            let turn = exact_turn_identity(record, identity)?;
            if turn.cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            let step = turn
                .sampling_step
                .checked_add(1)
                .ok_or(AgentError::LimitExceeded {
                    resource: "sampling step representation",
                    limit: u32::MAX as usize,
                })?;
            // Every earlier step's tool outputs are committed by now, so the
            // turn ends with complete history.
            if step > self.inner.limits.maximum_sampling_steps {
                return Err(AgentError::LimitExceeded {
                    resource: "sampling steps",
                    limit: self.inner.limits.maximum_sampling_steps as usize,
                });
            }
            let model = turn.model_session.clone();
            (
                tool_catalog_context(record, identity, step)?,
                turn.cancellation.clone(),
                model,
                (Some(turn.extensions.clone()), context_window_tokens),
                record.settings.config.scope.clone(),
            )
        };
        let step = catalog_context.sampling_step;
        let tool_plan = self
            .build_tool_plan(catalog_context, cancellation.clone())
            .await
            .map_err(|error| match error {
                crate::AgentToolCatalogError::Cancelled => AgentError::Cancelled,
                error => AgentError::Protocol(error.to_string()),
            })?;
        // The plugins the session's scope turns off neither continue the
        // turn nor contribute to its prompt.
        let mut hook_catalog = self.inner.registries.continuation_hooks.ordered();
        hook_catalog.retain(|entry| !scope.turns_off(entry.owner()));
        // The request's sections are shown by the renderers in force now.
        let contributors = self
            .inner
            .registries
            .prompt
            .snapshot()
            .without_plugins(|owner| scope.turns_off(owner));
        let prompt_step = hanafubuki_agent_contracts::PromptStep {
            session_id: identity.session_id,
            turn_id: identity.turn_id,
            generation: identity.generation,
            sampling_step: step,
            tools: Arc::new(tool_plan.offered_tool_names()),
        };
        let prepared = prepare_prompt_step(
            &contributors,
            prompt_step,
            &cancellation,
            self.inner.limits.plugin_call_deadline,
        )
        .await?;
        let prompt = request_context(
            &contributors,
            self.inner.registries.context_renderers.snapshot(),
            step_prompt_inputs,
            Some(&prepared),
        );
        Ok(SamplingStep {
            step,
            cancellation,
            model,
            tool_plan,
            hook_catalog,
            prompt,
            memory: crate::memory::StepMemoryPolicy::of_step(&prepared),
        })
    }

    /// Opens `step` and commits its request; only if the turn is still
    /// before that step.
    pub(in crate::service) async fn begin_sampling(
        &self,
        identity: TurnIdentity,
        step: &SamplingStep,
        event_sink: &dyn AgentEventSink,
    ) -> Result<ModelRequest, AgentError> {
        let SamplingStep {
            step,
            tool_plan,
            prompt,
            ..
        } = step;
        let step = *step;
        // The history's images, read with no lock held.
        let images = self.load_history_images(identity.session_id).await?;
        let (staged, history_event, event, reasoning_event, failed_event, request) = {
            let cell = self.session(identity.session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            {
                let turn = exact_turn_mut(record, identity)?;
                if turn.sampling_step.checked_add(1) != Some(step) || turn.phase.is_terminal() {
                    return Err(AgentError::StaleRevision);
                }
                turn.sampling_step = step;
                turn.phase = AgentTurnPhase::Sampling;
            }
            let mut world_state = crate::model::effective_world_state(&record.conversation.history);
            let omission_update = tool_omission_world_state_update(
                &mut world_state,
                tool_plan.revision,
                tool_plan.omissions(),
            )?;
            let history_event = if let Some(item) = omission_update {
                validate_history_append(record, std::slice::from_ref(&item), self.inner.limits)?;
                let start_index = record.conversation.history.len();
                append_history(record, &[item]);
                Some(record_event(
                    record,
                    identity,
                    AgentEventKind::HistoryCommitted {
                        start_index,
                        item_count: 1,
                        usage: None,
                    },
                    self.inner.limits,
                ))
            } else {
                None
            };
            exact_turn_mut(record, identity)?
                .reasoning
                .begin_step(step)
                .map_err(map_reasoning_error)?;
            let event = record_event(
                record,
                identity,
                AgentEventKind::SamplingStarted { step },
                self.inner.limits,
            );
            let reasoning = exact_turn_mut(record, identity)?.reasoning.snapshot();
            let reasoning_event = record_event(
                record,
                identity,
                AgentEventKind::ReasoningUpdated { reasoning },
                self.inner.limits,
            );
            let (mut request, compression) = step_model_request_projected(
                record,
                &images,
                tool_plan,
                prompt,
                self.inner.limits.maximum_tool_output_bytes,
            )?;
            request.reasoning = self.request_reasoning(record);
            if let Some(state) = compression.state {
                record.runtime.compression = Some(state);
            }
            if let Some(store) = compression.content_store {
                record.runtime.compression_runtime.keep_content_store(store);
            }
            if let Some((tokens, abort_booked)) = compression.sent {
                record
                    .runtime
                    .compression_runtime
                    .sent(tokens, abort_booked);
            }
            let failed_event = match compression.failed {
                Some(message) => {
                    exact_turn_mut(record, identity)?.compression_fallback = true;
                    Some(record_event(
                        record,
                        identity,
                        AgentEventKind::ContextCompressionFailed { message },
                        self.inner.limits,
                    ))
                }
                None => None,
            };
            (
                StagedPersistence::delta(&cell, record, previous),
                history_event,
                event,
                reasoning_event,
                failed_event,
                request,
            )
        };
        self.persist_staged(staged).await?;
        self.persist_content_store(identity.session_id);
        if let Some(history_event) = history_event {
            event_sink.emit(history_event);
        }
        event_sink.emit(event);
        event_sink.emit(reasoning_event);
        if let Some(failed_event) = failed_event {
            event_sink.emit(failed_event);
        }
        Ok(request)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "independent inputs that a parameter struct would only regroup"
    )]
    pub(in crate::service) async fn run_sampling_request(
        &self,
        identity: TurnIdentity,
        model: Arc<dyn ModelSession>,
        request: ModelRequest,
        tool_plan: AgentToolPlan,
        memory: crate::memory::StepMemoryPolicy,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<SamplingOutput, AgentError> {
        let policy = {
            let cell = self.session(identity.session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            exact_turn(record, identity.turn_id, identity.generation)?.sampling_retry_policy
        };
        log::debug!(
            "agent_diag stage=model.stream.policy session_id={} turn_id={} generation={} maximum_retries={} initial_backoff_ms={} maximum_backoff_ms={} scope=before_first_model_event",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            policy.maximum_retries,
            policy.initial_backoff_millis,
            policy.maximum_backoff_millis,
        );
        let mut active_request = request;
        let mut retries = 0u32;
        let mut image_fallback_used = false;
        let mut reasoning_fallback_used = false;
        loop {
            if cancellation.is_cancelled() {
                return Err(AgentError::Cancelled);
            }
            match self
                .run_sampling_request_once(
                    identity,
                    model.clone(),
                    active_request.clone(),
                    tool_plan.clone(),
                    memory.clone(),
                    cancellation.clone(),
                    event_sink,
                )
                .await
            {
                Err(
                    AgentError::UnsupportedImageInput { .. }
                    | AgentError::UnsupportedReasoning { .. },
                ) if cancellation.is_cancelled() => {
                    return Err(AgentError::Cancelled);
                }
                Err(AgentError::UnsupportedImageInput {
                    status,
                    code,
                    message,
                    request_id,
                }) if !image_fallback_used && !cancellation.is_cancelled() => {
                    let (sanitized, report) =
                        hanafubuki_agent_model_api::sanitize_images_for_unsupported_input(
                            &active_request,
                        );
                    if report.total_replaced() == 0 {
                        return Err(AgentError::UnsupportedImageInput {
                            status,
                            code,
                            message,
                            request_id,
                        });
                    }
                    self.record_unsupported_image_input(
                        identity,
                        model.as_ref(),
                        &active_request.model,
                    )
                    .await?;
                    self.emit(
                        identity,
                        AgentEventKind::ModelRetryScheduled {
                            stage: AgentModelRetryStage::ImageInputFallback,
                            current_retry: 1,
                            maximum_retries: 1,
                            delay_millis: 0,
                            reason: format!(
                                "image input unsupported; retrying with {} image item(s) omitted",
                                report.total_replaced()
                            ),
                        },
                        event_sink,
                    )
                    .await?;
                    active_request = sanitized;
                    image_fallback_used = true;
                }
                Err(AgentError::UnsupportedReasoning { .. })
                    if !reasoning_fallback_used && active_request.reasoning.is_some() =>
                {
                    self.fall_back_from_reasoning(
                        identity,
                        model.as_ref(),
                        &active_request.model,
                        event_sink,
                    )
                    .await?;
                    active_request.reasoning = None;
                    reasoning_fallback_used = true;
                }
                Err(AgentError::ModelStreamInterrupted(message))
                    if retries < policy.maximum_retries && !cancellation.is_cancelled() =>
                {
                    retries += 1;
                    let delay = policy.delay_for_retry(retries);
                    self.emit(
                        identity,
                        AgentEventKind::ModelRetryScheduled {
                            stage: AgentModelRetryStage::ResponseStream,
                            current_retry: retries,
                            maximum_retries: policy.maximum_retries,
                            delay_millis: delay.as_millis() as u64,
                            reason: bounded_diagnostic(message),
                        },
                        event_sink,
                    )
                    .await?;
                    if super::super::retry_wait::cancellable_delay(&cancellation, delay).await {
                        return Err(AgentError::Cancelled);
                    }
                }
                // An interruption seen once the turn is cancelled is the cancellation.
                Err(AgentError::ModelStreamInterrupted(_)) if cancellation.is_cancelled() => {
                    return Err(AgentError::Cancelled);
                }
                Err(AgentError::ModelStreamInterrupted(message)) => {
                    return Err(AgentError::ModelStreamRetriesExhausted { retries, message });
                }
                Err(AgentError::ProviderContextWindowExceeded {
                    status,
                    code,
                    message,
                    request_id,
                }) => {
                    if !cancellation.is_cancelled() {
                        self.learn_context_window(
                            identity,
                            model.as_ref(),
                            &active_request,
                            &message,
                            event_sink,
                        )
                        .await?;
                    }
                    return Err(AgentError::ProviderContextWindowExceeded {
                        status,
                        code,
                        message,
                        request_id,
                    });
                }
                result => return result,
            }
        }
    }

    /// The Provider refused the request's reasoning parameter: the route
    /// learns it, and the request is retried once without it.
    async fn fall_back_from_reasoning(
        &self,
        identity: TurnIdentity,
        model: &dyn ModelSession,
        model_id: &str,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.record_unsupported_reasoning(model, model_id);
        self.emit(
            identity,
            AgentEventKind::ModelRetryScheduled {
                stage: AgentModelRetryStage::ReasoningFallback,
                current_retry: 1,
                maximum_retries: 1,
                delay_millis: 0,
                reason: "reasoning parameter unsupported; retrying without it".to_string(),
            },
            event_sink,
        )
        .await
    }

    pub(super) async fn record_unsupported_image_input(
        &self,
        identity: TurnIdentity,
        model: &dyn ModelSession,
        model_id: &str,
    ) -> Result<(), AgentError> {
        if let Some(route_key) = model.route_key(model_id) {
            self.inner
                .model_metadata
                .learn_image_input_unsupported(&route_key, now_millis_u64());
        }
        let cell = self.session(identity.session_id)?;
        let mut guard = cell.lock_after_persistence().await?;
        let record = &mut *guard;
        exact_turn(record, identity.turn_id, identity.generation)?;
        record.settings.effective_input_capabilities = record
            .settings
            .config
            .input_capabilities
            .effective(ModelImageInputCapability::Unsupported);
        Ok(())
    }
}

/// The deferred tools `tool_search` selected since the last compaction. A
/// selection that no longer names a planned tool (gone, or its generation
/// changed) matches nothing in the planner and is ignored there.
fn selected_deferred_tools(
    history: &[Arc<AgentHistoryItem>],
) -> std::collections::BTreeSet<crate::AgentToolSelection> {
    let mut selected = std::collections::BTreeSet::new();
    for item in history {
        match &item.content {
            ConversationItem::ToolSelection { tools, .. } => {
                selected.extend(tools.iter().cloned());
            }
            ConversationItem::CompactionSummary { .. }
            | ConversationItem::ProviderCompaction { .. } => selected.clear(),
            _ => {}
        }
    }
    selected
}
