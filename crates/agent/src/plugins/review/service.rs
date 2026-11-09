// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/src/tasks/review.rs` and `codex-rs/core/src/session/review.rs`
// (a review as a one-shot sub-agent with the rubric, which never asks for
// approval, whose final answer is read as the review and whose end is
// recorded in the reviewed conversation) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the review is a durable child session of
// origin `Review` driven through the session driver, read-only by an
// approval rule rather than a sandbox; it keeps the Agent's tools (web
// search and image viewing included; a child spawns no sub-agents); the
// review uses the reviewed conversation's model; a failed review is
// reported as interrupted; the exit note waits for the reviewed
// conversation's running turn to end.

//! What the `review` row's service does: start a review of a session's
//! work in a child session, read its answer, and tell the reviewed
//! conversation; interrupt one; list what can be reviewed.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_kernel::{
    AgentChildSessionBase, AgentChildSessionRequest, AgentError, AgentPermissionMode,
    AgentSessionId, AgentSessionMode, AgentSessionOrigin, AgentSessionPersistence,
    AgentSessionRuntimeSnapshot, ConversationItem, AGENT_SESSION_TITLE_MAX_CHARS,
};
use hanafubuki_agent_worktree::{GitQueries, WorktreeError};

use super::output::{parse_review_answer, review_exit_note};
use super::request::{review_prompt, user_facing_hint};
use super::{AgentReviewCommit, AgentReviewOutput, AgentReviewTarget, AgentReviewTargets};
use crate::session_driver::AgentSessionDriver;

/// How many commits a review of a commit is chosen from.
const RECENT_COMMITS: usize = 100;
/// How often the exit note retries while the reviewed conversation runs a
/// turn or a compaction.
const PARENT_BUSY_POLL: Duration = Duration::from_millis(100);

/// What the `review` row provides under its name.
pub(crate) struct ReviewService {
    pub(super) driver: AgentSessionDriver,
    pub(super) git: GitQueries,
    pub(super) blocking: Arc<BlockingService>,
}

impl ReviewService {
    /// Reviews `target` of the repository `cwd` is in, for `parent`: a new
    /// review session runs one turn, its answer is read as the review, and
    /// `parent` is told how it ended (also when it was interrupted or
    /// failed, which is then this call's error).
    pub(crate) async fn start(
        &self,
        parent: AgentSessionId,
        target: AgentReviewTarget,
        cwd: PathBuf,
    ) -> Result<AgentReviewOutput, AgentError> {
        let prompt = self.prompt(&target, cwd).await?;
        let title = review_title(&user_facing_hint(&target));
        let child = self
            .blocking(move |driver| {
                // A history starts with a turn, so the review's note could
                // not follow one that has none.
                if !driver.has_history(parent)? {
                    return Err(AgentError::InvalidInput(
                        "a review reports to its conversation, and this one has had no turn yet; send a message first"
                            .to_string(),
                    ));
                }
                let child = driver.create_child(AgentChildSessionRequest {
                    parent_session_id: parent,
                    origin: AgentSessionOrigin::Review,
                    persistence: AgentSessionPersistence::Durable,
                    base: AgentChildSessionBase::Empty,
                    context: Vec::new(),
                })?;
                driver.rename(child.id, title)?;
                driver.set_mode(child.id, AgentSessionMode::Default)?;
                Ok(child.id)
            })
            .await?;
        // For this turn only: a later turn of the session, which a person
        // may start, runs in the session's own mode.
        // Boxed: a turn's future is too large for a caller's stack.
        let outcome = Box::pin(self.driver.run_turn(
            child,
            prompt,
            Some(AgentPermissionMode::FullAccess),
        ))
        .await;
        let output = outcome.map(|outcome| parse_review_answer(&outcome.final_answer, child));
        let note = review_exit_note(output.as_ref().ok()).map_err(AgentError::Internal)?;
        self.tell_parent(parent, note).await?;
        output
    }

    /// Interrupts the review `child` runs, if any, and waits until the
    /// cancellation is confirmed; the review then ends as interrupted.
    pub(crate) async fn cancel(&self, child: AgentSessionId) -> Result<(), AgentError> {
        if self.session(child).await?.origin != Some(AgentSessionOrigin::Review) {
            return Err(AgentError::InvalidInput(format!(
                "Agent session {child} is not a review"
            )));
        }
        match self.driver.interrupt(child).await {
            Err(AgentError::TurnNotRunning) => Ok(()),
            result => result,
        }
    }

    /// The branches, the current branch and the latest commits of the
    /// repository `cwd` is in.
    pub(crate) async fn targets(&self, cwd: PathBuf) -> Result<AgentReviewTargets, AgentError> {
        let git = self.git.clone();
        self.blocking
            .run("agent review targets", move || {
                Ok(AgentReviewTargets {
                    branches: git.local_branches(&cwd)?,
                    current_branch: git.current_branch(&cwd)?,
                    commits: git
                        .recent_commits(&cwd, RECENT_COMMITS)?
                        .into_iter()
                        .map(|commit| AgentReviewCommit {
                            sha: commit.sha,
                            subject: commit.subject,
                        })
                        .collect(),
                })
            })
            .await
            .map_err(|error| AgentError::Internal(error.to_string()))?
            .map_err(|error: WorktreeError| AgentError::InvalidInput(error.to_string()))
    }

    /// The session's current state.
    pub(super) async fn session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        self.blocking(move |driver| driver.snapshot(session_id))
            .await
    }

    /// The review's first message; a base branch's merge base is looked up
    /// in `cwd`.
    async fn prompt(&self, target: &AgentReviewTarget, cwd: PathBuf) -> Result<String, AgentError> {
        let merge_base = match target {
            AgentReviewTarget::BaseBranch { branch } => {
                let git = self.git.clone();
                let branch = branch.clone();
                self.blocking
                    .run("agent review merge base", move || {
                        git.merge_base_with_head(&cwd, &branch)
                    })
                    .await
                    .map_err(|error| AgentError::Internal(error.to_string()))?
                    .map_err(|error| AgentError::InvalidInput(error.to_string()))?
            }
            AgentReviewTarget::Custom { instructions } if instructions.trim().is_empty() => {
                return Err(AgentError::InvalidInput(
                    "Review prompt cannot be empty".to_string(),
                ));
            }
            _ => None,
        };
        Ok(review_prompt(target, merge_base.as_deref()))
    }

    /// Commits `note` to `parent` once it runs no turn and no compaction.
    async fn tell_parent(
        &self,
        parent: AgentSessionId,
        note: ConversationItem,
    ) -> Result<(), AgentError> {
        loop {
            let item = note.clone();
            match self
                .blocking(move |driver| driver.add_context(parent, vec![item]))
                .await
            {
                Err(AgentError::TurnAlreadyRunning | AgentError::CompactionAlreadyRunning) => {
                    tokio::time::sleep(PARENT_BUSY_POLL).await;
                }
                result => return result,
            }
        }
    }

    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&AgentSessionDriver) -> Result<T, AgentError> + Send + 'static,
    ) -> Result<T, AgentError> {
        let driver = self.driver.clone();
        self.blocking
            .run("agent review operation", move || work(&driver))
            .await
            .map_err(|error| AgentError::Internal(error.to_string()))?
    }
}

/// `Review: <hint>` on one line, cut to a session title's length.
fn review_title(hint: &str) -> String {
    let title = format!(
        "Review: {}",
        hint.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    if title.chars().count() <= AGENT_SESSION_TITLE_MAX_CHARS {
        return title;
    }
    let mut cut = title
        .chars()
        .take(AGENT_SESSION_TITLE_MAX_CHARS - 1)
        .collect::<String>();
    cut.push('…');
    cut
}
