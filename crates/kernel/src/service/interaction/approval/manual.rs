//! `Manual`: the request is routed to the user, who answers through
//! `resolve_approval` before the timeout; cancelling the turn aborts it.

use super::*;

impl AgentService {
    pub(super) async fn authorize_by_user(
        &self,
        pending: PendingApproval,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<ToolApprovalDisposition, AgentError> {
        let identity = pending.identity;
        let admission_started = Instant::now();
        log::debug!(
            "agent_diag stage=approval.request.begin session_id={} turn_id={} generation={}",
            identity.session_id,
            identity.turn_id,
            identity.generation
        );
        let offers_allow_for_session = matches!(
            pending.flow,
            WaitingFlow::User {
                offers_allow_for_session: true
            }
        );
        let admission = self
            .inner
            .approvals
            .request_manual(
                pending.action.clone(),
                pending.context.clone(),
                pending.timeout,
                offers_allow_for_session,
            )
            .map_err(map_approval_error)?;
        log::debug!(
            "agent_diag stage=approval.request.end session_id={} turn_id={} generation={} elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            admission_started.elapsed().as_millis()
        );
        let request = admission.request;
        let request_id = request.request_id;
        self.inner.pending_approvals.lock().insert(
            request_id,
            PendingApprovalRoute {
                identity,
                context: pending.context.clone(),
            },
        );
        // Close the only cancellation race: cancel_turn can run after the
        // first check but before this request is routable. Once the route
        // exists, later cancellation resolves it through abort.
        if cancellation.is_cancelled() {
            self.withdraw_approval_request(request_id, &pending.context);
            return Err(AgentError::Cancelled);
        }
        if let Err(error) = self
            .emit(
                identity,
                AgentEventKind::ApprovalRequested { request },
                event_sink,
            )
            .await
        {
            self.withdraw_approval_request(request_id, &pending.context);
            return Err(error);
        }
        log::debug!(
            "agent_diag stage=approval.wait.begin session_id={} turn_id={} generation={} request_id={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            request_id
        );
        let mut waiter = Box::pin(admission.waiter);
        let outcome = match tokio::time::timeout(pending.timeout, waiter.as_mut()).await {
            Ok(outcome) => outcome,
            Err(_) => {
                // A user decision can win between the Tokio deadline and this
                // exact timeout commit. RequestNotFound means that another
                // resolver already completed the same waiter.
                match self.inner.approvals.timeout(request_id, &pending.context) {
                    Ok(()) | Err(ApprovalError::RequestNotFound) => {}
                    Err(error) => return Err(map_approval_error(error)),
                }
                waiter.await
            }
        };
        log::debug!(
            "agent_diag stage=approval.wait.end session_id={} turn_id={} generation={} request_id={} elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            request_id,
            pending.started.elapsed().as_millis()
        );
        self.inner.pending_approvals.lock().remove(&request_id);
        self.finish_user_approval(pending, request_id, outcome, event_sink)
            .await
    }

    /// Records the user's `AllowForSession`. The allowance lives in the
    /// session's runtime state only, so it ends with the Agent instance.
    async fn allow_tool_for_session(
        &self,
        session_id: AgentSessionId,
        tool: crate::AgentToolSelection,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let mut record = cell.lock_after_persistence().await?;
        record.runtime.allowed_tools.insert(tool);
        Ok(())
    }

    /// Aborts a request that never reached the user and drops its route.
    fn withdraw_approval_request(&self, request_id: ApprovalRequestId, context: &ApprovalContext) {
        let _ = self.inner.approvals.abort(request_id, context);
        self.inner.pending_approvals.lock().remove(&request_id);
    }

    /// Consumes a granted approval, then reports the resolution.
    async fn finish_user_approval(
        &self,
        pending: PendingApproval,
        request_id: ApprovalRequestId,
        outcome: ApprovalOutcome,
        event_sink: &dyn AgentEventSink,
    ) -> Result<ToolApprovalDisposition, AgentError> {
        let identity = pending.identity;
        let mut consume_started = None;
        let (resolution, disposition, result) = match outcome {
            ApprovalOutcome::Granted { grant, resolution } => {
                consume_started = Some(Instant::now());
                let approved = self
                    .inner
                    .approvals
                    .consume(grant, &pending.action, &pending.context)
                    .map_err(map_approval_error)?;
                if resolution.decision == ApprovalDecision::AllowForSession {
                    self.allow_tool_for_session(identity.session_id, pending.tool.clone())
                        .await?;
                }
                (
                    resolution,
                    "approved",
                    Ok(ToolApprovalDisposition::Approved(Box::new(approved))),
                )
            }
            ApprovalOutcome::Denied(resolution) => (
                resolution,
                "denied",
                Ok(ToolApprovalDisposition::Rejected(
                    "user denied the requested operation".to_string(),
                )),
            ),
            ApprovalOutcome::TimedOut(resolution) => (
                resolution,
                "timed_out",
                Ok(ToolApprovalDisposition::Rejected(
                    "approval timed out before the operation was dispatched".to_string(),
                )),
            ),
            ApprovalOutcome::Aborted(resolution) => {
                (resolution, "aborted", Err(AgentError::Cancelled))
            }
        };
        self.emit(
            identity,
            AgentEventKind::ApprovalResolved {
                request_id,
                resolution,
            },
            event_sink,
        )
        .await?;
        match consume_started {
            Some(consume_started) => log::debug!(
                "agent_diag stage=approval.core.end session_id={} turn_id={} generation={} request_id={} disposition={disposition} elapsed_ms={} consume_ms={}",
                identity.session_id,
                identity.turn_id,
                identity.generation,
                request_id,
                pending.started.elapsed().as_millis(),
                consume_started.elapsed().as_millis()
            ),
            None => log::debug!(
                "agent_diag stage=approval.core.end session_id={} turn_id={} generation={} request_id={} disposition={disposition} elapsed_ms={}",
                identity.session_id,
                identity.turn_id,
                identity.generation,
                request_id,
                pending.started.elapsed().as_millis()
            ),
        }
        result
    }
}
