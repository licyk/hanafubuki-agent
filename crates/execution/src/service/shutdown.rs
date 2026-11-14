use super::AgentExecutionService;
use crate::model::{AgentExecutionError, AgentExecutionId, AgentExecutionScope};
use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use std::time::{Duration, Instant};

/// How often the sweep looks again at an execution another caller holds.
const HELD_POLL: Duration = Duration::from_millis(10);

impl AgentExecutionService {
    pub fn shutdown(&self, deadline: Instant) -> Vec<WorkerShutdownDiagnostic> {
        let entries = self
            .inner
            .registry
            .lock()
            .records
            .iter()
            .map(|(id, record)| (*id, record.scope.clone()))
            .collect::<Vec<_>>();
        let mut diagnostics = Vec::new();
        for (execution_id, scope) in entries {
            if Instant::now() >= deadline {
                diagnostics.push(WorkerShutdownDiagnostic {
                    worker: "agent-execution".to_string(),
                    message: "shutdown deadline elapsed with retained executions".to_string(),
                });
                break;
            }
            if let Err(message) = self.stop_for_shutdown(&scope, execution_id, deadline) {
                diagnostics.push(WorkerShutdownDiagnostic {
                    worker: format!("agent-execution-{execution_id}"),
                    message,
                });
            }
        }
        diagnostics
    }

    /// Stops the execution and confirms its end. One another caller holds
    /// (its disposal under way, as a stopped `!command`'s can still be once
    /// its turn has ended, or its process still starting) is waited for
    /// until `deadline`: gone, it was disposed of; let go of, the sweep
    /// stops it itself.
    fn stop_for_shutdown(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        deadline: Instant,
    ) -> Result<(), String> {
        loop {
            match self.stop_confirmed(scope, execution_id) {
                Ok(_) => return Ok(()),
                Err(AgentExecutionError::NotFound) => {}
                Err(error) => return Err(error.to_string()),
            }
            let Some(held) = self.held_elsewhere(execution_id) else {
                return Ok(());
            };
            if Instant::now() >= deadline {
                return Err(format!("at the shutdown deadline, {held}"));
            }
            std::thread::sleep(HELD_POLL);
        }
    }
}
