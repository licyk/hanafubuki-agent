use super::*;
use crate::approved_action::approved_action_note;

/// How many of the session's latest denied reviews the user may approve.
const APPROVABLE_DENIED_REVIEWS: usize = 10;

impl AgentService {
    /// Commits the user's approval of the action that automatic permission
    /// review `review_id`, one of the session's ten latest denials, denied:
    /// a context note the next sampling step and the next review read. It
    /// starts no turn and authorizes nothing by itself, so a retry is still
    /// reviewed. Refused while a turn or a compaction runs. Off the async
    /// runtime only.
    pub fn approve_denied_action(
        &self,
        session_id: AgentSessionId,
        review_id: &str,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let (staged, event) = {
            let mut guard = cell.lock_after_persistence_blocking()?;
            let record = &mut *guard;
            ensure_no_staged_commit(record)?;
            if record
                .runtime
                .turn
                .as_ref()
                .is_some_and(|turn| !turn.phase.is_terminal())
            {
                return Err(AgentError::TurnAlreadyRunning);
            }
            if record
                .runtime
                .compaction
                .as_ref()
                .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
            {
                return Err(AgentError::CompactionAlreadyRunning);
            }
            let items = [approved_action_note(denied_action_summary(
                record, review_id,
            )?)];
            validate_history_append(record, &items, self.inner.limits)?;
            let previous = record.rollback_state();
            let start_index = record.conversation.history.len();
            append_history(record, &items);
            let event = record_session_event(
                record,
                AgentEventKind::HistoryCommitted {
                    start_index,
                    item_count: items.len(),
                    usage: None,
                },
                self.inner.limits,
            );
            (StagedPersistence::commit(&cell, record, previous), event)
        };
        self.persist_staged_blocking(staged)?;
        self.emit_session_event(session_id, event);
        Ok(())
    }

    pub(super) async fn record_auto_review_outcome(
        &self,
        identity: TurnIdentity,
        denied: bool,
    ) -> Result<bool, AgentError> {
        let cell = self.session(identity.session_id)?;
        let mut guard = cell.lock_after_persistence().await?;
        let record = &mut *guard;
        let turn = exact_turn_mut(record, identity)?;
        if denied {
            turn.consecutive_auto_review_denials =
                turn.consecutive_auto_review_denials.saturating_add(1);
        } else {
            turn.consecutive_auto_review_denials = 0;
        }
        Ok(turn.consecutive_auto_review_denials >= 3)
    }

    pub(in crate::service) fn abort_pending_approvals(&self, identity: TurnIdentity) {
        let routes = self
            .inner
            .pending_approvals
            .lock()
            .iter()
            .filter_map(|(id, route)| {
                (route.identity.session_id == identity.session_id
                    && route.identity.turn_id == identity.turn_id
                    && route.identity.generation == identity.generation)
                    .then_some((*id, route.context.clone()))
            })
            .collect::<Vec<_>>();
        for (request_id, context) in routes {
            let _ = self.inner.approvals.abort(request_id, &context);
            self.inner.pending_approvals.lock().remove(&request_id);
        }
    }

    pub(super) async fn complete_permission_review(
        &self,
        identity: TurnIdentity,
        review_id: &str,
        status: PermissionReviewStatus,
        assessment: Option<PermissionAssessment>,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let completed_at = now_millis_u64();
        validate_permission_review_state(status, assessment.as_ref(), Some(completed_at))?;
        let (staged, event) = {
            let cell = self.session(identity.session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            exact_turn_mut(record, identity)?;
            let item_index = record
                .conversation
                .history
                .iter()
                .rposition(|item| {
                    matches!(
                        &item.content,
                        ConversationItem::PermissionReview {
                            review_id: candidate,
                            status: PermissionReviewStatus::Reviewing,
                            ..
                        } if candidate == review_id
                    )
                })
                .ok_or_else(|| {
                    AgentError::Protocol(format!(
                        "active permission review {review_id} was not found"
                    ))
                })?;
            let (call_id, action_summary, started_at, previous_bytes, updated_bytes) = {
                let item = Arc::make_mut(&mut mutable_history(record)[item_index]);
                let previous_bytes = item.content.retained_bytes();
                let (call_id, action_summary, started_at) = match &mut item.content {
                    ConversationItem::PermissionReview {
                        call_id,
                        action_summary,
                        status: current_status,
                        assessment: current_assessment,
                        started_at,
                        completed_at: current_completed_at,
                        ..
                    } => {
                        *current_status = status;
                        *current_assessment = assessment.clone();
                        *current_completed_at = Some(completed_at);
                        (call_id.clone(), action_summary.clone(), *started_at)
                    }
                    // `item_index` was found matching a reviewing
                    // `PermissionReview` under the same record borrow.
                    _ => unreachable!("permission review was selected above"),
                };
                (
                    call_id,
                    action_summary,
                    started_at,
                    previous_bytes,
                    item.content.retained_bytes(),
                )
            };
            let next_history_bytes = record
                .conversation
                .history_bytes
                .saturating_sub(previous_bytes)
                .saturating_add(updated_bytes);
            if next_history_bytes > self.inner.limits.maximum_history_bytes {
                record.restore_rollback_state(previous);
                return Err(AgentError::LimitExceeded {
                    resource: "history bytes",
                    limit: self.inner.limits.maximum_history_bytes,
                });
            }
            record.conversation.history_bytes = next_history_bytes;
            persist_history_item_snapshot(record, item_index)?;
            let event = record_event(
                record,
                identity,
                AgentEventKind::PermissionReviewCompleted {
                    review_id: review_id.to_string(),
                    call_id,
                    action_summary,
                    status,
                    assessment,
                    started_at,
                    completed_at,
                },
                self.inner.limits,
            );
            (StagedPersistence::commit(&cell, record, previous), event)
        };
        self.persist_staged(staged).await?;
        event_sink.emit(event);
        Ok(())
    }
}

/// The summary of the action review `review_id` denied, if it is one of the
/// session's latest denials.
fn denied_action_summary<'a>(
    record: &'a SessionRecord,
    review_id: &str,
) -> Result<&'a str, AgentError> {
    record
        .conversation
        .history
        .iter()
        .rev()
        .filter_map(|item| match &item.content {
            ConversationItem::PermissionReview {
                review_id,
                action_summary,
                status: PermissionReviewStatus::Denied,
                ..
            } => Some((review_id, action_summary)),
            _ => None,
        })
        .take(APPROVABLE_DENIED_REVIEWS)
        .find(|(candidate, _)| *candidate == review_id)
        .map(|(_, action_summary)| action_summary.as_str())
        .ok_or(AgentError::ApprovalNotFound)
}
