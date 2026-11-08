use super::*;

impl AgentExecutionService {
    /// Register the reporter that streams this execution's output to the
    /// Agent activity. Disposing the execution once it finishes waits (at most
    /// [`OBSERVER_DRAIN_TIMEOUT`]) for the reporter to release it, so the final
    /// output and status are reported before the execution is gone.
    pub fn attach_observer(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        generation: AgentExecutionGeneration,
    ) -> Result<(), AgentExecutionError> {
        let mut registry = self.inner.registry.lock();
        let record = registry
            .records
            .get_mut(&execution_id)
            .ok_or(AgentExecutionError::NotFound)?;
        if &record.scope != scope {
            return Err(AgentExecutionError::CrossScope);
        }
        if record.generation != generation {
            return Err(AgentExecutionError::StaleGeneration);
        }
        record.observed = true;
        Ok(())
    }

    /// The reporter has reported everything it will; a pending disposal may
    /// proceed.
    pub fn release_observer(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        generation: AgentExecutionGeneration,
    ) {
        if let Some(record) = self
            .inner
            .registry
            .lock()
            .records
            .get_mut(&execution_id)
            .filter(|record| &record.scope == scope && record.generation == generation)
        {
            record.observed = false;
        }
        self.inner.observer_released.notify_all();
    }

    pub(super) fn await_observer_release(&self, execution_id: AgentExecutionId) {
        let deadline = Instant::now() + OBSERVER_DRAIN_TIMEOUT;
        let mut registry = self.inner.registry.lock();
        while registry
            .records
            .get(&execution_id)
            .is_some_and(|record| record.observed)
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                log::warn!(
                    "agent_diag stage=unified_exec.observer_drain execution_id={execution_id} outcome=timeout"
                );
                return;
            }
            self.inner
                .observer_released
                .wait_for(&mut registry, remaining);
        }
    }
}
