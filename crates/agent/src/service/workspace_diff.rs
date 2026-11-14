//! Read-only Git output for an embedded host's diff view.

use super::AgentApplicationService;
use hanafubuki_agent_foundation::blocking::run_blocking_operation;
use hanafubuki_agent_kernel::{AgentError, AgentSessionId};

impl AgentApplicationService {
    /// Staged and unstaged changes and untracked names in this session's
    /// workspace. Git processes keep the Agent's normal owner receipts.
    pub async fn diff_for_session(&self, session_id: AgentSessionId) -> Result<String, AgentError> {
        let cwd = self
            .driver
            .workspace_roots_async(session_id)
            .await?
            .primary()
            .clone();
        let queries = self.git_queries.clone();
        run_blocking_operation(self.blocking.as_ref(), "agent workspace diff", move || {
            queries.working_tree_diff(&cwd)
        })
        .await
        .map_err(AgentError::Internal)?
        .map_err(|error| AgentError::InvalidInput(error.to_string()))
    }
}
