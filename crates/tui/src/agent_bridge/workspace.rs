//! `/cd`, `/worktree` and `/fork`'s worktree choice onto the facade: the
//! standalone runtime's `set_working_directory`, and the worktree calls
//! (`worktree_create`, `worktree_list`, `worktree_bind`,
//! `worktree_remove`), which the `worktree` plugin row serves. Moving the
//! working directory blocks, so it runs on a thread of its own.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hanafubuki_agent::{
    AgentError, AgentHandle, AgentPluginState, AgentRuntime, AgentSessionId,
    AgentSessionRuntimeSnapshot,
};

use super::commands::no_session;
use super::sessions::fork_at_head;
use super::{AgentBridge, AgentReply, EventSink, SessionReply, TurnUpdate};
use crate::app_event::AppEvent;

/// The plugin row that serves the worktree calls.
const WORKTREE_ROW: &str = "worktree";

/// A new worktree the working directory moved into.
#[derive(Debug)]
pub(crate) struct EnteredWorktree {
    path: PathBuf,
    roots: Vec<PathBuf>,
    /// The fork of the session that continues there; `None` when the next
    /// session the interface creates starts there.
    fork: Option<AgentSessionId>,
}

impl AgentBridge {
    /// Makes `path` the working directory, off the interface's thread.
    pub(super) fn change_directory(&mut self, path: PathBuf) {
        self.pending_worktree = None;
        let runtime = self.runtime.clone();
        self.blocking_session_call("cd", move |_| {
            runtime
                .set_working_directory(&path)
                .map(|roots| SessionReply::DirectoryChanged(roots.into_vec()))
                .map_err(|error| format!("the working directory was not changed: {error}"))
        });
    }

    /// Why no worktree can be made here, if none can: the working
    /// directory is in no Git repository, or the `worktree` row does not
    /// run.
    pub(super) fn worktrees_unavailable(&self) -> Option<String> {
        let root = self.workspace_roots().into_iter().next();
        match &root {
            Some(root) if in_git_repository(root) => {}
            Some(root) => {
                return Some(format!(
                    "{} is not in a Git repository; a worktree is made from the repository the \
                     working directory is in",
                    root.display()
                ))
            }
            None => return Some("there is no working directory".to_string()),
        }
        let row = self
            .agent
            .plugin_status()
            .into_iter()
            .find(|status| status.name == WORKTREE_ROW);
        match row {
            Some(row) if row.state == AgentPluginState::Active => None,
            Some(row) => {
                let state = format!("{:?}", row.state).to_lowercase();
                Some(match row.error {
                    Some(error) => {
                        format!(
                            "worktrees are unavailable: the worktree plugin is {state}: {error}"
                        )
                    }
                    None => format!("worktrees are unavailable: the worktree plugin is {state}"),
                })
            }
            None => Some("worktrees are unavailable: there is no worktree plugin".to_string()),
        }
    }

    pub(super) fn worktree_menu(&self) -> Result<TurnUpdate, String> {
        if let Some(reason) = self.worktrees_unavailable() {
            return Err(reason);
        }
        Ok(TurnUpdate::Session(SessionReply::WorktreeMenu {
            conversation: self.session_id.is_some(),
        }))
    }

    /// Makes a worktree and moves into it, off the interface's thread:
    /// with `continued`, a fork of the session (named `name`) goes on there;
    /// else the next session the interface creates starts there.
    pub(super) fn enter_new_worktree(
        &mut self,
        continued: bool,
        name: Option<String>,
    ) -> Result<(), String> {
        let source = if continued {
            Some(self.current()?)
        } else {
            None
        };
        let agent = self.agent.clone();
        let runtime = self.runtime.clone();
        let app_event_tx = self.app_event_tx.clone();
        std::thread::spawn(move || {
            let sink = EventSink(app_event_tx.clone());
            let reply = match enter_worktree(&agent, &runtime, &sink, source, name) {
                Ok(entered) => AgentReply::EnteredWorktree(entered),
                Err(error) => {
                    AgentReply::Update(Box::new(TurnUpdate::Session(SessionReply::Failed {
                        command: "worktree",
                        error,
                    })))
                }
            };
            app_event_tx.send(AppEvent::AgentReply(reply));
        });
        Ok(())
    }

    /// The interface is in the new worktree: the fork is shown, or no
    /// session until the next one, which is bound to the worktree.
    pub(super) fn entered_worktree(&mut self, entered: EnteredWorktree) -> Option<TurnUpdate> {
        self.pending_title = None;
        match entered.fork {
            Some(fork) => {
                let opened = self
                    .open(fork)
                    .map(|opened| self.reply(AgentReply::SessionOpened(opened)));
                let shown = match opened {
                    Ok(Some(update)) => AgentReply::Update(Box::new(update)),
                    Ok(None) => return None,
                    Err(error) => AgentReply::Failed(error),
                };
                self.app_event_tx.send(AppEvent::AgentReply(shown));
            }
            None => {
                self.pending_worktree = Some(entered.path.clone());
                let model = self.model.clone();
                self.spawn(|agent| async move {
                    Ok(Some(AgentReply::SessionOpened(
                        no_session(&agent, model).await,
                    )))
                });
            }
        }
        Some(TurnUpdate::Session(SessionReply::WorktreeEntered {
            path: entered.path,
            roots: entered.roots,
            continued: entered.fork.is_some(),
        }))
    }

    pub(super) fn list_worktrees(&self) {
        self.spawn_session_call("worktree", |agent| async move {
            agent
                .worktree_list()
                .await
                .map(SessionReply::Worktrees)
                .map_err(|error| worktree_error("the worktrees were not listed", error))
        });
    }

    pub(super) fn remove_worktree(&self, path: PathBuf) {
        self.spawn_session_call("worktree", |agent| async move {
            agent
                .worktree_remove(path.clone())
                .await
                .map(|()| SessionReply::WorktreeRemoved(path))
                .map_err(|error| worktree_error("the worktree was not removed", error))
        });
    }
}

/// Makes the worktree, forks the session into it when there is one, and
/// moves the working directory there. A worktree made before a later step
/// failed is kept, and the failure says where (Codex's retained checkout).
fn enter_worktree(
    agent: &AgentHandle,
    runtime: &Arc<AgentRuntime>,
    sink: &EventSink,
    source: Option<AgentSessionRuntimeSnapshot>,
    name: Option<String>,
) -> Result<EnteredWorktree, String> {
    let worktree = block_on(
        agent,
        |agent| async move { agent.worktree_create(None).await },
    )?
    .map_err(|error| worktree_error("no worktree was made", error))?;
    let path = worktree.path;
    let kept = |error: String| {
        format!(
            "{error}; the new worktree {} is kept (/worktree, Browse worktrees, removes it)",
            path.display()
        )
    };
    let fork = match source {
        Some(session) => {
            let (fork, _) = fork_at_head(agent, sink, &session, name).map_err(kept)?;
            let bound = path.clone();
            block_on(agent, |agent| async move {
                agent.worktree_bind(bound, fork).await
            })
            .map_err(kept)?
            .map_err(|error| kept(worktree_error("the fork was not bound to it", error)))?;
            Some(fork)
        }
        None => None,
    };
    let roots = runtime
        .set_working_directory(&path)
        .map_err(|error| kept(format!("the working directory was not changed: {error}")))?;
    Ok(EnteredWorktree {
        path,
        roots: roots.into_vec(),
        fork,
    })
}

/// Runs `call` on the Agent's runtime and waits for it, from a thread of
/// the interface's own.
fn block_on<T, F>(agent: &AgentHandle, call: impl FnOnce(AgentHandle) -> F) -> Result<T, String>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    agent
        .runtime()
        .block_on(call(agent.clone()))
        .map_err(|failure| failure.to_string())
}

/// A worktree call's failure; without the `worktree` row, why it is down.
fn worktree_error(what: &str, error: AgentError) -> String {
    match error {
        AgentError::FeatureUnavailable { reason, .. } => {
            format!("worktrees are unavailable: {reason}")
        }
        error => format!("{what}: {error}"),
    }
}

/// Whether `directory` or a directory above it holds `.git` (a directory,
/// or a worktree's file).
fn in_git_repository(directory: &Path) -> bool {
    directory
        .ancestors()
        .any(|directory| directory.join(".git").exists())
}
