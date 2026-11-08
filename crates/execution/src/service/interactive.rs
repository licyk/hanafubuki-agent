use super::*;

impl AgentExecutionService {
    pub fn resize_interactive(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        generation: AgentExecutionGeneration,
        cols: u16,
        rows: u16,
    ) -> Result<InteractiveResizeAccepted, AgentExecutionError> {
        if cols == 0 || rows == 0 {
            return Err(AgentExecutionError::InvalidRequest(
                "PTY dimensions must be non-zero".into(),
            ));
        }
        let receipt = self.receipt_for_generation(scope, execution_id, generation)?;
        self.inner
            .process
            .resize(&receipt, cols, rows)
            .map_err(AgentExecutionError::Process)?;
        Ok(InteractiveResizeAccepted {
            execution_id,
            generation,
            cols,
            rows,
        })
    }
}
