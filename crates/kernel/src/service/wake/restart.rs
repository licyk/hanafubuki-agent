//! Startup and shutdown reports stay queued until the next user turn.

use super::*;

impl AgentService {
    /// Records work lost at a stop, while no turn can consume it. Recovery
    /// has not assigned wake policies yet, and neither path notifies the
    /// dispatcher. Off the async runtime, like recovery and shutdown.
    pub fn record_lost_background_work(
        &self,
        session_id: AgentSessionId,
        sources: Vec<AgentWakeSource>,
    ) -> Result<(), AgentError> {
        if sources.is_empty() {
            return Ok(());
        }
        let cell = self.session(session_id)?;
        let staged = {
            let mut record = cell.lock_after_persistence_blocking()?;
            if record.status.lifecycle == AgentSessionLifecycle::Archived {
                return Ok(());
            }
            let previous = record.rollback_state();
            for source in sources {
                record.runtime.wake_queue.push(source, now_millis_u64());
            }
            StagedPersistence::commit(&cell, &mut record, previous)
        };
        self.persist_staged_blocking(staged)
    }
}
