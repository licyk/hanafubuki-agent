use super::*;
use crate::AgentHistoryFollowUp;

/// How a turn ends.
struct TerminalTransition {
    phase: AgentTurnPhase,
    error: Option<String>,
    reason: AgentTurnTerminalReason,
    event: AgentEventKind,
}

impl TerminalTransition {
    fn cancelled() -> Self {
        Self {
            phase: AgentTurnPhase::Cancelled,
            error: None,
            reason: AgentTurnTerminalReason::UserInterrupted,
            event: AgentEventKind::TurnCancelled {
                reason: AgentTurnTerminalReason::UserInterrupted,
            },
        }
    }

    fn failed(error: &AgentError) -> Self {
        let message = bounded_diagnostic(error.to_string());
        let reason = terminal_reason_for_error(error);
        Self {
            phase: AgentTurnPhase::Failed,
            error: Some(message.clone()),
            reason,
            event: AgentEventKind::TurnFailed { reason, message },
        }
    }
}

/// A staged terminal commit and the events it makes visible.
struct TerminalCommit {
    staged: StagedPersistence,
    history_event: Option<AgentEvent>,
    reasoning_event: Option<AgentEvent>,
    event: AgentEvent,
}

impl AgentService {
    /// Commits the turn's completion with its final answer, which a turn
    /// that sampled no model (a `!command`'s) does not have.
    pub(super) async fn finish_completed(
        &self,
        identity: TurnIdentity,
        answer: Option<(AgentMessageId, String)>,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<AgentTurnOutcome>, AgentError> {
        let cell = self.session(identity.session_id)?;
        // What the turn's calls reported is committed before its end.
        self.flush_execution_progress(&cell).await;
        let (staged, reasoning_event, event) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let wake_pending = super::super::wake::wake_events_pending(record);
            let turn = exact_turn_mut(record, identity)?;
            if !turn.pending_steering.is_empty() {
                return Err(AgentError::SteeringPending);
            }
            // Events queued after the last step's check: one more step
            // delivers them (a `!command`'s turn samples no model).
            if wake_pending && matches!(turn.kind, TurnKind::Regular) {
                return Err(AgentError::WakeEventsPending);
            }
            let final_answer_message_id = answer.as_ref().map(|(message_id, _)| *message_id);
            if let Some((final_answer_message_id, message)) = &answer {
                if !record.conversation.history.iter().any(|item| {
                    matches!(
                        &item.content,
                        ConversationItem::AssistantMessage {
                            message_id,
                            phase: None | Some(AgentMessagePhase::FinalAnswer),
                            text,
                            ..
                        } if message_id == final_answer_message_id && text == message
                    )
                }) {
                    return Err(AgentError::Protocol(
                        "final answer message is not present in committed history".to_string(),
                    ));
                }
            }
            let turn = exact_turn_mut(record, identity)?;
            turn.phase = AgentTurnPhase::Completed;
            turn.final_answer_message_id = final_answer_message_id;
            turn.terminal_reason = None;
            record.runtime.context_window.new_context_requested = false;
            let reasoning_event =
                close_reasoning_for_terminal(record, identity, self.inner.limits)?;
            let event = record_event(
                record,
                identity,
                AgentEventKind::TurnCompleted {
                    final_answer_message_id,
                },
                self.inner.limits,
            );
            settle_turn_working_time(record, identity)?;
            (
                StagedPersistence::commit(&cell, record, previous),
                reasoning_event,
                event,
            )
        };
        self.persist_staged(staged).await?;
        cell.notify_terminal();
        if let Some(reasoning_event) = reasoning_event {
            event_sink.emit(reasoning_event);
        }
        event_sink.emit(event);
        self.notify_turn_finished(identity);
        Ok(
            answer.map(|(final_answer_message_id, final_answer)| AgentTurnOutcome {
                session_id: identity.session_id,
                turn_id: identity.turn_id,
                generation: identity.generation,
                final_answer_message_id,
                final_answer,
            }),
        )
    }

    pub(super) async fn finish_cancelled(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.finish_terminal(identity, TerminalTransition::cancelled(), event_sink)
            .await
    }

    pub(super) async fn finish_failed(
        &self,
        identity: TurnIdentity,
        error: &AgentError,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.finish_terminal(identity, TerminalTransition::failed(error), event_sink)
            .await
    }

    /// Commits the turn's end; nothing when the session or the turn is gone
    /// or the turn already ended.
    async fn finish_terminal(
        &self,
        identity: TurnIdentity,
        transition: TerminalTransition,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let Some(cell) = self.inner.sessions.find(identity.session_id) else {
            return Ok(());
        };
        self.flush_execution_progress(&cell).await;
        let terminal = {
            let Ok(mut record) = cell.lock_after_persistence().await else {
                return Ok(());
            };
            self.stage_terminal(&cell, &mut record, identity, transition)?
        };
        let Some(terminal) = terminal else {
            return Ok(());
        };
        let TerminalCommit {
            staged,
            history_event,
            reasoning_event,
            event,
        } = terminal;
        self.persist_staged(staged).await?;
        self.publish_terminal(
            &cell,
            identity,
            [history_event, reasoning_event, Some(event)],
            event_sink,
        );
        Ok(())
    }

    /// `finish_cancelled` for a drop guard, which can neither await nor
    /// wait. The cancellation is staged now, or, while a commit the dropped
    /// turn staged is in flight, as soon as that one settles; either way it
    /// goes to the history store with nothing awaiting it, and its
    /// acknowledgement publishes it, wakes the terminal waiters and emits its
    /// events, on the store's thread.
    pub(super) fn finish_cancelled_on_drop(
        &self,
        identity: TurnIdentity,
        event_sink: Arc<dyn AgentEventSink>,
    ) {
        let Some(cell) = self.inner.sessions.find(identity.session_id) else {
            return;
        };
        let service = self.clone();
        let follow_up = cell.stage_or_defer(Box::new(move |cell, record| {
            service.stage_cancellation_hand_off(cell, record, identity, event_sink)
        }));
        self.hand_off(follow_up);
    }

    /// Stages the cancellation of a dropped turn as a checkpoint that makes
    /// itself visible once durable.
    fn stage_cancellation_hand_off(
        &self,
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        identity: TurnIdentity,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Option<AgentHistoryFollowUp> {
        let terminal = self
            .stage_terminal(cell, record, identity, TerminalTransition::cancelled())
            .unwrap_or_else(|error| {
                log::warn!(
                    "agent_diag stage=turn.drop_cancel session_id={} turn_id={} generation={} outcome=error error={}",
                    identity.session_id,
                    identity.turn_id,
                    identity.generation,
                    bounded_diagnostic(error.to_string())
                );
                None
            })?;
        let TerminalCommit {
            staged,
            history_event,
            reasoning_event,
            event,
        } = terminal;
        let (service, cell) = (self.clone(), cell.clone());
        Some(staged.into_follow_up(move || {
            service.publish_terminal(
                &cell,
                identity,
                [history_event, reasoning_event, Some(event)],
                event_sink.as_ref(),
            );
        }))
    }

    /// Stages the turn's end, or nothing when the turn is gone or already
    /// ended.
    fn stage_terminal(
        &self,
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        identity: TurnIdentity,
        transition: TerminalTransition,
    ) -> Result<Option<TerminalCommit>, AgentError> {
        let previous = record.rollback_state();
        {
            let Ok(turn) = exact_turn_mut(record, identity) else {
                return Ok(None);
            };
            if turn.phase.is_terminal() {
                return Ok(None);
            }
        }
        let synthetic_outputs =
            interrupted_tool_outputs(&history_contents(&record.conversation.history))?;
        let history_event = if synthetic_outputs.is_empty() {
            None
        } else {
            let start_index = record.conversation.history.len();
            append_history(record, &synthetic_outputs);
            Some(record_event(
                record,
                identity,
                AgentEventKind::HistoryCommitted {
                    start_index,
                    item_count: synthetic_outputs.len(),
                    usage: None,
                },
                self.inner.limits,
            ))
        };
        let turn = exact_turn_mut(record, identity)?;
        turn.phase = transition.phase;
        turn.last_error = transition.error;
        turn.terminal_reason = Some(transition.reason);
        record.runtime.context_window.new_context_requested = false;
        if transition.reason == AgentTurnTerminalReason::UserInterrupted {
            super::super::compression::note_interrupt(record);
        }
        let reasoning_event = close_reasoning_for_terminal(record, identity, self.inner.limits)?;
        let event = record_event(record, identity, transition.event, self.inner.limits);
        settle_turn_working_time(record, identity)?;
        Ok(Some(TerminalCommit {
            staged: StagedPersistence::commit(cell, record, previous),
            history_event,
            reasoning_event,
            event,
        }))
    }

    /// After a terminal commit is durable: wakes the terminal waiters, emits
    /// the commit's events in order and tells the observers.
    fn publish_terminal(
        &self,
        cell: &SessionCell,
        identity: TurnIdentity,
        events: [Option<AgentEvent>; 3],
        event_sink: &dyn AgentEventSink,
    ) {
        cell.notify_terminal();
        for event in events.into_iter().flatten() {
            event_sink.emit(event);
        }
        self.notify_turn_finished(identity);
    }

    fn notify_turn_finished(&self, identity: TurnIdentity) {
        self.inner
            .registries
            .observers
            .notify(&AgentNotification::TurnFinished {
                session_id: identity.session_id,
                turn_id: identity.turn_id,
                generation: identity.generation,
            });
    }
}
