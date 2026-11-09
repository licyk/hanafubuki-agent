//! The command center's actions on a listed session, not the one shown:
//! forking it, deleting it (and its children), interrupting its turn. Each
//! reads the session's generation and revision as they are now.

use hanafubuki_agent::{AgentSessionId, AgentSessionRuntimeSnapshot};

use super::sessions::fork_at_head;
use super::AgentBridge;

impl AgentBridge {
    /// A fork of `session_id` at its head.
    pub(crate) fn fork_listed(&self, session_id: AgentSessionId) -> Result<AgentSessionId, String> {
        let session = self.listed(session_id)?;
        let sink = self.sink();
        fork_at_head(&self.agent, &*sink, &session, None).map(|(fork, _)| fork)
    }

    /// Deletes `session_id` and its children.
    pub(crate) fn delete_listed(&self, session_id: AgentSessionId) -> Result<(), String> {
        let session = self.listed(session_id)?;
        self.agent
            .delete_session_confirmed(session.id, session.generation, session.revision)
            .map(drop)
            .map_err(|error| format!("the session was not deleted: {error}"))
    }

    /// Interrupts the turn `session_id` runs; a failure comes back as a
    /// reply.
    pub(crate) fn interrupt_listed(&self, session_id: AgentSessionId) -> Result<(), String> {
        let turn = self
            .listed(session_id)?
            .turn
            .filter(|turn| !turn.phase.is_terminal())
            .ok_or("nothing runs there")?;
        self.spawn(move |agent| async move {
            agent
                .cancel_turn(session_id, turn.id, turn.generation)
                .await
                .map(|_| None)
                .map_err(|error| format!("the turn was not interrupted: {error}"))
        });
        Ok(())
    }

    fn listed(&self, session_id: AgentSessionId) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.agent
            .open_session(session_id, 1)
            .map(|opened| opened.runtime)
            .map_err(|error| format!("the session was not read: {error}"))
    }
}
