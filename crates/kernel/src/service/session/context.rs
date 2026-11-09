use super::*;

impl AgentService {
    /// Commits `items`, continuation context only, to the session's history
    /// between turns, for its model to read at its next turn. Refused while
    /// a turn or a compaction runs. Off the async runtime only.
    pub fn add_continuation_context(
        &self,
        session_id: AgentSessionId,
        items: Vec<ConversationItem>,
    ) -> Result<(), AgentError> {
        if items.is_empty()
            || items
                .iter()
                .any(|item| !matches!(item, ConversationItem::ContinuationContext { .. }))
        {
            return Err(AgentError::InvalidInput(
                "only continuation context is added between turns".to_string(),
            ));
        }
        self.commit_between_turns(session_id, |_| Ok(items))
    }

    /// Commits what `items` makes of the session's record, while no turn and
    /// no compaction runs. Off the async runtime only.
    pub(in crate::service) fn commit_between_turns(
        &self,
        session_id: AgentSessionId,
        items: impl FnOnce(&SessionRecord) -> Result<Vec<ConversationItem>, AgentError>,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let (staged, event) = {
            let mut guard = cell.lock_after_persistence_blocking()?;
            let record = &mut *guard;
            ensure_no_staged_commit(record)?;
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
            let items = items(record)?;
            validate_history_append(record, &items, self.inner.limits)?;
            let previous = record.rollback_state();
            let start_index = record.conversation.history.len();
            append_history(record, &items);
            let event = record_session_event(
                record,
                AgentEventKind::HistoryCommitted {
                    start_index,
                    item_count: items.len(),
                    usage: None,
                },
                self.inner.limits,
            );
            (StagedPersistence::commit(&cell, record, previous), event)
        };
        self.persist_staged_blocking(staged)?;
        self.emit_session_event(session_id, event);
        Ok(())
    }
}
