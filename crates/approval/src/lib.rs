//! Domain-neutral approval contracts and one-shot authorization authority.
//!
//! This crate decides whether one exact, already-resolved action may be
//! dispatched. It deliberately owns no process, task, filesystem, instance or
//! Tauri state.

mod model;
mod requirement;
mod service;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// Config persists the mode, so it is defined below both.
pub use hanafubuki_agent_foundation::agent_settings::AgentPermissionMode;
pub use model::{
    ApprovalAction, ApprovalActionDigest, ApprovalActionKind, ApprovalContext,
    ApprovalContextDigest, ApprovalContextScope, ApprovalDecision, ApprovalDecisionSource,
    ApprovalError, ApprovalGrantId, ApprovalOrigin, ApprovalRequest, ApprovalRequestId,
    ApprovalResolution, ApprovalRiskClass, ApprovalUserDecision, PermissionActionDetails,
};
pub use requirement::PermissionRequirement;
pub use service::{
    ApprovalClock, ApprovalGrant, ApprovalOutcome, ApprovalService, ApprovalWaiter,
    AuthorizedAction, ManualApprovalAdmission, SystemApprovalClock,
};
