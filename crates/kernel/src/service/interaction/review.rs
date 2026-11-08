use super::*;

impl AgentService {
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
