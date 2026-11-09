//! The narrow port through which the `goal` plugin reads and writes a
//! session's goal, which the kernel keeps with the session's runtime state.

use hanafubuki_agent_kernel::{
    AgentError, AgentGoal, AgentGoalChange, AgentService, AgentSessionId, AgentSessionMode,
};

/// What the `goal` plugin reads and writes of a session. Every method
/// blocks; the plugin calls them on the `BlockingService`, except
/// [`Self::goal`], which only takes the session's lock.
#[derive(Clone)]
pub(crate) struct AgentGoalApplication {
    core: AgentService,
}

/// What goal accounting reads of a session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentGoalSession {
    pub(crate) mode: AgentSessionMode,
    /// Kept in memory only, where a goal has nothing to persist to.
    pub(crate) ephemeral: bool,
    /// The session's model tokens so far, input and output.
    pub(crate) tokens: u64,
    /// When the running turn started (milliseconds since the Unix epoch);
    /// `None` while no turn runs.
    pub(crate) turn_started_at: Option<u128>,
}

impl AgentGoalApplication {
    pub(crate) fn new(core: AgentService) -> Self {
        Self { core }
    }

    pub(crate) fn goal(&self, session_id: AgentSessionId) -> Result<Option<AgentGoal>, AgentError> {
        self.core.session_goal(session_id)
    }

    pub(crate) fn session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentGoalSession, AgentError> {
        self.core.activate_session(session_id)?;
        let snapshot = self.core.snapshot(session_id)?;
        Ok(AgentGoalSession {
            mode: snapshot.mode,
            ephemeral: snapshot.ephemeral,
            tokens: snapshot.usage.map_or(0, |usage| {
                usage
                    .cumulative_input_tokens
                    .saturating_add(usage.cumulative_output_tokens)
            }),
            turn_started_at: snapshot
                .turn
                .filter(|turn| !turn.phase.is_terminal())
                .map(|turn| turn.created_at),
        })
    }

    /// Applies `update` to the session's goal as it stands under the
    /// session's lock (a compare-and-swap); returns the goal before and
    /// after.
    pub(crate) fn update(
        &self,
        session_id: AgentSessionId,
        update: impl FnOnce(Option<&AgentGoal>) -> Result<AgentGoalChange, AgentError>,
    ) -> Result<(Option<AgentGoal>, Option<AgentGoal>), AgentError> {
        self.core.update_session_goal(session_id, update)
    }
}
