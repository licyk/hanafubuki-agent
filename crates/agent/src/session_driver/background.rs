use hanafubuki_agent_execution::{AgentBackgroundTerminalSnapshot, AgentExecutionScope};
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};

use super::AgentSessionDriver;

impl AgentSessionDriver {
    pub(crate) fn running_terminals(
        &self,
        session_id: AgentSessionId,
    ) -> Result<usize, AgentError> {
        self.core.activate_session(session_id)?;
        Ok(self.core.background_terminals(session_id))
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
