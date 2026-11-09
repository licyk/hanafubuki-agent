use super::*;

use super::source::compaction_source;
use super::status::reserve_compaction;

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
        let (staged, identity, cancellation, source) = {
            let cell = self.session(session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let compaction_id = self.allocate_compaction_id();
            let record = &mut *guard;
            ensure_no_staged_commit(record)?;
            let previous = record.rollback_state();
            validate_compaction_start(record, trigger, phase, expected_session, turn_identity)?;
            let source = compaction_source(record, phase, turn_identity, &images, &transport)?;
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
            let identity = CompactionIdentity {
                session_id,
                id: compaction_id,
                generation,
            };
            let compaction_now = now_millis_u64();
            let reserved = AgentCompactionSnapshot {
                id: compaction_id,
                generation,
                trigger,
                phase,
                reason,
                implementation: source.implementation,
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
                cancellation,
                source,
            )
        };
        self.persist_staged(staged).await?;
        self.set_compaction_status(identity, AgentCompactionStatus::Running, None, None)
            .await?;
        Ok(source.into_plan(identity, phase, cancellation, transport, images))
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
                compression_tools: super::super::compression::CompressionMode::of(record)
                    == super::super::compression::CompressionMode::Hierarchical,
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
