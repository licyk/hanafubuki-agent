//! Approval of one action a tool call needs: the plugins' approval rules
//! first, then the flow of the effective permission mode (`FullAccess`
//! authorizes, `AutoReview` asks the reviewer model, `Manual` asks a person).

use super::*;

use hanafubuki_agent_approval::AgentPermissionMode;
use hanafubuki_agent_contracts::ApprovalRuleVerdict;

mod auto_review;
mod manual;

/// One action on its way to a decision, in the turn that asked for it.
pub(in crate::service) struct PendingApproval {
    identity: TurnIdentity,
    call_id: String,
    /// The tool an `AllowForSession` answer allows.
    tool: crate::AgentToolSelection,
    action: ApprovalAction,
    context: ApprovalContext,
    timeout: Duration,
    started: Instant,
    flow: WaitingFlow,
}

/// Who decides a pending approval.
enum WaitingFlow {
    Reviewer,
    User { offers_allow_for_session: bool },
}

/// How an action that needs review is decided.
pub(in crate::service) enum ApprovalRoute {
    /// Decided at once: a rule's denial, `FullAccess`, or the session's
    /// allowance of the tool.
    Decided(ToolApprovalDisposition),
    /// The reviewer model or the user decides. The caller waits for it with
    /// [`AgentService::await_approval`], holding none of the step's gate.
    Waiting(Box<PendingApproval>),
}

impl AgentService {
    pub fn resolve_approval(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
        request_id: ApprovalRequestId,
        decision: ApprovalUserDecision,
    ) -> Result<(), AgentError> {
        {
            let cell = self.session(session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            let turn = exact_turn(record, turn_id, generation)?;
            if turn.phase.is_terminal() {
                return Err(AgentError::TurnNotRunning);
            }
        }
        let context = {
            let pending = self.inner.pending_approvals.lock();
            let route = pending
                .get(&request_id)
                .ok_or(AgentError::ApprovalNotFound)?;
            if route.identity.session_id != session_id
                || route.identity.turn_id != turn_id
                || route.identity.generation != generation
            {
                return Err(AgentError::StaleApproval);
            }
            route.context.clone()
        };
        self.inner
            .approvals
            .resolve_user(request_id, &context, decision)
            .map_err(map_approval_error)
    }

    /// Decides how `action` is approved: the rules of the plugins that own
    /// it first, then the session's allowance of the tool, then the turn's
    /// permission mode. Nothing here waits.
    pub(in crate::service) fn route_approval(
        &self,
        tool_context: &AgentToolInvocationContext,
        action: ApprovalAction,
        cancellation: &CancellationToken,
    ) -> Result<ApprovalRoute, AgentError> {
        let identity = TurnIdentity {
            session_id: tool_context.session_id,
            turn_id: tool_context.turn_id,
            generation: tool_context.generation,
        };
        let timeout = self
            .turn_budget_policy(identity)?
            .approval_timeout()
            .min(self.inner.limits.approval_timeout);
        log::debug!(
            "agent_diag stage=approval.core.begin session_id={} turn_id={} generation={} action_kind={:?} risk={:?}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            action.kind,
            action.risk
        );
        if cancellation.is_cancelled() {
            return Err(AgentError::Cancelled);
        }
        let context = approval_context(tool_context)?;
        let tool = crate::AgentToolSelection {
            provider_id: tool_context.provider_id.clone(),
            tool_id: tool_context.tool_id.clone(),
            generation: tool_context.tool_generation,
        };
        let (turn_mode, allowed_for_session, origin) = {
            let cell = self.session(identity.session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            (
                exact_turn(record, identity.turn_id, identity.generation)?.permission_mode,
                record.runtime.allowed_tools.contains(&tool),
                record.lineage.origin,
            )
        };
        // The rules of the plugins that own the action can only tighten the
        // turn's mode: a denial ends here, and a required human decision is
        // asked every time, so neither FullAccess, the automatic reviewer
        // nor an earlier "allow for this session" answers it.
        let verdict = tool_context
            .approval_rules
            .as_ref()
            .and_then(|rules| rules.verdict(origin, &action));
        let mut pending = PendingApproval {
            identity,
            call_id: tool_context.call_id.clone(),
            tool,
            action,
            context,
            timeout,
            started: Instant::now(),
            flow: WaitingFlow::Reviewer,
        };
        let source = match verdict {
            Some(ApprovalRuleVerdict::Deny { reason }) => {
                return Ok(ApprovalRoute::Decided(ToolApprovalDisposition::Rejected(
                    bounded_diagnostic(reason),
                )));
            }
            Some(ApprovalRuleVerdict::RequireUser) => {
                pending.flow = WaitingFlow::User {
                    offers_allow_for_session: false,
                };
                return Ok(ApprovalRoute::Waiting(Box::new(pending)));
            }
            None if allowed_for_session => ApprovalDecisionSource::SessionAllowance,
            None => match turn_mode {
                AgentPermissionMode::FullAccess => ApprovalDecisionSource::FullAccess,
                AgentPermissionMode::AutoReview => {
                    return Ok(ApprovalRoute::Waiting(Box::new(pending)))
                }
                AgentPermissionMode::Manual => {
                    pending.flow = WaitingFlow::User {
                        offers_allow_for_session: true,
                    };
                    return Ok(ApprovalRoute::Waiting(Box::new(pending)));
                }
            },
        };
        Ok(ApprovalRoute::Decided(
            self.authorize_directly(pending, source)?,
        ))
    }

    /// Waits for the decision on a pending approval: the reviewer model's, or
    /// the user's, asked one prompt at a time per step (`prompts`).
    pub(in crate::service) async fn await_approval(
        &self,
        pending: PendingApproval,
        prompts: &tokio::sync::Mutex<()>,
        cancellation: CancellationToken,
        event_sink: &dyn AgentEventSink,
    ) -> Result<ToolApprovalDisposition, AgentError> {
        match pending.flow {
            WaitingFlow::Reviewer => {
                self.authorize_by_review(pending, cancellation, event_sink)
                    .await
            }
            WaitingFlow::User { .. } => {
                let _prompt = prompts.lock().await;
                self.authorize_by_user(pending, cancellation, event_sink)
                    .await
            }
        }
    }

    /// Issues the one-shot authorization without asking anyone further.
    fn authorize_directly(
        &self,
        pending: PendingApproval,
        source: ApprovalDecisionSource,
    ) -> Result<ToolApprovalDisposition, AgentError> {
        let authorized = self
            .inner
            .approvals
            .authorize(pending.action, &pending.context, pending.timeout, source)
            .map_err(map_approval_error)?;
        Ok(ToolApprovalDisposition::Approved(Box::new(authorized)))
    }
}
