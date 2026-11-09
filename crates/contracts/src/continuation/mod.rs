//! Continuation hooks: when the model gives a final answer, the kernel asks
//! each registered hook, in `(priority, provider id, hook id)` order, whether
//! the turn may end. A hook can let it pass, stop the remaining hooks, or add
//! context fragments that the kernel commits before it samples again.

mod registry;

use std::future::Future;
use std::pin::Pin;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::ToolContractError;
use serde::{Deserialize, Serialize};

pub use registry::ContinuationHookRegistry;

use crate::{AgentGeneration, AgentRevision, AgentSessionId, AgentTurnId};

macro_rules! bounded_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: String) -> Result<Self, ToolContractError> {
                if value.is_empty()
                    || value.len() > 128
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
                {
                    return Err(ToolContractError::InvalidIdentifier);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

bounded_id!(AgentContinuationHookProviderId);
bounded_id!(AgentContinuationHookId);

/// The version of a hook's behaviour, recorded with every fragment it adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentContinuationHookGeneration(pub [u8; 32]);

pub const MAXIMUM_CONTINUATION_HOOKS: usize = 64;
pub const MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK: usize = 64;
pub const MAXIMUM_CONTINUATION_FRAGMENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentContinuationContext {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub sampling_step: u32,
    pub candidate_final_answer: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentContinuationHookDecision {
    Pass,
    /// Commit `fragments` (1 to [`MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK`],
    /// each non-blank and at most [`MAXIMUM_CONTINUATION_FRAGMENT_BYTES`])
    /// and sample again.
    Continue {
        fragments: Vec<String>,
    },
    /// End the turn without asking the remaining hooks.
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentContinuationHookError {
    Cancelled,
    Failed(String),
}

pub type AgentContinuationHookFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<AgentContinuationHookDecision, AgentContinuationHookError>>
            + Send
            + 'a,
    >,
>;

/// Decides whether a candidate final answer ends the turn.
///
/// `evaluate` observes `cancellation`, which the kernel cancels with the turn
/// and the registry cancels when the hook's plugin stops. The kernel bounds
/// each call by the turn's tool idle timeout.
pub trait AgentContinuationHook: Send + Sync {
    /// Whether the hook has anything to decide for `context`: a pure,
    /// bounded inspection. The kernel neither calls nor reports a hook that
    /// does not apply.
    fn applies(&self, _context: &AgentContinuationContext) -> bool {
        true
    }

    fn evaluate<'a>(
        &'a self,
        context: AgentContinuationContext,
        cancellation: CancellationToken,
    ) -> AgentContinuationHookFuture<'a>;
}

/// What a failing or timed-out hook does to the turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentContinuationHookFailurePolicy {
    FailTurn,
    Ignore,
}

/// One hook as a plugin registers it.
#[derive(Clone)]
pub struct AgentContinuationHookRegistration {
    pub provider_id: AgentContinuationHookProviderId,
    pub hook_id: AgentContinuationHookId,
    pub generation: AgentContinuationHookGeneration,
    pub priority: i32,
    pub failure_policy: AgentContinuationHookFailurePolicy,
    pub hook: std::sync::Arc<dyn AgentContinuationHook>,
}
