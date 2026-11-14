//! What the status line and the terminal title show of the session
//! ([`SessionStatus`]): read from its snapshot when something it shows
//! changes (a commit's usage, a new title or plan, a compaction, the end
//! of a turn, a model or role change), with the configured permission mode and the
//! goal, which are read once when the session opens and kept. The chat also
//! reads there whether the session's model is known not to read images.

use std::path::{Path, PathBuf};

use hanafubuki_agent::{
    AgentCompressionSummary, AgentGoal, AgentPermissionMode, AgentSessionId,
    AgentTaskPlanStepStatus,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::AppEvent;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionStatus {
    pub(crate) session_id: AgentSessionId,
    pub(crate) title: Option<String>,
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) usage: Option<UsageSummary>,
    /// The latest plan's completed steps and all its steps.
    pub(crate) task_progress: Option<(usize, usize)>,
    /// The session's own mode, else the configured one once it is read.
    pub(crate) permission_mode: Option<AgentPermissionMode>,
    pub(crate) goal: Option<AgentGoal>,
    /// The branch the workspace's first root has checked out.
    pub(crate) git_branch: Option<String>,
    /// What the hierarchical compression folded, when it ran.
    pub(crate) compression: Option<AgentCompressionSummary>,
    /// The session's role; none for `default`.
    pub(crate) role: Option<String>,
    /// Why the session sends its model no image (the kernel refuses a
    /// message with one); `None` when it sends them, or nothing says.
    pub(crate) images_refused: Option<String>,
}

/// The session's tokens so far, and its context when the window is known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UsageSummary {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    /// The tokens in the context and the window's size.
    pub(crate) context: Option<(u64, u64)>,
}

/// What is read once per session and kept: the configured permission
/// mode, and the goal (which its events then carry).
#[derive(Default)]
pub(super) struct StatusCache {
    session_id: Option<AgentSessionId>,
    permission_mode: Option<AgentPermissionMode>,
    goal: Option<AgentGoal>,
}

impl AgentBridge {
    /// The session's status now; `None` with no session, or when it cannot
    /// be read (the line keeps what it shows).
    pub(super) fn session_status(&self) -> Option<TurnUpdate> {
        let session_id = self.session_id?;
        let snapshot = self.agent.snapshot(session_id).ok()?;
        let cache = Some(&self.status).filter(|cache| cache.session_id == Some(session_id));
        let permission_mode = self
            .agent
            .session_permission_mode(session_id)
            .ok()
            .flatten()
            .or_else(|| cache.and_then(|cache| cache.permission_mode));
        let task_progress = snapshot.task_plan.as_ref().map(|plan| {
            let done = plan
                .plan
                .iter()
                .filter(|step| step.status == AgentTaskPlanStepStatus::Completed)
                .count();
            (done, plan.plan.len())
        });
        let root = self.workspace_roots().into_iter().next();
        Some(TurnUpdate::SessionStatus(Box::new(SessionStatus {
            session_id,
            title: snapshot.title.map(|title| title.value),
            model: snapshot.model,
            reasoning_effort: snapshot
                .reasoning_effort
                .map(|effort| effort.as_str().to_string()),
            usage: snapshot.usage.map(|usage| UsageSummary {
                input_tokens: usage.cumulative_input_tokens,
                output_tokens: usage.cumulative_output_tokens,
                context: usage
                    .context_consumed_tokens
                    .zip(usage.context_window_tokens),
            }),
            task_progress,
            permission_mode,
            goal: cache.and_then(|cache| cache.goal.clone()),
            git_branch: root.as_deref().and_then(git_branch),
            compression: snapshot.compression,
            role: snapshot.role,
            images_refused: (!snapshot.input_capabilities.image_input)
                .then(|| self.images_refused_because(session_id)),
        })))
    }

    /// Why the session's model reads no image: where that is known from,
    /// else that the settings turn images off.
    fn images_refused_because(&self, session_id: AgentSessionId) -> String {
        self.agent
            .session_model_metadata(session_id)
            .ok()
            .and_then(|metadata| metadata.image_input)
            .filter(|known| !known.value)
            .map(|known| known.source.label().to_string())
            .unwrap_or_else(|| "turned off in the settings".to_string())
    }

    /// Sends the session's status to the chat, beside the update being
    /// handled.
    pub(super) fn send_session_status(&self) {
        if let Some(update) = self.session_status() {
            self.app_event_tx
                .send(AppEvent::AgentReply(AgentReply::Update(Box::new(update))));
        }
    }

    /// A session opened: its status now, then the configured permission
    /// mode and the goal, read off the loop. A failed read leaves them out.
    pub(super) fn load_session_status(&mut self) {
        self.status = StatusCache {
            session_id: self.session_id,
            ..StatusCache::default()
        };
        let Some(session_id) = self.session_id else {
            return;
        };
        self.send_session_status();
        self.spawn(move |agent| async move {
            let permission_mode = agent
                .settings_view()
                .await
                .ok()
                .map(|view| view.settings.permission_mode);
            let goal = agent.goal(session_id).await.ok().flatten();
            Ok(Some(AgentReply::StatusLoaded {
                session_id,
                permission_mode,
                goal,
            }))
        });
    }

    /// What `load_session_status` read, kept when the session is still
    /// the one shown.
    pub(super) fn status_loaded(
        &mut self,
        session_id: AgentSessionId,
        permission_mode: Option<AgentPermissionMode>,
        goal: Option<AgentGoal>,
    ) -> Option<TurnUpdate> {
        if self.status.session_id != Some(session_id) {
            return None;
        }
        self.status.permission_mode = permission_mode;
        self.status.goal = goal;
        self.session_status()
    }

    /// The goal's event: set, changed or (`None`) cleared.
    pub(super) fn goal_updated(&mut self, goal: Option<AgentGoal>) -> Option<TurnUpdate> {
        self.status.goal = goal;
        self.session_status()
    }
}

/// The branch checked out in the Git repository `directory` is in (a short
/// commit when detached), read from its `HEAD` file: no process is started.
pub(crate) fn git_branch(directory: &Path) -> Option<String> {
    let git_dir = directory.ancestors().find_map(git_dir)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(reference) => Some(
            reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_string(),
        ),
        None => Some(head.chars().take(7).collect()).filter(|short: &String| !short.is_empty()),
    }
}

/// The Git directory of a repository rooted at `directory`: `.git`, or the
/// one a `.git` file names (a worktree's, a submodule's).
fn git_dir(directory: &Path) -> Option<PathBuf> {
    let dot_git = directory.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    Some(directory.join(target))
}

#[cfg(test)]
mod tests;
