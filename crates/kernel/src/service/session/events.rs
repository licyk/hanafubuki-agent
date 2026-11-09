use crate::conversation::AgentHistoryStore;
use crate::model::{AgentError, AgentEventBatch, AgentEventSequence, AgentSessionId};
use crate::service::persistence::history_store_error;
use crate::service::AgentService;

impl AgentService {
    pub fn events_since(
        &self,
        session_id: AgentSessionId,
        after: Option<AgentEventSequence>,
    ) -> Result<AgentEventBatch, AgentError> {
        let Some(cell) = self.inner.sessions.find(session_id) else {
            self.inner
                .history_store
                .load_runtime_state(session_id)
                .map_err(history_store_error)?;
            return Ok(AgentEventBatch {
                events: Vec::new(),
                oldest_available_sequence: AgentEventSequence::new(1),
                next_sequence: AgentEventSequence::new(1),
                truncated: false,
            });
        };
        let record = cell.lock_after_persistence_blocking()?;
        let oldest = record
            .journal
            .events
            .front()
            .map(|event| event.sequence.value())
            .unwrap_or(record.journal.next_event_sequence);
        let requested = after.map_or(oldest, |sequence| sequence.value().saturating_add(1));
        Ok(AgentEventBatch {
            events: record
                .journal
                .events
                .iter()
                .filter(|event| event.sequence.value() >= requested)
                .cloned()
                .collect(),
            oldest_available_sequence: AgentEventSequence::new(oldest),
            next_sequence: AgentEventSequence::new(record.journal.next_event_sequence),
            truncated: requested < oldest,
        })
    }
}
