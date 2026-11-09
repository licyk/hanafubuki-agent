//! Approval rules: a plugin that owns an action can make its approval
//! stricter than the turn's permission mode. A rule never relaxes it: the
//! kernel combines every rule's verdict monotonically (a denial beats a
//! required human decision, which beats the mode's own flow), so the result
//! does not depend on which plugin registered first.

mod registry;

use hanafubuki_agent_approval::ApprovalAction;

use crate::AgentSessionOrigin;

pub use registry::{ApprovalRuleRegistry, ApprovalRuleSnapshot};

/// What a rule requires for one action, beyond the turn's permission mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalRuleVerdict {
    /// A person decides, even under `AutoReview` or `FullAccess`.
    RequireUser,
    /// The action does not run; `reason` reaches the model.
    Deny { reason: String },
}

/// Inspects an action that needs approval and may tighten how it is
/// approved.
///
/// `evaluate` is a pure, bounded inspection of the action: it does no I/O,
/// never blocks and keeps no state per call, so the kernel calls it for every
/// action on its turn task.
pub trait ApprovalRule: Send + Sync + 'static {
    /// Unique within the registry.
    fn id(&self) -> &str;

    /// `None` when the rule has no opinion on `action`, which a session of
    /// `origin` (`None` for a session without a parent) asks to run.
    fn evaluate(
        &self,
        origin: Option<AgentSessionOrigin>,
        action: &ApprovalAction,
    ) -> Option<ApprovalRuleVerdict>;
}
