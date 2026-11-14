use super::*;

impl AgentService {
    pub fn session_workspace(
        &self,
        session_id: AgentSessionId,
    ) -> Result<crate::AgentSessionWorkspace, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok(record.lineage.workspace.clone())
    }

    /// Persists a workspace change between this session's operations.
    pub fn set_session_workspace(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        workspace: crate::AgentSessionWorkspace,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        ensure_no_staged_commit(record)?;
        ensure_head_fork_idle(record)?;
        if has_active_operation(record) {
            return Err(AgentError::CompactionAlreadyRunning);
        }
        if record.status.generation != generation {
            return Err(AgentError::StaleGeneration);
        }
        if record.status.revision != revision.value() {
            return Err(AgentError::StaleRevision);
        }
        if record.lineage.workspace == workspace {
            return Ok(snapshot(session_id, record));
        }
        let previous = record.rollback_state();
        record.lineage.workspace = workspace;
        record.status.revision = record
            .status
            .revision
            .checked_add(1)
            .expect("agent snapshot revision exhausted");
        record.status.updated_at = now_millis();
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        self.snapshot(session_id)
    }
}
