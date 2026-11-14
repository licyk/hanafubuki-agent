//! The lineage that permits delegation and identifies its shared budget.

use hanafubuki_agent_kernel::{AgentError, AgentSessionId, AgentSessionOrigin};

use crate::session_driver::AgentSessionDriver;

pub(super) const MAXIMUM_DEPTH: usize = 2;

pub(super) struct Lineage {
    pub(super) root: AgentSessionId,
    pub(super) depth: usize,
}

pub(super) fn lineage(
    driver: &AgentSessionDriver,
    session_id: AgentSessionId,
) -> Result<Lineage, AgentError> {
    let mut current = session_id;
    let mut depth = 0;
    loop {
        let session = driver.snapshot(current)?;
        if matches!(
            session.origin,
            Some(AgentSessionOrigin::Side | AgentSessionOrigin::Review)
        ) {
            return Err(AgentError::InvalidInput(
                "this conversation cannot spawn agents".to_string(),
            ));
        }
        let Some(parent) = session.parent_session_id else {
            return Ok(Lineage {
                root: current,
                depth,
            });
        };
        if session.origin != Some(AgentSessionOrigin::Subagent) {
            return Err(AgentError::InvalidInput(
                "this conversation cannot spawn agents".to_string(),
            ));
        }
        depth += 1;
        current = parent;
    }
}

pub(super) fn spawn_lineage(
    driver: &AgentSessionDriver,
    session_id: AgentSessionId,
) -> Result<Lineage, AgentError> {
    let lineage = lineage(driver, session_id)?;
    if lineage.depth >= MAXIMUM_DEPTH {
        return Err(AgentError::InvalidInput(
            "a sub-agent spawned by another sub-agent cannot spawn agents: sub-agents nest at most two levels below the main conversation; do the work yourself, or send your parent a message".to_string(),
        ));
    }
    Ok(lineage)
}
