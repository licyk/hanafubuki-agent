use super::*;

impl AgentSessionDriver {
    pub(crate) fn workspace_roots(
        &self,
        session_id: AgentSessionId,
    ) -> Result<crate::AgentWorkspaceRoots, AgentError> {
        match self.core.session_workspace(session_id)?.execution_root {
            Some(cwd) => {
                let path = hanafubuki_agent_foundation::paths::ResolvedPath::resolve(cwd)
                    .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
                if !path.system().is_dir() {
                    return Err(AgentError::InvalidInput(
                        "session workspace is not a directory".to_string(),
                    ));
                }
                crate::agent_workspace_roots_from([path.native().to_path_buf()]).ok_or_else(|| {
                    AgentError::InvalidInput("session workspace is empty".to_string())
                })
            }
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
