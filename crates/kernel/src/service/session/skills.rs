use super::*;

impl AgentService {
    pub fn active_skills(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<hanafubuki_agent_skills::SkillId>, AgentError> {
        let cell = self.session(session_id)?;
        let guard = cell.lock()?;
        let record = &*guard;
        Ok(super::tool_execution::active_skills(
            &record.conversation.history,
        ))
    }

    pub fn set_session_mode(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        mode: crate::AgentSessionMode,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        ensure_no_staged_commit(record)?;
        if record.status.generation != generation || record.status.revision != revision.value() {
            return Err(AgentError::StaleRevision);
        }
        if record.status.disposing
            || record
                .runtime
                .turn
                .as_ref()
                .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::SessionMutationNotConfirmed);
        }
        if record
            .runtime
            .compaction
            .as_ref()
            .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
        {
            return Err(AgentError::CompactionAlreadyRunning);
        }
        if record.settings.config.mode == mode {
            return Ok(snapshot(session_id, record));
        }
        let previous = record.rollback_state();
        record.settings.config.mode = mode;
        record.status.revision = record
            .status
            .revision
            .checked_add(1)
            .expect("agent session revision exhausted");
        record.status.updated_at = now_millis();
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        self.snapshot(session_id)
    }
}
