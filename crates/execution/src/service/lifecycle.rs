use super::AgentExecutionService;
use crate::model::{
    AgentExecutionError, AgentExecutionId, AgentExecutionScope, AgentExecutionSnapshot,
    ConfirmedAgentExecutionDisposal,
};
use std::time::Instant;

impl AgentExecutionService {
    pub fn terminate(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
    ) -> Result<AgentExecutionSnapshot, AgentExecutionError> {
        let started = Instant::now();
        log::info!("agent_diag stage=execution.terminate.begin execution_id={execution_id}");
        let receipt = self.receipt_for_scope(scope, execution_id)?;
        self.inner
            .process
            .terminate(&receipt)
            .map_err(|error| self.retain_cleanup_error(execution_id, error))?;
        let result = self.snapshot(scope, execution_id);
        log::info!(
            "agent_diag stage=execution.terminate.end execution_id={execution_id} outcome={} elapsed_ms={}",
            if result.is_ok() { "ok" } else { "error" },
            started.elapsed().as_millis()
        );
        result
    }

    pub fn dispose(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
    ) -> Result<ConfirmedAgentExecutionDisposal, AgentExecutionError> {
        let dispose_started = Instant::now();
        log::info!("agent_diag stage=execution.dispose.begin execution_id={execution_id}");
        let receipt = self.receipt_for_scope(scope, execution_id)?;
        if let Err(error) = self.inner.process.dispose_confirmed(&receipt) {
            return Err(self.retain_cleanup_error(execution_id, error));
        }
        let mut registry = self.inner.registry.lock();
        let record = registry
            .records
            .get(&execution_id)
            .ok_or(AgentExecutionError::NotFound)?;
        if &record.scope != scope || record.receipt.as_ref() != Some(&receipt) {
            return Err(AgentExecutionError::CrossScope);
        }
        let generation = record.generation;
        registry.records.remove(&execution_id);
        log::info!(
            "agent_diag stage=execution.dispose.end execution_id={execution_id} outcome=ok elapsed_ms={}",
            dispose_started.elapsed().as_millis()
        );
        Ok(ConfirmedAgentExecutionDisposal {
            execution_id,
            generation,
        })
    }
}
