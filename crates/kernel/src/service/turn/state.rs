use super::*;
use crate::service::tool_execution::unfinished_user_shell_command;

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
    pub(in crate::service) fn begin_turn_with_admission(
        &self,
        session_id: AgentSessionId,
        input: crate::AgentTurnInput,
        admission: TurnAdmission,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AdmittedTurn, AgentError> {
        // A wake turn's opener is its events, not the input.
        if !matches!(admission, TurnAdmission::Wake) {
            validate_turn_input(&input)?;
        }
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
        let session_id = admitted.identity.session_id;
        let delivered = !admitted.wake_events.is_empty();
        self.inner
            .registries
            .observers
            .notify(&AgentNotification::TurnAdmitted {
                session_id: admitted.identity.session_id,
                turn_id: admitted.identity.turn_id,
                generation: admitted.identity.generation,
                origin: admitted.origin,
                automatic_title: admitted.automatic_title,
            });
        if delivered {
            self.notify_wake_events_left(session_id);
        }
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
        let extensions = input.turn.clone();
        let opened = admission_items(record, session_id, turn_id, input, admission)?;
        log_selected_skills(session_id, &opened.items);
        validate_history_append(record, &opened.items, self.inner.limits)?;
        let wake_events = match opened.delivers_wake_events {
            true => std::mem::take(&mut record.runtime.wake_queue.events),
            false => Vec::new(),
        };
        self.refresh_context_window(record);
        if opened.implementing_plan {
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
            kind: opened.kind,
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
        append_history(record, &opened.items);
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
                item_count: opened.items.len(),
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
        let title_event = provisional_title.map(|title| {
            record.settings.title = Some(title.clone());
            record_session_event(
                record,
                AgentEventKind::SessionTitleUpdated { title },
                self.inner.limits,
            )
        });
        let automatic_title = record
            .settings
            .title
            .as_ref()
            .filter(|title| title.source == AgentSessionTitleSource::Automatic)
            .map(|title| title.value.clone());
        Ok(StagedAdmission {
            staged: StagedPersistence::commit(cell, record, previous),
            admitted: AdmittedTurn {
                identity,
                origin,
                automatic_title,
                wake_events,
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
pub(in crate::service) struct AdmittedTurn {
    pub(super) identity: TurnIdentity,
    origin: hanafubuki_agent_contracts::AgentTurnOrigin,
    /// The session's title after the admission, when the Agent gave it.
    automatic_title: Option<String>,
    /// The wake events the admission delivered.
    pub(in crate::service) wake_events: Vec<crate::AgentWakeEvent>,
}

/// What an admission opens its turn with, and what the turn runs.
struct OpenedTurn {
    implementing_plan: bool,
    kind: TurnKind,
    items: Vec<ConversationItem>,
    /// `items` carry every queued wake event, which the admission takes.
    delivers_wake_events: bool,
}

/// Checks `admission` against the session and builds its turn's first
/// items: `TurnStart` and the world state that changed, then the queued wake
/// events when the policy delivers them (never in a `!command`'s turn, which
/// samples no model), then the admission's opener (Skills and the user
/// message; a wake turn has none, so its request ends with the user-role
/// events).
fn admission_items(
    record: &SessionRecord,
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
    input: crate::AgentTurnInput,
    admission: TurnAdmission,
) -> Result<OpenedTurn, AgentError> {
    let delivering = record.runtime.wake_policy == crate::AgentWakePolicy::On;
    let wake = (delivering && !record.runtime.wake_queue.is_empty()).then(|| {
        ConversationItem::WakeEvents {
            events: record.runtime.wake_queue.events.clone(),
        }
    });
    let (implementing_plan, kind) = match admission {
        TurnAdmission::ImplementPlan(request) => {
            validate_plan_implementation(record, session_id, &request)?;
            (true, TurnKind::Regular)
        }
        TurnAdmission::Regular => (false, TurnKind::Regular),
        TurnAdmission::Wake if !delivering => {
            return Err(AgentError::InvalidInput(
                "wake events are not delivered in this session".to_string(),
            ));
        }
        TurnAdmission::Wake if wake.is_none() => return Err(AgentError::NoWakeEvents),
        TurnAdmission::Wake => {
            let mut items =
                turn_start_items(turn_id, &record.conversation.history, input.world_state)?;
            items.push(wake.expect("a wake turn has events"));
            return Ok(OpenedTurn {
                implementing_plan: false,
                kind: TurnKind::Regular,
                items,
                delivers_wake_events: true,
            });
        }
        TurnAdmission::UserShellCommand(command) => {
            if record.settings.config.mode == crate::AgentSessionMode::Plan {
                return Err(AgentError::InvalidInput(
                    "`!` commands do not run in Plan mode, whose tools only read".to_string(),
                ));
            }
            let items = vec![
                ConversationItem::TurnStart { turn_id },
                unfinished_user_shell_command(turn_id, &command),
            ];
            return Ok(OpenedTurn {
                implementing_plan: false,
                kind: TurnKind::UserShellCommand(command),
                items,
                delivers_wake_events: false,
            });
        }
    };
    if !input.images.is_empty() && !record.settings.effective_input_capabilities.image_input {
        return Err(AgentError::InvalidInput(
            "the configured model does not support image input".to_string(),
        ));
    }
    let mut items = resolve_turn_items(turn_id, &record.conversation.history, input)?;
    let delivers_wake_events = wake.is_some();
    if let Some(wake) = wake {
        // After the turn's start and its world state: the events are what
        // the person's message follows.
        let at = items
            .iter()
            .position(|item| {
                !matches!(
                    item,
                    ConversationItem::TurnStart { .. } | ConversationItem::WorldStateUpdate { .. }
                )
            })
            .unwrap_or(items.len());
        items.insert(at, wake);
    }
    Ok(OpenedTurn {
        implementing_plan,
        kind,
        items,
        delivers_wake_events,
    })
}

fn log_selected_skills(session_id: AgentSessionId, items: &[ConversationItem]) {
    let selected_skill_count = items
        .iter()
        .filter(|item| matches!(item, ConversationItem::SkillContext { .. }))
        .count();
    for item in items {
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
