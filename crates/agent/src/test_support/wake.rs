//! Wake events queued by hand: nothing in the Agent queues one yet, so a
//! test gives a session its policy and its events here.

use hanafubuki_agent_kernel::{
    AgentError, AgentSessionId, AgentWakePolicy, AgentWakeQueued, AgentWakeSource,
};

use crate::AgentApplicationService;

/// Gives `session_id` the wake `policy` (the kernel's default is off);
/// whether it changed.
pub async fn set_wake_policy(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    policy: AgentWakePolicy,
) -> Result<bool, AgentError> {
    service.core.activate_session(session_id)?;
    service
        .core
        .set_session_wake_policy(session_id, policy)
        .await
}

/// Queues `source` on `session_id`, as a sub-agent's end or a terminal's
/// would be.
pub async fn queue_wake_event(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    source: AgentWakeSource,
) -> Result<AgentWakeQueued, AgentError> {
    service.core.activate_session(session_id)?;
    service.core.enqueue_wake_event(session_id, source).await
}
