use crate::store::*;

impl FileAgentHistoryStore {
    pub(in crate::store) fn try_claim_session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<bool, AgentHistoryStoreError> {
        let mut owners = self.inner.session_owners.lock();
        if owners.contains_key(&session_id) {
            return Ok(true);
        }
        let path = self
            .inner
            .paths
            .root
            .join("writers")
            .join(format!("{session_id}.lock"));
        let Some(owner) = FileLock::try_exclusive(&path).map_err(io_error)? else {
            return Ok(false);
        };
        owners.insert(session_id, Arc::new(owner));
        Ok(true)
    }

    pub(in crate::store) fn claim_session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        if self.try_claim_session(session_id)? {
            Ok(())
        } else {
            Err(AgentHistoryStoreError::Io(format!(
                "session {session_id} is in use by another Agent; exit that Agent or start a new session"
            )))
        }
    }
}
