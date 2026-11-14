use super::AgentApplicationService;
use hanafubuki_agent_execution::{
    AgentBackgroundTerminalSnapshot, AgentBackgroundTerminalStopOutcome, AgentExecutionError,
    AgentExecutionId, AgentExecutionScope,
};
use hanafubuki_agent_kernel::{
    AgentActivitySnapshot, AgentClosedBackgroundTerminal, AgentError, AgentSessionId,
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
        let terminals = self.list_background_terminals(session_id)?;
        let outcome = self.stop_background_terminal_authority(session_id)?;
        self.report_terminal_closes(session_id, &terminals, &outcome)?;
        Ok(outcome)
    }

    /// Closes one background terminal and reports the person's close to its owner.
    /// Blocks while termination and history persistence are confirmed.
    pub fn close_background_terminal(
        &self,
        session_id: AgentSessionId,
        execution_id: AgentExecutionId,
    ) -> Result<(), AgentError> {
        self.activate_session(session_id)?;
        let terminal = self
            .list_background_terminals(session_id)?
            .into_iter()
            .find(|terminal| terminal.execution_id == execution_id)
            .ok_or_else(|| {
                AgentError::InvalidInput(
                    "execution is not a background terminal of this session".into(),
                )
            })?;
        let disposed = self
            .executions
            .stop_confirmed(&background_terminal_scope(session_id)?, execution_id)
            .map_err(background_terminal_error)?;
        let outcome = AgentBackgroundTerminalStopOutcome {
            disposed: vec![disposed],
            retained: Vec::new(),
        };
        self.report_terminal_closes(session_id, &[terminal], &outcome)
    }

    fn report_terminal_closes(
        &self,
        session_id: AgentSessionId,
        terminals: &[AgentBackgroundTerminalSnapshot],
        outcome: &AgentBackgroundTerminalStopOutcome,
    ) -> Result<(), AgentError> {
        let sources = outcome
            .disposed
            .iter()
            .filter_map(|disposed| {
                terminals
                    .iter()
                    .find(|terminal| terminal.execution_id == disposed.execution_id)
                    .map(|terminal| AgentClosedBackgroundTerminal {
                        execution_id: terminal.execution_id.value(),
                        generation: terminal.generation.value(),
                        command: terminal.command.clone(),
                        exit_code: terminal.exit_code,
                    })
            })
            .collect();
        self.publish_background_terminal_stop(
            session_id,
            &[],
            &AgentBackgroundTerminalStopOutcome {
                disposed: Vec::new(),
                retained: outcome.retained.clone(),
            },
        )?;
        self.core
            .report_closed_background_terminals(session_id, sources)
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
        self.driver
            .publish_background_terminal_stop(session_id, projected, outcome)
    }

    pub(super) fn quiesce_background_terminals(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        self.activate_session(session_id)?;
        let projected = self.core.snapshot(session_id)?.active_activities;
        let outcome = self.stop_background_terminal_authority(session_id)?;
        self.publish_background_terminal_stop(session_id, &projected, &outcome)?;
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
