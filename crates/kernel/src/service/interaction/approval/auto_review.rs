//! `AutoReview`: the reviewer model decides, the review is recorded in
//! history, and three consecutive denials stop the turn.

use super::*;

impl AgentService {
    pub(super) async fn authorize_by_review(
        &self,
        pending: PendingApproval,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<ToolApprovalDisposition, AgentError> {
        let identity = pending.identity;
        let (review_session, review_model, review_transcript) = {
            let cell = self.session(identity.session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            exact_turn(record, identity.turn_id, identity.generation)?;
            (
                record.settings.model.create_model_session(),
                record.settings.config.model.clone(),
                history_contents(&record.conversation.history),
            )
        };
        let review_started_at = now_millis_u64();
        let review_id = format!(
            "permission-review-{}-{}-{}-{}",
            identity.session_id, identity.turn_id, identity.generation, pending.call_id
        );
        self.commit_history(
            identity,
            &[ConversationItem::PermissionReview {
                review_id: review_id.clone(),
                call_id: pending.call_id.clone(),
                action_summary: pending.action.summary.clone(),
                status: PermissionReviewStatus::Reviewing,
                assessment: None,
                started_at: review_started_at,
                completed_at: None,
            }],
            event_sink,
        )
        .await?;
        self.emit(
            identity,
            AgentEventKind::PermissionReviewStarted {
                review_id: review_id.clone(),
                call_id: pending.call_id.clone(),
                action: pending.action.clone(),
                started_at: review_started_at,
            },
            event_sink,
        )
        .await?;
        let reviewed = review_permission(
            review_session,
            review_model,
            review_transcript,
            &pending.action,
            cancellation,
        )
        .await;
        let (status, assessment) = review_status(&reviewed);
        self.complete_permission_review(identity, &review_id, status, assessment, event_sink)
            .await?;
        let denial_limit_reached = self
            .record_auto_review_outcome(
                identity,
                matches!(
                    &reviewed,
                    Ok(assessment) if assessment.outcome == PermissionAssessmentOutcome::Deny
                ),
            )
            .await?;
        match reviewed {
            Ok(assessment) if assessment.outcome == PermissionAssessmentOutcome::Allow => {
                self.authorize_directly(pending, ApprovalDecisionSource::AutoReviewer)
            }
            Ok(assessment) if denial_limit_reached => Err(AgentError::Approval(
                bounded_diagnostic(format!(
                    "automatic permission review denied three consecutive actions; the turn was stopped to prevent repeated attempts. Last rationale: {}",
                    assessment.rationale
                )),
            )),
            Ok(assessment) => Ok(ToolApprovalDisposition::Rejected(bounded_diagnostic(
                format!(
                    "automatic permission review denied the action: {}. Do not retry the same outcome through an indirect or workaround action.",
                    assessment.rationale
                ),
            ))),
            Err(PermissionReviewError::Cancelled) => Err(AgentError::Cancelled),
            Err(PermissionReviewError::TimedOut) => Ok(ToolApprovalDisposition::Rejected(
                "automatic permission review timed out; the action was not executed".to_string(),
            )),
            Err(PermissionReviewError::Failed(_)) => Ok(ToolApprovalDisposition::Rejected(
                "automatic permission review failed closed; the action was not executed"
                    .to_string(),
            )),
        }
    }
}

/// The recorded status of a finished review and the assessment it keeps.
fn review_status(
    reviewed: &Result<PermissionAssessment, PermissionReviewError>,
) -> (PermissionReviewStatus, Option<PermissionAssessment>) {
    match reviewed {
        Ok(assessment) if assessment.outcome == PermissionAssessmentOutcome::Allow => {
            (PermissionReviewStatus::Approved, Some(assessment.clone()))
        }
        Ok(assessment) => (PermissionReviewStatus::Denied, Some(assessment.clone())),
        Err(PermissionReviewError::TimedOut) => (PermissionReviewStatus::TimedOut, None),
        Err(PermissionReviewError::Cancelled) => (PermissionReviewStatus::Aborted, None),
        Err(PermissionReviewError::Failed(_)) => (PermissionReviewStatus::Failed, None),
    }
}
