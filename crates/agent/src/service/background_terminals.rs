use super::AgentApplicationService;
use hanafubuki_agent_execution::{
    AgentBackgroundTerminalSnapshot, AgentBackgroundTerminalStopOutcome, AgentExecutionError,
    AgentExecutionScope,
};
use hanafubuki_agent_kernel::{
    AgentActivitySnapshot, AgentError, AgentExecutionProgressStatus, AgentSessionId,
    AgentToolProgressUpdate,
};

impl AgentApplicationService {
    pub fn list_background_terminals(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentBackgroundTerminalSnapshot>, AgentError> {
        self.activate_session(session_id)?;
        let scope = background_terminal_scope(session_id)?;
        self.executions
            .list_background_terminals(&scope)
            .map_err(background_terminal_error)
    }

    pub fn stop_background_terminals_confirmed(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentBackgroundTerminalStopOutcome, AgentError> {
        self.activate_session(session_id)?;
        let projected = self.core.snapshot(session_id)?.active_activities;
        let outcome = self.stop_background_terminal_authority(session_id)?;
        self.publish_background_terminal_stop(session_id, &projected, &outcome)?;
        Ok(outcome)
    }

    pub(super) fn stop_background_terminal_authority(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentBackgroundTerminalStopOutcome, AgentError> {
        let scope = background_terminal_scope(session_id)?;
        Ok(self.executions.stop_background_terminals_confirmed(&scope))
    }

    pub(super) fn publish_background_terminal_stop(
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

    pub(super) fn quiesce_background_terminals(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let outcome = self.stop_background_terminals_confirmed(session_id)?;
        if outcome.retained.is_empty() {
            Ok(())
        } else {
            Err(AgentError::SessionMutationNotConfirmed)
        }
    }

    /// Settles the execution activities the previous process left running:
    /// their processes did not survive it. A session that refuses the update
    /// is logged and keeps its activity; the others are still settled.
    pub(super) fn reconcile_recovered_execution_activities(&self) {
        let stale = self
            .core
            .list_snapshots()
            .into_iter()
            .flat_map(|session| {
                session
                    .active_activities
                    .into_iter()
                    .filter_map(move |activity| {
                        activity.execution.map(|execution| {
                            (session.id, execution.execution_id, execution.generation)
                        })
                    })
            })
            .collect::<Vec<_>>();
        for (session_id, execution_id, generation) in &stale {
            if let Err(error) = self.core.report_session_execution_progress(
                *session_id,
                AgentToolProgressUpdate::ExecutionDisposed {
                    execution_id: *execution_id,
                    generation: *generation,
                },
            ) {
                log::error!(
                    "Agent session {session_id} kept an interrupted execution activity: {error}"
                );
            }
        }
        if !stale.is_empty() {
            log::info!(
                "agent_diag stage=execution.recover outcome=interrupted count={}",
                stale.len()
            );
        }
    }
}

pub(super) fn background_terminal_scope(
    session_id: AgentSessionId,
) -> Result<AgentExecutionScope, AgentError> {
    AgentExecutionScope::for_agent_session(session_id.value())
        .map_err(|error| AgentError::InvalidInput(error.to_string()))
}

fn background_terminal_error(_error: AgentExecutionError) -> AgentError {
    AgentError::Protocol("Background terminal query could not be completed".to_string())
}

fn record_progress_error(slot: &mut Option<AgentError>, result: Result<(), AgentError>) {
    if slot.is_none() {
        *slot = result.err();
    }
}
