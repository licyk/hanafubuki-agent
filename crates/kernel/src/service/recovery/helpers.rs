use super::*;

impl AgentService {
    /// Checks what a session persisted before it is loaded: its counters,
    /// configuration, model context, message ids, usage, compaction, context
    /// window and activities. `contents` is `model_context`'s items.
    ///
    /// The configuration is re-normalized first: a rule that tightened since
    /// the session was written clamps or drops that field instead of
    /// quarantining the session; the next commit persists the repair. The
    /// integrity checks stay strict.
    pub(super) fn validate_persisted_session(
        &self,
        runtime: &mut PersistedAgentRuntimeState,
        model_context: &[AgentHistoryItem],
        contents: &[ConversationItem],
    ) -> Result<(), AgentError> {
        validate_persisted_counters(runtime, model_context)?;
        normalize_session_config(&mut runtime.config);
        validate_session_config(&runtime.config, self.inner.limits)?;
        validate_persisted_history(contents, &runtime.config, self.inner.limits)?;
        validate_persisted_message_state(
            runtime.id,
            runtime.next_message_id,
            runtime.turn.as_ref(),
            contents,
        )?;
        validate_persisted_usage(runtime.usage, &runtime.config)?;
        validate_persisted_compaction(runtime.compaction.as_ref(), runtime.revision)?;
        validate_context_window_checkpoint(
            runtime.context_window,
            runtime.compaction.as_ref(),
            &runtime.model_context_boundary,
            runtime.revision,
            self.inner.limits,
        )?;
        validate_persisted_activities(&runtime.activities)
    }

    /// The live record of a validated persisted session over `model`.
    ///
    /// Nothing is running yet: cancellation tokens and the turn's model
    /// session are new, the event journal is empty, and runtime-only turn
    /// state (review denials, Skill catalog) starts over.
    pub(super) fn persisted_record(
        &self,
        runtime: PersistedAgentRuntimeState,
        model_context: Vec<AgentHistoryItem>,
        contents: &[ConversationItem],
        model: Arc<dyn ModelPort>,
    ) -> SessionRecord {
        let history_bytes = contents.iter().fold(0usize, |total, item| {
            total.saturating_add(item.retained_bytes())
        });
        let observed_image_input = model
            .route_key(&runtime.config.model)
            .map(|key| {
                self.inner
                    .image_capabilities
                    .lock()
                    .capability(&key, now_millis())
            })
            .unwrap_or(ModelImageInputCapability::Unknown);
        let turn = runtime.turn.map(|turn| TurnRecord {
            id: turn.id,
            generation: turn.generation,
            phase: turn.phase,
            sampling_step: turn.sampling_step,
            reasoning: turn.reasoning,
            final_answer_message_id: turn.final_answer_message_id,
            last_error: turn.last_error,
            terminal_reason: turn.terminal_reason,
            cancellation: CancellationToken::default(),
            created_at: turn.created_at,
            updated_at: turn.updated_at,
            recovery: turn.recovery,
            budget_ledger: turn.budget_ledger,
            no_progress: turn.no_progress,
            pending_steering: turn.pending_steering.into(),
            next_steering_sequence: turn.next_steering_sequence,
            last_consumed_steering_sequence: turn.last_consumed_steering_sequence,
            model_session: model.create_model_session(),
            model_session_generation: turn.model_session_generation,
            permission_mode: turn.permission_mode,
            consecutive_auto_review_denials: 0,
            sampling_retry_policy: AgentSamplingRetryPolicy::default(),
            extensions: hanafubuki_agent_contracts::TurnExtensions::default(),
        });
        SessionRecord {
            session_id: runtime.id,
            forked_from_session_id: runtime.forked_from_session_id,
            created_at: runtime.created_at,
            settings: SessionSettings {
                effective_input_capabilities: runtime
                    .config
                    .input_capabilities
                    .effective(observed_image_input),
                model,
                title: runtime.title,
                config: runtime.config,
                memory_contribution: runtime.memory_contribution,
                budget_policy: runtime.turn_budget_policy,
            },
            status: SessionStatus {
                lifecycle: runtime.lifecycle,
                generation: runtime.generation,
                revision: runtime.revision.value(),
                updated_at: runtime.updated_at,
                disposing: false,
                persisting_revision: None,
            },
            conversation: SessionConversation {
                history: Arc::new(model_context.into_iter().map(Arc::new).collect()),
                history_version: runtime.revision.value(),
                pending_history_mutations: Vec::new(),
                next_history_item_id: runtime.next_history_item_id,
                model_context_boundary: runtime.model_context_boundary,
                history_base: runtime.history_base,
                history_bytes,
                next_message_id: runtime.next_message_id,
            },
            journal: SessionJournal::empty(),
            runtime: SessionRuntime {
                turn,
                compaction: runtime.compaction.map(|snapshot| CompactionRecord {
                    snapshot,
                    cancellation: CancellationToken::default(),
                    operation_revision: runtime.revision.value(),
                }),
                usage: runtime.usage,
                context_window: runtime.context_window,
                activities: AgentActivityTracker::restore(runtime.activities),
                task_plan: runtime.task_plan,
                pending_user_input: runtime.pending_user_input,
                allowed_tools: BTreeSet::new(),
            },
        }
    }
}
