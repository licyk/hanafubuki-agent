//! What a session's background work holds open: its sub-agents still
//! working, two levels deep, and its own background terminals still
//! running (`terminal.rs`). Only loaded sessions are read: one that runs
//! a turn is loaded, and one that is not runs nothing.

use super::*;
use crate::{AgentBackgroundWork, AgentSessionOrigin};

/// How many levels of sub-agents below a session are counted.
const COUNTED_LEVELS: usize = 2;

impl AgentService {
    /// The session's background work now. Each session is locked in turn,
    /// never two at once.
    pub fn background_work(&self, session_id: AgentSessionId) -> AgentBackgroundWork {
        AgentBackgroundWork {
            agents: self.working_subagents(session_id, COUNTED_LEVELS),
            terminals: self.background_terminals(session_id),
        }
    }

    /// The background work the session is woken for: none while its wake
    /// policy is off.
    pub(in crate::service) fn background_work_waking(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentBackgroundWork, AgentError> {
        let policy = self.session(session_id)?.lock()?.runtime.wake_policy;
        Ok(match policy {
            AgentWakePolicy::On => self.background_work(session_id),
            AgentWakePolicy::Off => AgentBackgroundWork::default(),
        })
    }

    /// The sub-agents of `session_id` that run a turn, or whose own
    /// sub-agents do, `levels` deep.
    fn working_subagents(&self, session_id: AgentSessionId, levels: usize) -> usize {
        if levels == 0 {
            return 0;
        }
        self.inner
            .sessions
            .cells()
            .into_iter()
            .filter(|cell| {
                let (child, running) = match cell.lock() {
                    Ok(record) => (
                        record.lineage.parent_session_id == Some(session_id)
                            && record.lineage.origin == Some(AgentSessionOrigin::Subagent),
                        record
                            .runtime
                            .turn
                            .as_ref()
                            .is_some_and(|turn| !turn.phase.is_terminal()),
                    ),
                    Err(_) => (false, false),
                };
                child && (running || self.working_subagents(cell.session_id(), levels - 1) > 0)
            })
            .count()
    }
}
