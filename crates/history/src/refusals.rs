//! Sessions the history store refuses while it runs.
//!
//! What `open` refuses reaches the Agent status through recovery's report.
//! A session refused later (a row the session list cannot decode) is told
//! to the sink its owner injected, once, when it is first refused.

use hanafubuki_agent_kernel::AgentQuarantinedSession;

/// Told of each session the store refuses after it opened. Called on the
/// thread of the read that refused it, so it only hands the report on.
pub trait AgentHistoryRefusalSink: Send + Sync {
    fn session_refused(&self, session: AgentQuarantinedSession);
}

/// Refusals are only logged.
pub struct NoopAgentHistoryRefusalSink;

impl AgentHistoryRefusalSink for NoopAgentHistoryRefusalSink {
    fn session_refused(&self, _session: AgentQuarantinedSession) {}
}
