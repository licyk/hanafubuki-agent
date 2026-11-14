//! The review calls: a review of the working directory's repository in a
//! child session, served by the `review` plugin row through the feature
//! helper.

use std::sync::Arc;

use hanafubuki_agent_kernel::{AgentError, AgentSessionId};

use super::AgentApplicationService;
use crate::plugins::{
    AgentReviewOutput, AgentReviewTarget, AgentReviewTargets, ReviewService, REVIEW_PLUGIN,
};

impl AgentApplicationService {
    /// Reviews `target` in the repository of the working directory (the
    /// first workspace root) for `parent`, and waits for the review: a
    /// review session, a durable child of `parent` of origin `Review`,
    /// runs one read-only turn whose final answer is read as the findings.
    /// `parent` is then told how the review ended, once it runs no turn.
    /// An interrupted ([`Self::cancel_review`]) or failed review is this
    /// call's error. `FeatureUnavailable` while the `review` plugin is not
    /// running.
    pub async fn start_review(
        &self,
        parent: AgentSessionId,
        target: AgentReviewTarget,
    ) -> Result<AgentReviewOutput, AgentError> {
        let cwd = self
            .driver
            .workspace_roots_async(parent)
            .await?
            .primary()
            .clone();
        self.feature(REVIEW_PLUGIN, |reviews: Arc<ReviewService>| async move {
            reviews.start(parent, target, cwd).await
        })
        .await
    }

    /// Interrupts the review session `child` (listed among its parent's
    /// children as soon as the review starts), and waits until the
    /// cancellation is confirmed. `FeatureUnavailable` while the `review`
    /// plugin is not running.
    pub async fn cancel_review(&self, child: AgentSessionId) -> Result<(), AgentError> {
        self.feature(REVIEW_PLUGIN, |reviews: Arc<ReviewService>| async move {
            reviews.cancel(child).await
        })
        .await
    }

    /// What a review can target in the working directory's repository: its
    /// branches, the current branch and its latest commits.
    /// `FeatureUnavailable` while the `review` plugin is not running.
    pub async fn review_targets(&self) -> Result<AgentReviewTargets, AgentError> {
        self.review_targets_in(self.working_directory().await?)
            .await
    }

    pub async fn review_targets_for_session(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentReviewTargets, AgentError> {
        self.review_targets_in(
            self.driver
                .workspace_roots_async(session_id)
                .await?
                .primary()
                .clone(),
        )
        .await
    }

    async fn review_targets_in(
        &self,
        cwd: std::path::PathBuf,
    ) -> Result<AgentReviewTargets, AgentError> {
        self.feature(REVIEW_PLUGIN, |reviews: Arc<ReviewService>| async move {
            reviews.targets(cwd).await
        })
        .await
    }
}
