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
