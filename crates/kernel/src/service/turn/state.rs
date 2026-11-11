use super::*;

impl AgentService {
    pub(super) fn begin_turn(
        &self,
        session_id: AgentSessionId,
        input: crate::AgentTurnInput,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AdmittedTurn, AgentError> {
        self.begin_turn_with_admission(session_id, input, TurnAdmission::Regular, event_sink)
    }

    /// Admits a turn on the calling thread, for callers off the async
    /// runtime (the application submits on its blocking boundary, so the
    /// admission and the managed task's spawn are never separated). The
    /// observers are told by `spawn_managed_turn`, once the session's sink
    /// is retained and before the turn's task can commit anything.
    pub(super) fn begin_turn_with_admission(
        &self,
        session_id: AgentSessionId,
        input: crate::AgentTurnInput,
        admission: TurnAdmission,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AdmittedTurn, AgentError> {
        validate_turn_input(&input)?;
        self.refresh_session_image_input_capability_blocking(session_id)?;
        let cell = self.session(session_id)?;
        let admitted = {
            let mut record = cell.lock_after_persistence_blocking()?;
            self.stage_turn_admission(&cell, &mut record, session_id, input, admission)?
        };
        let StagedAdmission {
            staged,
            admitted,
            started_event,
            history_event,
            title_event,
        } = admitted;
        self.persist_staged_blocking(staged)?;
        event_sink.emit(started_event);
        event_sink.emit(history_event);
        if let Some(title_event) = title_event {
            event_sink.emit(title_event);
        }
        Ok(admitted)
    }

    /// Admits a turn from async code: staged under the session's lock, then
    /// appended with the lock released and the append awaited, then
    /// published and told to the observers. Dropped while the append is in
    /// flight, it cancels the turn once the admission is published, as a
    /// dropped turn does.
    pub(super) async fn admit_turn(
        &self,
        session_id: AgentSessionId,
        input: crate::AgentTurnInput,
        event_sink: &Arc<dyn AgentEventSink>,
    ) -> Result<TurnIdentity, AgentError> {
        validate_turn_input(&input)?;
        self.refresh_session_image_input_capability(session_id)
            .await?;
        let cell = self.session(session_id)?;
        let admitted = {
            let mut record = cell.lock_after_persistence().await?;
            self.stage_turn_admission(
                &cell,
                &mut record,
                session_id,
                input,
                TurnAdmission::Regular,
            )?
        };
        let StagedAdmission {
            staged,
            admitted,
            started_event,
            history_event,
            title_event,
        } = admitted;
        let identity = admitted.identity;
        let mut guard = TurnExecutionGuard {
            service: self.clone(),
            identity,
            event_sink: event_sink.clone(),
            armed: true,
        };
        let persisted = self.persist_staged(staged).await;
        guard.armed = false;
        persisted?;
        event_sink.emit(started_event);
        event_sink.emit(history_event);
        if let Some(title_event) = title_event {
            event_sink.emit(title_event);
        }
        self.notify_turn_admitted(admitted);
        Ok(identity)
    }

    /// Tells the observers that a turn's admission is durable and its
    /// events are emitted. No session lock is held.
    pub(super) fn notify_turn_admitted(&self, admitted: AdmittedTurn) {
        self.inner
            .registries
            .observers
            .notify(&AgentNotification::TurnAdmitted {
                session_id: admitted.identity.session_id,
                turn_id: admitted.identity.turn_id,
                generation: admitted.identity.generation,
                origin: admitted.origin,
                provisional_title: admitted.provisional_title,
            });
    }

    /// Stages the admission of a turn on `record`, the record `cell` holds.
    fn stage_turn_admission(
        &self,
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        session_id: AgentSessionId,
        mut input: crate::AgentTurnInput,
        admission: TurnAdmission,
    ) -> Result<StagedAdmission, AgentError> {
        // The turn's mode is frozen here, tightened to the session's cap.
        let permission_mode = record
            .settings
            .config
            .scope
            .capped_permission_mode(input.permission_mode);
        let origin = input.origin;
        let provisional_title = input.provisional_title.take();
        // Read under the session's lock: shutdown clears the flag before it
        // locks each session to cancel what runs there.
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let turn_id = self.allocate_turn_id();
        if record.settings.config.model.is_empty() {
            return Err(AgentError::InvalidInput(
                "no model is selected for this Agent session".to_string(),
            ));
        }
        ensure_no_staged_commit(record)?;
        let previous = record.rollback_state();
        if record.status.disposing {
            return Err(AgentError::SessionMutationNotConfirmed);
        }
        if record.status.lifecycle == AgentSessionLifecycle::Archived {
            return Err(AgentError::SessionArchived);
        }
        if record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::TurnAlreadyRunning);
        }
        if record
            .runtime
            .compaction
            .as_ref()
            .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
        {
            return Err(AgentError::CompactionAlreadyRunning);
        }
        let implementing_plan = match admission {
            TurnAdmission::ImplementPlan(request) => {
                validate_plan_implementation(record, session_id, &request)?;
                true
            }
            TurnAdmission::Regular => false,
        };
        if !input.images.is_empty() && !record.settings.effective_input_capabilities.image_input {
            return Err(AgentError::InvalidInput(
                "the configured model does not support image input".to_string(),
            ));
        }
        let extensions = input.turn.clone();
        let turn_items = resolve_turn_items(turn_id, &record.conversation.history, input)?;
        let selected_skill_count = turn_items
            .iter()
            .filter(|item| matches!(item, ConversationItem::SkillContext { .. }))
            .count();
        for item in &turn_items {
            if let ConversationItem::SkillContext { skill, .. } = item {
                log::info!(
                    "agent_diag stage=turn.skill.select session_id={} skill={} scope={} reason=explicit_reference_or_dollar_mention selected_count={}",
                    session_id,
                    skill,
                    skill.scope(),
                    selected_skill_count,
                );
            }
        }
        validate_history_append(record, &turn_items, self.inner.limits)?;
        if implementing_plan {
            record.settings.config.mode = crate::AgentSessionMode::Default;
        }
        record.runtime.task_plan = None;
        let generation = AgentGeneration::new(
            record
                .status
                .generation
                .value()
                .checked_add(1)
                .expect("agent generation exhausted"),
        );
        record.status.generation = generation;
        let now = now_millis();
        let identity = TurnIdentity {
            session_id,
            turn_id,
            generation,
        };
        record.runtime.turn = Some(TurnRecord {
            id: turn_id,
            generation,
            phase: AgentTurnPhase::Sampling,
            sampling_step: 0,
            reasoning: AgentReasoningCheckpoint::default(),
            final_answer_message_id: None,
            last_error: None,
            terminal_reason: None,
            cancellation: CancellationToken::default(),
            created_at: now,
            updated_at: now,
            recovery: None,
            budget_ledger: AgentTurnBudgetLedger::default(),
            no_progress: AgentNoProgressState::default(),
            pending_steering: VecDeque::new(),
            next_steering_sequence: 1,
            last_consumed_steering_sequence: None,
            model_session: record.settings.model.create_model_session(),
            model_session_generation: AgentModelSessionGeneration::new(1),
            permission_mode,
            consecutive_auto_review_denials: 0,
            sampling_retry_policy: record.settings.config.sampling_retry_policy,
            extensions,
            compression_fallback: false,
        });
        let history_start = record.conversation.history.len();
        append_history(record, &turn_items);
        let started_event = record_event(
            record,
            identity,
            AgentEventKind::TurnStarted {
                role: record.settings.config.scope.role.clone(),
            },
            self.inner.limits,
        );
        let history_event = record_event(
            record,
            identity,
            AgentEventKind::HistoryCommitted {
                start_index: history_start,
                item_count: turn_items.len(),
                usage: None,
            },
            self.inner.limits,
        );
        // A session without a title takes the provisional one in the same
        // commit, so it is shown as soon as the turn is.
        let provisional_title = provisional_title
            .filter(|_| record.settings.title.is_none())
            .and_then(|title| match AgentSessionTitle::automatic(title) {
                Ok(title) => Some(title),
                Err(error) => {
                    log::debug!(
                        "Agent provisional title not committed session={session_id}: {error}"
                    );
                    None
                }
            });
        let title_event = provisional_title.as_ref().map(|title| {
            record.settings.title = Some(title.clone());
            record_session_event(
                record,
                AgentEventKind::SessionTitleUpdated {
                    title: title.clone(),
                },
                self.inner.limits,
            )
        });
        Ok(StagedAdmission {
            staged: StagedPersistence::commit(cell, record, previous),
            admitted: AdmittedTurn {
                identity,
                origin,
                provisional_title: provisional_title.map(|title| title.value),
            },
            started_event,
            history_event,
            title_event,
        })
    }
}

/// A staged turn admission and the events it makes visible once durable.
struct StagedAdmission {
    staged: StagedPersistence,
    admitted: AdmittedTurn,
    started_event: AgentEvent,
    history_event: AgentEvent,
    /// The provisional title the admission gave the session.
    title_event: Option<AgentEvent>,
}

/// A durably admitted turn and what the observers are told about it.
pub(super) struct AdmittedTurn {
    pub(super) identity: TurnIdentity,
    origin: hanafubuki_agent_contracts::AgentTurnOrigin,
    /// The provisional title committed with the admission.
    provisional_title: Option<String>,
}

/// A turn's input holds text or an image.
fn validate_turn_input(input: &crate::AgentTurnInput) -> Result<(), AgentError> {
    if input.text.trim().is_empty() && input.images.is_empty() {
        return Err(AgentError::InvalidInput(
            "user message must contain text or an image".to_string(),
        ));
    }
    Ok(())
}

/// A plan implementation is admitted only for the exact completed Plan turn
/// whose final answer holds the committed plan.
fn validate_plan_implementation(
    record: &SessionRecord,
    session_id: AgentSessionId,
    request: &crate::AgentPlanImplementationRequest,
) -> Result<(), AgentError> {
    if request.session_id != session_id
        || record.status.generation != request.generation
        || record.status.revision != request.revision.value()
    {
        return Err(AgentError::StaleRevision);
    }
    if record.settings.config.mode != crate::AgentSessionMode::Plan {
        return Err(AgentError::InvalidInput(
            "plan implementation requires an idle Plan mode session".to_string(),
        ));
    }
    let Some(turn) = record.runtime.turn.as_ref() else {
        return Err(AgentError::SessionMutationNotConfirmed);
    };
    if turn.phase != AgentTurnPhase::Completed
        || turn.generation != request.generation
        || turn.final_answer_message_id != Some(request.message_id)
    {
        return Err(AgentError::StaleRevision);
    }
    let has_exact_plan = record.conversation.history.iter().rev().any(|item| {
        matches!(
            &item.content,
            ConversationItem::AssistantMessage {
                message_id,
                proposed_plan: Some(AgentPlanBlock { text }),
                ..
            } if *message_id == request.message_id && !text.trim().is_empty()
        )
    });
    if !has_exact_plan {
        return Err(AgentError::InvalidInput(
            "the selected final assistant message has no committed plan".to_string(),
        ));
    }
    Ok(())
}
