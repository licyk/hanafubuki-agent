use super::*;

/// The part of the history a compaction replaces, and the windows it moves
/// between.
struct CompactionSource {
    items: Vec<ConversationItem>,
    item_ids: Vec<AgentHistoryItemId>,
    current_turn_suffix: Vec<AgentHistoryItem>,
    revision: AgentRevision,
    window: AgentContextWindowSnapshot,
    target_window: AgentContextWindowSnapshot,
    target_window_chain: Vec<AgentContextWindowSnapshot>,
}

impl AgentService {
    #[allow(
        clippy::too_many_arguments,
        reason = "independent inputs that a parameter struct would only regroup"
    )]
    pub(super) async fn begin_compaction(
        &self,
        session_id: AgentSessionId,
        trigger: AgentCompactionTrigger,
        phase: AgentCompactionPhase,
        reason: AgentCompactionReason,
        expected_session: Option<(AgentGeneration, AgentRevision)>,
        turn_identity: Option<TurnIdentity>,
        transport: CompactionTransport,
    ) -> Result<CompactionPlan, AgentError> {
        // Read with no lock held; the request below is assembled from them.
        let images = self.load_history_images(session_id).await?;
        let (
            staged,
            identity,
            request,
            cancellation,
            source,
            canonical_initial_context,
            local_trim_prefix_ends,
            sampling_retry_policy,
        ) = {
            let cell = self.session(session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let compaction_id = self.allocate_compaction_id();
            let record = &mut *guard;
            ensure_no_staged_commit(record)?;
            let previous = record.rollback_state();
            validate_compaction_start(record, trigger, phase, expected_session, turn_identity)?;
            let source = compaction_source(record, phase, turn_identity)?;
            let canonical_initial_context = canonical_initial_context(record);
            let generation = AgentCompactionGeneration::new(
                record
                    .runtime
                    .compaction
                    .as_ref()
                    .map_or(0, |value| value.snapshot.generation.value())
                    .checked_add(1)
                    .expect("agent compaction generation exhausted"),
            );
            let cancellation = CancellationToken::default();
            let sampling_retry_policy = record.settings.config.sampling_retry_policy;
            let identity = CompactionIdentity {
                session_id,
                id: compaction_id,
                generation,
            };
            let (request, implementation, local_trim_prefix_ends) = compaction_request(
                record,
                &source.items,
                &images,
                transport.model.as_ref(),
                &transport.tool_plan,
            )?;
            let compaction_now = now_millis_u64();
            let reserved = AgentCompactionSnapshot {
                id: compaction_id,
                generation,
                trigger,
                phase,
                reason,
                implementation,
                status: AgentCompactionStatus::Reserved,
                source_revision: source.revision,
                source_window_id: source.window.window_id,
                target_window: source.target_window,
                turn_generation: turn_identity.map(|value| value.generation),
                summary: None,
                usage: None,
                last_error: None,
                created_at: compaction_now,
                updated_at: compaction_now,
            };
            reserve_compaction(record, reserved, cancellation.clone(), self.inner.limits);
            (
                StagedPersistence::commit(&cell, record, previous),
                identity,
                request,
                cancellation,
                source,
                canonical_initial_context,
                local_trim_prefix_ends,
                sampling_retry_policy,
            )
        };
        self.persist_staged(staged).await?;
        self.set_compaction_status(identity, AgentCompactionStatus::Running, None, None)
            .await?;
        Ok(CompactionPlan {
            identity,
            request,
            next_tool_plan: transport.tool_plan,
            next_prompt: transport.prompt,
            model: transport.model,
            cancellation,
            source_items: source.items,
            source_item_ids: source.item_ids,
            current_turn_suffix: source.current_turn_suffix,
            source_revision: source.revision,
            source_window: source.window,
            target_window: source.target_window,
            target_window_chain: source.target_window_chain,
            canonical_initial_context,
            local_trim_prefix_ends,
            sampling_retry_policy,
            images,
        })
    }

    /// The transport of a standalone compaction: the session's model, and
    /// the tool plan and prompt its next request would have, outside any
    /// sampling step (no memory, nothing prepared).
    pub(super) async fn prepare_compaction_transport(
        &self,
        session_id: AgentSessionId,
    ) -> Result<CompactionTransport, AgentError> {
        let (model, context) = {
            let cell = self.session(session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            let model = record.settings.model.create_model_session();
            let context = AgentToolCatalogContext {
                session_id,
                turn_id: record
                    .runtime
                    .turn
                    .as_ref()
                    .map_or(AgentTurnId::new(0), |turn| turn.id),
                generation: record.status.generation,
                session_revision: crate::AgentRevision::new(record.status.revision),
                sampling_step: 1,
                mode: record.settings.config.mode,
                input_capabilities: record.settings.effective_input_capabilities,
                provider_capabilities: model.tool_capabilities(),
                selected_deferred_tools: BTreeSet::new(),
                memory_contribution_enabled: false,
            };
            (model, context)
        };
        let tool_plan = self
            .build_tool_plan(context, CancellationToken::default())
            .await
            .map_err(|error| AgentError::Protocol(error.to_string()))?;
        // The commit goes ahead only if the session's revision is still the
        // compaction's, so the prompt is rendered for the state it installs
        // into.
        let prompt = self.render_prompt(session_id)?;
        Ok(CompactionTransport {
            model,
            tool_plan,
            prompt,
        })
    }

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
}

/// Refuses a compaction the session cannot start: archived, disposing, one
/// already running, an identity that does not match the trigger and phase,
/// or nothing to compact.
fn validate_compaction_start(
    record: &SessionRecord,
    trigger: AgentCompactionTrigger,
    phase: AgentCompactionPhase,
    expected_session: Option<(AgentGeneration, AgentRevision)>,
    turn_identity: Option<TurnIdentity>,
) -> Result<(), AgentError> {
    if record.status.lifecycle == AgentSessionLifecycle::Archived {
        return Err(AgentError::SessionArchived);
    }
    if record.status.disposing {
        return Err(AgentError::SessionMutationNotConfirmed);
    }
    if record
        .runtime
        .compaction
        .as_ref()
        .is_some_and(|value| !value.snapshot.status.is_terminal())
    {
        return Err(AgentError::CompactionAlreadyRunning);
    }
    match (trigger, phase, expected_session, turn_identity) {
        (
            AgentCompactionTrigger::Manual,
            AgentCompactionPhase::StandaloneTurn,
            Some((generation, revision)),
            None,
        ) => validate_session_mutation(record, generation, revision)?,
        (
            AgentCompactionTrigger::Automatic,
            AgentCompactionPhase::PreTurn | AgentCompactionPhase::MidTurn,
            None,
            Some(identity),
        ) => {
            if exact_turn_identity(record, identity)?.phase.is_terminal() {
                return Err(AgentError::TurnNotRunning);
            }
        }
        _ => {
            return Err(AgentError::Protocol(
                "invalid Agent compaction lifecycle identity".to_string(),
            ));
        }
    }
    if record.conversation.history.is_empty() {
        return Err(AgentError::CompactionUnavailable);
    }
    Ok(())
}

/// The history before the compaction boundary (a pre-turn compaction keeps
/// the starting turn out of it) and the next context window.
fn compaction_source(
    record: &SessionRecord,
    phase: AgentCompactionPhase,
    turn_identity: Option<TurnIdentity>,
) -> Result<CompactionSource, AgentError> {
    let compaction_boundary = if phase == AgentCompactionPhase::PreTurn {
        let identity = turn_identity.ok_or_else(|| {
            AgentError::Protocol("pre-turn compaction is missing its turn identity".to_string())
        })?;
        record
            .conversation.history
            .iter()
            .position(|item| {
                matches!(&item.content, ConversationItem::TurnStart { turn_id } if *turn_id == identity.turn_id)
            })
            .ok_or(AgentError::TurnNotRunning)?
    } else {
        record.conversation.history.len()
    };
    let items = history_contents(&record.conversation.history[..compaction_boundary]);
    if items.is_empty() {
        return Err(AgentError::CompactionUnavailable);
    }
    let window = record.runtime.context_window;
    let target_window = window.next()?;
    let mut target_window_chain = match &record.conversation.model_context_boundary {
        AgentModelContextBoundary::Full => vec![window],
        AgentModelContextBoundary::Compacted { checkpoint } => checkpoint.window_chain.clone(),
    };
    target_window_chain.push(target_window);
    Ok(CompactionSource {
        items,
        item_ids: record
            .conversation
            .history
            .iter()
            .map(|item| item.item_id)
            .collect(),
        current_turn_suffix: record.conversation.history[compaction_boundary..]
            .iter()
            .map(|item| item.as_ref().clone())
            .collect(),
        revision: AgentRevision::new(record.status.revision),
        window,
        target_window,
        target_window_chain,
    })
}

/// The model request that compacts `source` with the session's compaction
/// implementation, and the local-trim prefix ends of its input.
fn compaction_request(
    record: &SessionRecord,
    source: &[ConversationItem],
    images: &LoadedImages,
    model: &dyn ModelSession,
    tool_plan: &AgentToolPlan,
) -> Result<(ModelRequest, AgentCompactionImplementation, Vec<usize>), AgentError> {
    let (mut input, local_trim_prefix_ends) = build_compaction_input(source, images)?;
    let implementation =
        AgentCompactionImplementation::from(record.settings.config.compaction_capability);
    if model.compaction_capability() != record.settings.config.compaction_capability {
        return Err(AgentError::Protocol(
            "compaction transport capability changed after session creation".to_string(),
        ));
    }
    let local_summary = implementation == AgentCompactionImplementation::LocalSummary;
    if local_summary {
        if let Some(task_plan) = record.runtime.task_plan.as_ref() {
            input.push(task_plan_model_input(task_plan));
        }
        input.push(ModelInputItem::user_text(SUMMARIZATION_PROMPT));
    } else if implementation == AgentCompactionImplementation::ResponsesCompactV2 {
        input.push(ModelInputItem::CompactionTrigger);
    }
    let request = ModelRequest {
        model: record.settings.config.model.clone(),
        instructions: record.settings.config.instructions.clone(),
        input,
        tools: if local_summary {
            Vec::new()
        } else {
            tool_plan.model_specs()
        },
        tool_choice: hanafubuki_agent_model_api::ModelToolChoice::None,
        parallel_tool_calls: !local_summary && tool_plan.parallel_tool_calls(),
        reasoning: record.settings.config.effective_reasoning(),
        output_format: None,
    };
    Ok((request, implementation, local_trim_prefix_ends))
}

/// Records `reserved` as the session's compaction and its event.
fn reserve_compaction(
    record: &mut SessionRecord,
    reserved: AgentCompactionSnapshot,
    cancellation: CancellationToken,
    limits: AgentHardLimits,
) {
    record.status.updated_at = now_millis();
    record.runtime.compaction = Some(CompactionRecord {
        snapshot: reserved.clone(),
        cancellation,
        operation_revision: record.status.revision,
    });
    record_compaction_event(record, reserved, limits);
    if let Some(compaction) = record.runtime.compaction.as_mut() {
        compaction.operation_revision = record.status.revision;
    }
}
