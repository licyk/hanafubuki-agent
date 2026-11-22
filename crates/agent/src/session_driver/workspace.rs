use super::*;

impl AgentSessionDriver {
    pub(crate) fn workspace_roots(
        &self,
        session_id: AgentSessionId,
    ) -> Result<crate::AgentWorkspaceRoots, AgentError> {
        let workspace = self.core.session_workspace(session_id)?;
        match workspace.execution_root {
            Some(cwd) => crate::AgentWorkspaceRoots::resolve_session(
                std::iter::once(cwd).chain(workspace.additional_roots),
            )
            .map_err(AgentError::InvalidInput),
            None => self.workspace.roots().map_err(AgentError::InvalidInput),
        }
    }

    pub(crate) async fn workspace_roots_async(
        &self,
        session_id: AgentSessionId,
    ) -> Result<crate::AgentWorkspaceRoots, AgentError> {
        let driver = self.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent session workspace",
            move || {
                driver.core.activate_session(session_id)?;
                driver.workspace_roots(session_id)
            },
        )
        .await
        .map_err(AgentError::InvalidInput)?
    }
}
