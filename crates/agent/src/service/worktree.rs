//! The worktree calls: Git worktrees of the repository the Agent works in,
//! served by the `worktree` plugin row (`hanafubuki-agent-plugin-worktree`)
//! through the feature helper. A standalone consumer moves into one with
//! `AgentRuntime::set_working_directory`.

use std::path::PathBuf;
use std::sync::Arc;

use hanafubuki_agent_kernel::{AgentError, AgentSessionId};
use hanafubuki_agent_plugin_worktree::{WorktreeService, WORKTREE_PLUGIN};
use hanafubuki_agent_worktree::Worktree;
use serde::Serialize;

use super::AgentApplicationService;

/// A worktree under the data directory's `worktrees/`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWorktree {
    /// Its root.
    pub path: PathBuf,
    /// The commit checked out.
    pub head: String,
    /// The branch checked out; `None` while detached, as a new worktree is.
    pub branch: Option<String>,
    /// The session it belongs to.
    pub owner: Option<AgentSessionId>,
}

impl AgentApplicationService {
    /// A new worktree of the repository the working directory (the first
    /// workspace root) is in, detached at its `HEAD`, and bound to `owner`
    /// when there is one; a worktree that could not be bound is removed.
    pub async fn worktree_create(
        &self,
        owner: Option<AgentSessionId>,
    ) -> Result<AgentWorktree, AgentError> {
        let source = self.working_directory().await?;
        self.feature(
            WORKTREE_PLUGIN,
            |worktrees: Arc<WorktreeService>| async move {
                let created = worktrees
                    .create(source.clone())
                    .await
                    .map_err(worktree_error)?;
                if let Some(owner) = owner {
                    let path = created.path.clone();
                    if let Err(error) = worktrees.bind(path.clone(), owner.to_string()).await {
                        let removed = worktrees.remove(source, path.clone()).await;
                        return Err(worktree_error(match removed {
                            Ok(()) => error,
                            Err(removal) => format!(
                                "{error}; the worktree {} was not removed: {removal}",
                                path.display()
                            ),
                        }));
                    }
                }
                Ok(AgentWorktree {
                    owner,
                    ..agent_worktree(created)
                })
            },
        )
        .await
    }

    /// The worktrees of the repository the working directory is in.
    pub async fn worktree_list(&self) -> Result<Vec<AgentWorktree>, AgentError> {
        let source = self.working_directory().await?;
        self.feature(
            WORKTREE_PLUGIN,
            |worktrees: Arc<WorktreeService>| async move {
                let listed = worktrees.list(source).await.map_err(worktree_error)?;
                Ok(listed.into_iter().map(agent_worktree).collect())
            },
        )
        .await
    }

    /// Binds the worktree at `path` to `owner`; one another session owns is
    /// refused.
    pub async fn worktree_bind(
        &self,
        path: PathBuf,
        owner: AgentSessionId,
    ) -> Result<(), AgentError> {
        self.feature(
            WORKTREE_PLUGIN,
            |worktrees: Arc<WorktreeService>| async move {
                worktrees
                    .bind(path, owner.to_string())
                    .await
                    .map_err(worktree_error)
            },
        )
        .await
    }

    /// Removes the worktree at `path`, local changes and all; refused while
    /// the working directory is inside it.
    pub async fn worktree_remove(&self, path: PathBuf) -> Result<(), AgentError> {
        let source = self.working_directory().await?;
        self.feature(
            WORKTREE_PLUGIN,
            |worktrees: Arc<WorktreeService>| async move {
                worktrees.remove(source, path).await.map_err(worktree_error)
            },
        )
        .await
    }

    /// The first workspace root, read on the blocking pool as the port
    /// requires.
    async fn working_directory(&self) -> Result<PathBuf, AgentError> {
        let workspace = self.workspace.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent workspace roots",
            move || workspace.roots(),
        )
        .await
        .and_then(|roots| roots)
        .map(|roots| roots.primary().clone())
        .map_err(AgentError::InvalidInput)
    }
}

fn agent_worktree(worktree: Worktree) -> AgentWorktree {
    AgentWorktree {
        path: worktree.path,
        head: worktree.head,
        branch: worktree.branch,
        // An owner that is not a session number names no session.
        owner: worktree
            .owner
            .and_then(|owner| owner.parse().ok())
            .map(AgentSessionId::new),
    }
}

fn worktree_error(message: String) -> AgentError {
    AgentError::InvalidInput(message)
}
