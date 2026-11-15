//! Presentation of authoritative compaction and permission-review events.

use hanafubuki_agent::{
    AgentCompactionStatus, AgentCompactionTrigger, AgentEventKind, PermissionReviewStatus,
};

#[derive(Debug, Clone)]
pub(crate) struct WorkSnapshot {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) text: Option<String>,
    pub(crate) started_at: u64,
    pub(crate) completed_at: Option<u64>,
    pub(crate) warning: Option<String>,
    pub(crate) committed: bool,
}

impl WorkSnapshot {
    pub(crate) fn from_event(event: &AgentEventKind) -> Option<Self> {
        let snapshot = match event {
            AgentEventKind::CompactionUpdated { compaction } => compaction,
            AgentEventKind::PermissionReviewStarted {
                review_id,
                action,
                started_at,
                ..
            } => {
                return Some(Self::review(
                    review_id.clone(),
                    action.summary.clone(),
                    PermissionReviewStatus::Reviewing,
                    None,
                    *started_at,
                    None,
                ))
            }
            AgentEventKind::PermissionReviewCompleted {
                review_id,
                action_summary,
                status,
                assessment,
                started_at,
                completed_at,
                ..
            } => {
                return Some(Self::review(
                    review_id.clone(),
                    action_summary.clone(),
                    *status,
                    assessment.as_ref().map(|value| value.rationale.clone()),
                    *started_at,
                    Some(*completed_at),
                ))
            }
            _ => return None,
        };
        let title = match snapshot.status {
            AgentCompactionStatus::Reserved => "Context compaction queued",
            AgentCompactionStatus::Running => "Compacting context",
            AgentCompactionStatus::ReadyToInstall => "Context compaction ready to install",
            AgentCompactionStatus::Completed => "Context compacted",
            AgentCompactionStatus::Failed => "Context compaction failed",
            AgentCompactionStatus::Cancelled => "Context compaction cancelled",
        };
        let warning = (snapshot.status == AgentCompactionStatus::Failed
            && snapshot.trigger == AgentCompactionTrigger::Automatic)
            .then(|| {
                format!(
                    "Context compaction failed: {}",
                    snapshot.last_error.as_deref().unwrap_or_default()
                )
            });
        let title = match &snapshot.last_error {
            Some(error) => format!("{title}: {error}"),
            None => title.to_string(),
        };
        Some(Self {
            id: format!("compaction:{}", snapshot.id),
            title,
            text: snapshot.summary.clone(),
            started_at: snapshot.created_at,
            completed_at: snapshot.status.is_terminal().then_some(snapshot.updated_at),
            warning,
            committed: snapshot.status == AgentCompactionStatus::Completed,
        })
    }

    pub(crate) fn review(
        id: String,
        action: String,
        status: PermissionReviewStatus,
        rationale: Option<String>,
        started_at: u64,
        completed_at: Option<u64>,
    ) -> Self {
        let label = match status {
            PermissionReviewStatus::Reviewing => "Automatic approval review",
            PermissionReviewStatus::Approved => "Automatic approval review approved",
            PermissionReviewStatus::Denied => "Automatic approval review denied",
            PermissionReviewStatus::TimedOut => "Automatic approval review timed out",
            PermissionReviewStatus::Failed => "Automatic approval review failed",
            PermissionReviewStatus::Aborted => "Automatic approval review aborted",
        };
        Self {
            id: format!("permission-review:{id}"),
            title: format!("{label}: {action}"),
            text: rationale,
            started_at,
            completed_at,
            warning: None,
            committed: false,
        }
    }
}
