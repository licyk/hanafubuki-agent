//! `/review` onto the facade's review calls: whether the session can be
//! reviewed and what it can review against, the review itself (it waits
//! for the whole review, so it runs as a task of the Agent's runtime), and
//! its cancellation. While a review runs no turn does: Esc cancels it, and
//! the chat holds a message sent meanwhile until it ends, so the next turn
//! reads what the review found.

use hanafubuki_agent::{
    AgentError, AgentReviewOutput, AgentReviewTarget, AgentReviewTargets, AgentSessionId,
    AgentSessionListRequest, AgentSessionOrigin, AgentSessionParentFilter, AgentTurnPhase,
};

use super::{AgentBridge, AgentReply, TurnUpdate};

/// Why a session cannot be reviewed yet: the review's findings are told to
/// the conversation, whose history starts with a turn.
const NO_TURN_YET: &str = "Send a message first; a review reports back into a conversation.";

/// What `/review` asks of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewCall {
    /// The picker: whether the session can be reviewed, and the branches
    /// and commits to choose from.
    Open,
    Start(AgentReviewTarget),
    Cancel,
}

/// What the Agent answered `/review`.
#[derive(Debug)]
pub(crate) enum ReviewReply {
    /// The session can be reviewed; the targets, or why they were not read
    /// (outside a repository, say).
    Ready(Result<AgentReviewTargets, String>),
    /// The review runs; what it looks at, in a few words.
    Started(String),
    Cancelling,
    Ended(ReviewEnd),
    /// A call failed; a review that runs goes on.
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum ReviewEnd {
    Finished(Box<AgentReviewOutput>),
    Cancelled,
    Failed(String),
}

/// The review that runs: the session it reports to.
pub(super) struct RunningReview {
    parent: AgentSessionId,
}

impl AgentBridge {
    pub(super) fn review_call(&mut self, call: ReviewCall) -> Option<TurnUpdate> {
        let reply = match call {
            ReviewCall::Open => match self.reviewable() {
                Ok(_) => {
                    self.spawn(|agent| async move {
                        let targets = match agent.review_targets().await {
                            Ok(targets) => Ok(targets),
                            Err(AgentError::FeatureUnavailable { reason, .. }) => {
                                return Ok(Some(reply(ReviewReply::Failed(unavailable(reason)))))
                            }
                            Err(error) => Err(error.to_string()),
                        };
                        Ok(Some(reply(ReviewReply::Ready(targets))))
                    });
                    return None;
                }
                Err(error) => ReviewReply::Failed(error),
            },
            ReviewCall::Start(target) => self
                .start_review(target)
                .unwrap_or_else(ReviewReply::Failed),
            ReviewCall::Cancel => self.cancel_review().unwrap_or_else(ReviewReply::Failed),
        };
        Some(TurnUpdate::Review(reply))
    }

    pub(super) fn review_ended(&mut self, end: ReviewEnd) -> Option<TurnUpdate> {
        self.review = None;
        self.retry_missed_wake();
        Some(TurnUpdate::Review(ReviewReply::Ended(end)))
    }

    pub(super) fn review_running(&self) -> bool {
        self.review.is_some()
    }

    /// The session, when a review of it can start: it exists, has had a
    /// turn, and neither a turn nor a review runs.
    fn reviewable(&self) -> Result<AgentSessionId, String> {
        if self.review.is_some() {
            return Err("A review is running; Esc or /review cancel stops it.".to_string());
        }
        if self.turn_running {
            return Err("a review does not start while a turn runs.".to_string());
        }
        let session_id = self.session_id.ok_or(NO_TURN_YET)?;
        let opened = self
            .agent
            .open_session(session_id, 1)
            .map_err(|error| format!("the session was not read: {error}"))?;
        if opened.transcript.items.is_empty() {
            return Err(NO_TURN_YET.to_string());
        }
        Ok(session_id)
    }

    fn start_review(&mut self, target: AgentReviewTarget) -> Result<ReviewReply, String> {
        let parent = self.reviewable()?;
        let hint = target_hint(&target);
        self.review = Some(RunningReview { parent });
        self.spawn(move |agent| async move {
            let end = match agent.start_review(parent, target).await {
                Ok(output) => ReviewEnd::Finished(Box::new(output)),
                Err(AgentError::Cancelled) => ReviewEnd::Cancelled,
                Err(AgentError::FeatureUnavailable { reason, .. }) => {
                    ReviewEnd::Failed(unavailable(reason))
                }
                Err(error) => ReviewEnd::Failed(format!("the review failed: {error}")),
            };
            Ok(Some(AgentReply::ReviewEnded(end)))
        });
        Ok(ReviewReply::Started(hint))
    }

    /// Interrupts the review session that runs: a child of the session of
    /// origin `Review` whose turn has not ended (the earlier reviews are
    /// children too).
    fn cancel_review(&mut self) -> Result<ReviewReply, String> {
        let parent = self
            .review
            .as_ref()
            .map(|review| review.parent)
            .ok_or("No review is running.")?;
        let children = self
            .agent
            .list_sessions_page(AgentSessionListRequest {
                parent: AgentSessionParentFilter::ChildrenOf(parent),
                ..AgentSessionListRequest::default()
            })
            .map_err(|error| format!("the review was not found: {error}"))?;
        let running: Vec<AgentSessionId> = children
            .sessions
            .into_iter()
            .filter(|child| {
                child.origin == Some(AgentSessionOrigin::Review)
                    && !matches!(
                        child.latest_turn_phase,
                        Some(
                            AgentTurnPhase::Completed
                                | AgentTurnPhase::Cancelled
                                | AgentTurnPhase::Failed
                                | AgentTurnPhase::Interrupted
                        )
                    )
            })
            .map(|child| child.session_id)
            .collect();
        if running.is_empty() {
            return Err("The review is still starting; cancel it again in a moment.".to_string());
        }
        self.spawn(move |agent| async move {
            for child in running {
                agent
                    .cancel_review(child)
                    .await
                    .map_err(|error| format!("the review was not cancelled: {error}"))?;
            }
            Ok(None)
        });
        Ok(ReviewReply::Cancelling)
    }
}

fn reply(reply: ReviewReply) -> AgentReply {
    AgentReply::Update(Box::new(TurnUpdate::Review(reply)))
}

fn unavailable(reason: String) -> String {
    format!("reviews are unavailable: {reason}")
}

/// What a review looks at, as the status line and the transcript say it
/// (the review plugin's words).
fn target_hint(target: &AgentReviewTarget) -> String {
    match target {
        AgentReviewTarget::UncommittedChanges => "current changes".to_string(),
        AgentReviewTarget::BaseBranch { branch } => format!("changes against '{branch}'"),
        AgentReviewTarget::Commit { sha, title } => {
            let short_sha: String = sha.chars().take(7).collect();
            match title {
                Some(title) => format!("commit {short_sha}: {title}"),
                None => format!("commit {short_sha}"),
            }
        }
        AgentReviewTarget::Custom { instructions } => instructions.trim().to_string(),
    }
}
