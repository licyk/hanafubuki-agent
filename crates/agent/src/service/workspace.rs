//! Moving a running Agent's tools to new workspace roots: what
//! `AgentRuntime::set_working_directory` asks of the running instance. The
//! sessions stay live; each learns the new roots from the `workspace_roots`
//! section of its next turn.

use hanafubuki_agent_workspace_fs::WorkspaceFsPolicy;

use super::background_terminals::background_terminal_scope;
use super::AgentApplicationService;
use crate::AgentWorkspaceRoots;

impl AgentApplicationService {
    /// Points the filesystem tools' policy, the commands' working directory
    /// and their workspace-write sandbox at `roots`. Refused, with nothing changed, while a turn, a
    /// compaction or a background terminal runs in any session: each was
    /// started, and approved, in the old workspace.
    pub(crate) fn move_workspace(&self, roots: &AgentWorkspaceRoots) -> Result<(), String> {
        if let Some(busy) = self.workspace_in_use() {
            return Err(format!("the working directory cannot change while {busy}"));
        }
        let policy =
            WorkspaceFsPolicy::new(roots.roots().to_vec(), self.workspace.protected_roots())
                .map_err(|error| error.to_string())?;
        self.files.replace_policy(policy);
        self.executions.set_workspace_roots(roots.roots().to_vec());
        log::info!(
            "agent workspace moved count={} primary={}",
            roots.roots().len(),
            roots.primary().display()
        );
        Ok(())
    }

    /// What runs in the current workspace, if anything: the first turn,
    /// compaction or background terminals found.
    fn workspace_in_use(&self) -> Option<String> {
        for session in self.core.list_snapshots() {
            let id = session.id;
            if session.turn.is_some_and(|turn| !turn.phase.is_terminal()) {
                return Some(format!("a turn runs in session {id}"));
            }
            if session
                .compaction
                .is_some_and(|compaction| !compaction.status.is_terminal())
            {
                return Some(format!("a compaction runs in session {id}"));
            }
            let running = background_terminal_scope(id)
                .ok()
                .and_then(|scope| self.executions.list_background_terminals(&scope).ok())
                .map_or(0, |terminals| {
                    terminals
                        .iter()
                        .filter(|terminal| !terminal.status.is_terminal())
                        .count()
                });
            if running > 0 {
                return Some(format!(
                    "{running} background terminal(s) run in session {id}"
                ));
            }
        }
        None
    }
}

/// Host request to change one conversation's authoritative directory.
#[derive(Clone, Debug)]
pub struct AgentSessionWorkspaceRequest {
    pub session_id: hanafubuki_agent_kernel::AgentSessionId,
    pub generation: hanafubuki_agent_kernel::AgentGeneration,
    pub revision: hanafubuki_agent_kernel::AgentRevision,
    pub path: String,
}

impl AgentApplicationService {
    pub async fn create_session_with_workspace(
        &self,
        mode: hanafubuki_agent_kernel::AgentSessionMode,
        role: Option<String>,
        path: String,
    ) -> Result<hanafubuki_agent_kernel::AgentSessionRuntimeSnapshot, String> {
        let service = self.clone();
        let workspace = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent new session workspace",
            move || service.resolve_session_workspace(&path),
        )
        .await??;
        self.create_session_in(mode, role, Some(workspace)).await
    }

    pub fn set_session_workspace(
        &self,
        request: AgentSessionWorkspaceRequest,
    ) -> Result<hanafubuki_agent_kernel::AgentSessionRuntimeSnapshot, String> {
        self.activate_session(request.session_id)
            .map_err(|error| error.to_string())?;
        let workspace = self.resolve_session_workspace(&request.path)?;
        let scope =
            background_terminal_scope(request.session_id).map_err(|error| error.to_string())?;
        if self
            .executions
            .list_background_terminals(&scope)
            .map_err(|error| error.to_string())?
            .iter()
            .any(|terminal| !terminal.status.is_terminal())
        {
            return Err(
                "the session workspace cannot change while its background terminals run"
                    .to_string(),
            );
        }
        self.core
            .set_session_workspace(
                request.session_id,
                request.generation,
                request.revision,
                workspace,
            )
            .map_err(|error| error.to_string())
    }

    pub fn session_workspace_roots(
        &self,
        session_id: hanafubuki_agent_kernel::AgentSessionId,
    ) -> Result<AgentWorkspaceRoots, String> {
        self.activate_session(session_id)
            .map_err(|error| error.to_string())?;
        self.driver
            .workspace_roots(session_id)
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn session_files(
        &self,
        session_id: hanafubuki_agent_kernel::AgentSessionId,
    ) -> Result<hanafubuki_agent_workspace_fs::WorkspaceFsService, String> {
        let service = self.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session file policy",
            move || {
                service
                    .files
                    .with_workspace_roots(service.session_workspace_roots(session_id)?.into_vec())
                    .map_err(|error| error.to_string())
            },
        )
        .await?
    }
}

impl AgentApplicationService {
    fn resolve_session_workspace(
        &self,
        path: &str,
    ) -> Result<hanafubuki_agent_kernel::AgentSessionWorkspace, String> {
        let path = hanafubuki_agent_foundation::paths::ResolvedPath::resolve(path)
            .map_err(|error| error.to_string())?;
        WorkspaceFsPolicy::new(
            vec![path.native().to_path_buf()],
            self.workspace.protected_roots(),
        )
        .map_err(|error| error.to_string())?;
        let mut workspace = crate::session_workspace::session_workspace(path.native());
        workspace.execution_root = Some(path.native_text().to_string());
        Ok(workspace)
    }
}
