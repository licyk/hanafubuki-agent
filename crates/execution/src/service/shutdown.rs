use super::AgentExecutionService;
use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use std::time::Instant;

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
            let cleanup = (|| {
                let snapshot = self.snapshot(&scope, execution_id)?;
                if !snapshot.status.is_terminal() {
                    self.terminate(&scope, execution_id)?;
                }
                self.dispose(&scope, execution_id)
            })();
            if let Err(error) = cleanup {
                diagnostics.push(WorkerShutdownDiagnostic {
                    worker: format!("agent-execution-{execution_id}"),
                    message: error.to_string(),
                });
            }
        }
        diagnostics
    }
}
