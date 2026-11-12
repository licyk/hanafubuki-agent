//! The command center's actions on a listed session, not the one shown:
//! forking, renaming and deleting it (and its children), interrupting its
//! turn or a child's. Each reads the session's generation and revision as
//! they are now.

use hanafubuki_agent::{
    AgentSessionId, AgentSessionOrigin, AgentSessionRuntimeSnapshot, AgentSessionSummary,
    AgentSessionTitleRenameRequest,
};

use super::sessions::fork_at_head;
use super::AgentBridge;

impl AgentBridge {
    /// A fork of `session_id` at its head.
    pub(crate) fn fork_listed(&self, session_id: AgentSessionId) -> Result<AgentSessionId, String> {
        let session = self.listed(session_id)?;
        let sink = self.sink();
        fork_at_head(&self.agent, &*sink, &session, None).map(|(fork, _)| fork)
    }

    /// Names `session_id` `title`.
    pub(crate) fn rename_listed(
        &self,
        session_id: AgentSessionId,
        title: String,
    ) -> Result<(), String> {
        let session = self.listed(session_id)?;
        let sink = self.sink();
        self.agent
            .rename_session(
                AgentSessionTitleRenameRequest {
                    session_id: session.id,
                    generation: session.generation,
                    revision: session.revision,
                    title,
                },
                &*sink,
            )
            .map(drop)
            .map_err(|error| format!("the session was not renamed: {error}"))
    }

    /// Deletes `session_id` and its children.
    pub(crate) fn delete_listed(&self, session_id: AgentSessionId) -> Result<(), String> {
        let session = self.listed(session_id)?;
        self.agent
            .delete_session_confirmed(session.id, session.generation, session.revision)
            .map(drop)
            .map_err(|error| format!("the session was not deleted: {error}"))
    }

    /// Interrupts the turn `session` runs: a subagent's through the
    /// subagents' own call, which keeps it taking follow-up tasks. A
    /// failure comes back as a reply.
    pub(crate) fn stop_listed(&self, session: &AgentSessionSummary) -> Result<(), String> {
        if session.origin != Some(AgentSessionOrigin::Subagent) {
            return self.interrupt_listed(session.session_id);
        }
        let child = session.session_id;
        self.spawn(move |agent| async move {
            agent
                .interrupt_subagent(child)
                .await
                .map(|()| None)
                .map_err(|error| format!("the subagent was not interrupted: {error}"))
        });
        Ok(())
    }

    fn interrupt_listed(&self, session_id: AgentSessionId) -> Result<(), String> {
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
