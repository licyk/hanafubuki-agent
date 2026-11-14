use hanafubuki_agent_execution::{
    AgentBackgroundTerminalSnapshot, AgentBackgroundTerminalStopOutcome, AgentExecutionScope,
};
use hanafubuki_agent_kernel::{
    AgentActivitySnapshot, AgentError, AgentExecutionProgressStatus, AgentSessionId,
    AgentToolProgressUpdate,
};

use super::AgentSessionDriver;

impl AgentSessionDriver {
    pub(crate) fn quiesce_background_terminals(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let projected = self.snapshot(session_id)?.active_activities;
        let scope = AgentExecutionScope::for_agent_session(session_id.value())
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let outcome = self.executions.stop_background_terminals_confirmed(&scope);
        self.publish_background_terminal_stop(session_id, &projected, &outcome)?;
        if outcome.retained.is_empty() {
            Ok(())
        } else {
            Err(AgentError::SessionMutationNotConfirmed)
        }
    }

    pub(crate) fn publish_background_terminal_stop(
        &self,
        session_id: AgentSessionId,
        projected: &[AgentActivitySnapshot],
        outcome: &AgentBackgroundTerminalStopOutcome,
    ) -> Result<(), AgentError> {
        let mut progress_error = None;
        for disposed in &outcome.disposed {
            let (status, exit_code) = projected
                .iter()
                .filter_map(|activity| activity.execution.as_ref())
                .find(|execution| {
                    execution.execution_id == disposed.execution_id.value()
                        && execution.generation == disposed.generation.value()
                })
                .map(|execution| {
                    let status = if execution.status.is_terminal() {
                        execution.status
                    } else {
                        AgentExecutionProgressStatus::Cancelled
                    };
                    (status, execution.exit_code)
                })
                .unwrap_or((AgentExecutionProgressStatus::Cancelled, None));
            record_progress_error(
                &mut progress_error,
                self.core.report_session_execution_progress(
                    session_id,
                    AgentToolProgressUpdate::ExecutionStatus {
                        execution_id: disposed.execution_id.value(),
                        generation: disposed.generation.value(),
                        status,
                        exit_code,
                        cleanup_required: false,
                    },
                ),
            );
            record_progress_error(
                &mut progress_error,
                self.core.report_session_execution_progress(
                    session_id,
                    AgentToolProgressUpdate::ExecutionDisposed {
                        execution_id: disposed.execution_id.value(),
                        generation: disposed.generation.value(),
                    },
                ),
            );
        }
        for retained in &outcome.retained {
            record_progress_error(
                &mut progress_error,
                self.core.report_session_execution_progress(
                    session_id,
                    AgentToolProgressUpdate::ExecutionCleanupFailed {
                        execution_id: retained.execution_id.value(),
                        generation: retained.generation.value(),
                    },
                ),
            );
        }
        if let Some(error) = progress_error {
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn background_terminals(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentBackgroundTerminalSnapshot>, AgentError> {
        let scope = AgentExecutionScope::for_agent_session(session_id.value())
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        self.executions
            .list_background_terminals(&scope)
            .map_err(|_| {
                AgentError::Protocol("Background terminal query could not be completed".into())
            })
    }
}

fn record_progress_error(slot: &mut Option<AgentError>, result: Result<(), AgentError>) {
    if slot.is_none() {
        *slot = result.err();
    }
}
