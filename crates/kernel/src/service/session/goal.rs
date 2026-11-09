use super::*;

impl AgentService {
    /// The session's goal, read without waiting for a commit in flight.
    pub fn session_goal(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<AgentGoal>, AgentError> {
        self.activate_session(session_id)?;
        let cell = self.session(session_id)?;
        let record = cell.lock()?;
        Ok(record.settings.goal.clone())
    }

    /// Applies what `update` makes of the session's goal, under the
    /// session's lock, so a change made in the meantime is what `update`
    /// sees: a compare-and-swap. Returns the goal before and after. A
    /// change is committed with the session's runtime state and announced
    /// as `SessionGoalUpdated` to the sink of the session's managed turn,
    /// if it has one.
    pub fn update_session_goal(
        &self,
        session_id: AgentSessionId,
        update: impl FnOnce(Option<&AgentGoal>) -> Result<AgentGoalChange, AgentError>,
    ) -> Result<(Option<AgentGoal>, Option<AgentGoal>), AgentError> {
        self.activate_session(session_id)?;
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        ensure_no_staged_commit(record)?;
        let before = record.settings.goal.clone();
        let after = match update(before.as_ref())? {
            AgentGoalChange::Keep => return Ok((before.clone(), before)),
            AgentGoalChange::Set(goal) => Some(goal),
            AgentGoalChange::Clear => None,
        };
        if after == before {
            return Ok((before, after));
        }
        let previous = record.rollback_state();
        record.settings.goal = after.clone();
        let event = record_session_event(
            record,
            AgentEventKind::SessionGoalUpdated {
                goal: after.clone(),
            },
            self.inner.limits,
        );
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        self.emit_session_event(session_id, event);
        Ok((before, after))
    }
}
