use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::{
    ApprovalAction, ApprovalActionDigest, ApprovalContext, ApprovalContextDigest,
    ApprovalContextScope, ApprovalDecision, ApprovalDecisionSource, ApprovalError, ApprovalGrantId,
    ApprovalRequest, ApprovalRequestId, ApprovalResolution, ApprovalUserDecision,
};

pub trait ApprovalClock: Send + Sync {
    fn now_millis(&self) -> u128;
}

#[derive(Default)]
pub struct SystemApprovalClock;

impl ApprovalClock for SystemApprovalClock {
    fn now_millis(&self) -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    }
}

#[derive(Clone)]
pub struct ApprovalService {
    inner: Arc<ApprovalServiceInner>,
}

struct ApprovalServiceInner {
    clock: Arc<dyn ApprovalClock>,
    state: Mutex<ApprovalState>,
}

struct ApprovalState {
    next_request_id: u64,
    next_grant_id: u64,
    pending: HashMap<ApprovalRequestId, PendingRequest>,
    issued: HashMap<ApprovalGrantId, IssuedGrant>,
}

struct PendingRequest {
    request: ApprovalRequest,
    context_digest: ApprovalContextDigest,
    completion: Arc<ApprovalCompletion>,
}

struct IssuedGrant {
    request_id: ApprovalRequestId,
    action: ApprovalAction,
    context_digest: ApprovalContextDigest,
    expires_at_ms: u128,
    source: ApprovalDecisionSource,
}

#[derive(Default)]
struct ApprovalCompletion {
    outcome: Mutex<Option<ApprovalOutcome>>,
    waker: Mutex<Option<Waker>>,
}

impl ApprovalCompletion {
    fn finish(&self, outcome: ApprovalOutcome) {
        *self.outcome.lock() = Some(outcome);
        if let Some(waker) = self.waker.lock().take() {
            waker.wake();
        }
    }
}

pub struct ApprovalWaiter {
    completion: Arc<ApprovalCompletion>,
}

impl Future for ApprovalWaiter {
    type Output = ApprovalOutcome;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(outcome) = self.completion.outcome.lock().take() {
            return Poll::Ready(outcome);
        }
        *self.completion.waker.lock() = Some(context.waker().clone());
        if let Some(outcome) = self.completion.outcome.lock().take() {
            Poll::Ready(outcome)
        } else {
            Poll::Pending
        }
    }
}

pub struct ManualApprovalAdmission {
    pub request: ApprovalRequest,
    pub waiter: ApprovalWaiter,
}

pub enum ApprovalOutcome {
    Granted {
        grant: ApprovalGrant,
        resolution: ApprovalResolution,
    },
    Denied(ApprovalResolution),
    Aborted(ApprovalResolution),
    TimedOut(ApprovalResolution),
}

/// Opaque, non-serializable, one-shot capability issued for one exact action.
pub struct ApprovalGrant {
    id: ApprovalGrantId,
    request_id: ApprovalRequestId,
    action_digest: ApprovalActionDigest,
    context_digest: ApprovalContextDigest,
    expires_at_ms: u128,
    source: ApprovalDecisionSource,
}

impl ApprovalGrant {
    pub fn id(&self) -> ApprovalGrantId {
        self.id
    }

    pub fn request_id(&self) -> ApprovalRequestId {
        self.request_id
    }
}

/// A grant consumed by the approval authority immediately before dispatch.
///
/// This capability is intentionally neither cloneable nor serializable.
pub struct AuthorizedAction {
    grant_id: ApprovalGrantId,
    request_id: ApprovalRequestId,
    action: ApprovalAction,
    context_digest: ApprovalContextDigest,
    context_scope: Option<ApprovalContextScope>,
    source: ApprovalDecisionSource,
}

impl AuthorizedAction {
    pub fn action(&self) -> &ApprovalAction {
        &self.action
    }

    pub fn matches(&self, action: &ApprovalAction, context: &ApprovalContext) -> bool {
        self.action == *action && self.context_digest == *context.digest()
    }

    pub fn grant_id(&self) -> ApprovalGrantId {
        self.grant_id
    }

    pub fn request_id(&self) -> ApprovalRequestId {
        self.request_id
    }

    pub fn source(&self) -> ApprovalDecisionSource {
        self.source
    }

    pub fn context_scope(&self) -> Option<&ApprovalContextScope> {
        self.context_scope.as_ref()
    }
}

impl ApprovalService {
    pub fn manual() -> Self {
        Self::new(Arc::new(SystemApprovalClock))
    }

    pub fn new(clock: Arc<dyn ApprovalClock>) -> Self {
        Self {
            inner: Arc::new(ApprovalServiceInner {
                clock,
                state: Mutex::new(ApprovalState {
                    next_request_id: 1,
                    next_grant_id: 1,
                    pending: HashMap::new(),
                    issued: HashMap::new(),
                }),
            }),
        }
    }

    pub fn request_manual(
        &self,
        action: ApprovalAction,
        context: ApprovalContext,
        timeout: Duration,
        offers_allow_for_session: bool,
    ) -> Result<ManualApprovalAdmission, ApprovalError> {
        if timeout.is_zero() {
            return Err(ApprovalError::InvalidTimeout);
        }
        let now = self.inner.clock.now_millis();
        let expires_at_ms = now.saturating_add(timeout.as_millis());
        let mut state = self.inner.state.lock();
        let request_id = ApprovalRequestId::new(state.next_request_id);
        state.next_request_id = state
            .next_request_id
            .checked_add(1)
            .expect("approval request id exhausted");
        let request = ApprovalRequest {
            request_id,
            operation_id: context.operation_id().to_string(),
            action,
            requested_at_ms: now,
            expires_at_ms,
            offers_allow_for_session,
        };
        let completion = Arc::new(ApprovalCompletion::default());
        state.pending.insert(
            request_id,
            PendingRequest {
                request: request.clone(),
                context_digest: context.digest().clone(),
                completion: completion.clone(),
            },
        );
        log::debug!(
            "approval request {request_id:?} waiting operation={}",
            context.operation_id()
        );
        Ok(ManualApprovalAdmission {
            request,
            waiter: ApprovalWaiter { completion },
        })
    }

    pub fn authorize(
        &self,
        action: ApprovalAction,
        context: &ApprovalContext,
        timeout: Duration,
        source: ApprovalDecisionSource,
    ) -> Result<AuthorizedAction, ApprovalError> {
        if timeout.is_zero()
            || !matches!(
                source,
                ApprovalDecisionSource::AutoReviewer
                    | ApprovalDecisionSource::FullAccess
                    | ApprovalDecisionSource::SessionAllowance
            )
        {
            return Err(ApprovalError::InvalidDecisionSource);
        }
        let now = self.inner.clock.now_millis();
        let expires_at_ms = now.saturating_add(timeout.as_millis());
        let mut state = self.inner.state.lock();
        let request_id = next_request_id(&mut state);
        let grant = issue_grant(
            &mut state,
            request_id,
            action.clone(),
            context.digest().clone(),
            expires_at_ms,
            source,
        );
        drop(state);
        self.consume(grant, &action, context)
    }

    pub fn resolve_user(
        &self,
        request_id: ApprovalRequestId,
        context: &ApprovalContext,
        decision: ApprovalUserDecision,
    ) -> Result<(), ApprovalError> {
        self.resolve(
            request_id,
            context,
            decision.into(),
            ApprovalDecisionSource::User,
        )
    }

    pub fn timeout(
        &self,
        request_id: ApprovalRequestId,
        context: &ApprovalContext,
    ) -> Result<(), ApprovalError> {
        self.resolve(
            request_id,
            context,
            ApprovalDecision::TimedOut,
            ApprovalDecisionSource::System,
        )
    }

    pub fn abort(
        &self,
        request_id: ApprovalRequestId,
        context: &ApprovalContext,
    ) -> Result<(), ApprovalError> {
        self.resolve(
            request_id,
            context,
            ApprovalDecision::Aborted,
            ApprovalDecisionSource::System,
        )
    }

    pub fn consume(
        &self,
        grant: ApprovalGrant,
        expected_action: &ApprovalAction,
        expected_context: &ApprovalContext,
    ) -> Result<AuthorizedAction, ApprovalError> {
        let now = self.inner.clock.now_millis();
        let mut state = self.inner.state.lock();
        let issued = state
            .issued
            .get(&grant.id)
            .ok_or(ApprovalError::GrantNotFound)?;
        if issued.request_id != grant.request_id
            || issued.action.digest != grant.action_digest
            || issued.context_digest != grant.context_digest
            || issued.action != *expected_action
            || issued.context_digest != *expected_context.digest()
            || grant.expires_at_ms != issued.expires_at_ms
            || grant.source != issued.source
        {
            return Err(ApprovalError::GrantMismatch);
        }
        if now >= issued.expires_at_ms {
            state.issued.remove(&grant.id);
            log::warn!(
                "approval grant {:?} for request {:?} expired before dispatch",
                grant.id,
                grant.request_id
            );
            return Err(ApprovalError::GrantExpired);
        }
        let issued = state
            .issued
            .remove(&grant.id)
            .expect("issued approval grant was checked under the registry lock");
        Ok(AuthorizedAction {
            grant_id: grant.id,
            request_id: issued.request_id,
            action: issued.action,
            context_digest: issued.context_digest,
            context_scope: expected_context.scope().cloned(),
            source: issued.source,
        })
    }

    fn resolve(
        &self,
        request_id: ApprovalRequestId,
        context: &ApprovalContext,
        decision: ApprovalDecision,
        source: ApprovalDecisionSource,
    ) -> Result<(), ApprovalError> {
        let now = self.inner.clock.now_millis();
        let mut state = self.inner.state.lock();
        let pending = state
            .pending
            .get(&request_id)
            .ok_or(ApprovalError::RequestNotFound)?;
        if pending.context_digest != *context.digest() {
            return Err(ApprovalError::StaleContext);
        }
        if decision == ApprovalDecision::AllowForSession
            && !pending.request.offers_allow_for_session
        {
            return Err(ApprovalError::DecisionNotOffered);
        }
        let decision = if now >= pending.request.expires_at_ms {
            ApprovalDecision::TimedOut
        } else {
            decision
        };
        let source = if decision == ApprovalDecision::TimedOut {
            ApprovalDecisionSource::System
        } else {
            source
        };
        let pending = state
            .pending
            .remove(&request_id)
            .expect("pending approval was checked under the registry lock");
        let resolution = ApprovalResolution {
            request_id,
            decision,
            source,
            resolved_at_ms: now,
        };
        let outcome = match decision {
            ApprovalDecision::AllowOnce | ApprovalDecision::AllowForSession => {
                ApprovalOutcome::Granted {
                    grant: issue_grant(
                        &mut state,
                        request_id,
                        pending.request.action,
                        pending.context_digest,
                        pending.request.expires_at_ms,
                        source,
                    ),
                    resolution,
                }
            }
            ApprovalDecision::Deny => ApprovalOutcome::Denied(resolution),
            ApprovalDecision::Aborted => ApprovalOutcome::Aborted(resolution),
            ApprovalDecision::TimedOut => ApprovalOutcome::TimedOut(resolution),
        };
        log::debug!(
            "approval request {request_id:?} resolved decision={decision:?} source={source:?}"
        );
        pending.completion.finish(outcome);
        Ok(())
    }
}

fn next_request_id(state: &mut ApprovalState) -> ApprovalRequestId {
    let request_id = ApprovalRequestId::new(state.next_request_id);
    state.next_request_id = state
        .next_request_id
        .checked_add(1)
        .expect("approval request id exhausted");
    request_id
}

fn issue_grant(
    state: &mut ApprovalState,
    request_id: ApprovalRequestId,
    action: ApprovalAction,
    context_digest: ApprovalContextDigest,
    expires_at_ms: u128,
    source: ApprovalDecisionSource,
) -> ApprovalGrant {
    let grant_id = ApprovalGrantId::new(state.next_grant_id);
    state.next_grant_id = state
        .next_grant_id
        .checked_add(1)
        .expect("approval grant id exhausted");
    state.issued.insert(
        grant_id,
        IssuedGrant {
            request_id,
            action: action.clone(),
            context_digest: context_digest.clone(),
            expires_at_ms,
            source,
        },
    );
    ApprovalGrant {
        id: grant_id,
        request_id,
        action_digest: action.digest,
        context_digest,
        expires_at_ms,
        source,
    }
}
