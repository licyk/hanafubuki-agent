//! Tool lifecycle hooks: around each tool call of a sampling step, the
//! kernel asks the hooks that apply to it, in `(priority, provider id, hook
//! id)` order. Before the call a hook may deny it; after the call it may add
//! context fragments, which the kernel commits after the step's outputs.
//!
//! A hook that fails or times out has no say (the call runs, or its output
//! stands): hooks are the user's policy scripts, and a broken one must not
//! stop the Agent's tools. A denial is the only way a hook changes a call.

mod registry;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::ToolOutput;

pub use registry::ToolLifecycleHookRegistry;

use crate::{
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    AgentGeneration, AgentSessionId, AgentTurnId,
};

pub const MAXIMUM_TOOL_LIFECYCLE_HOOKS: usize = 64;

/// When a hook is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolLifecyclePhase {
    /// Before the call's permission and approval; a hook may deny it.
    PreToolUse,
    /// After the call produced its output; a hook may add context.
    PostToolUse,
}

/// The call a hook is asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolLifecycleCall {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub call_id: String,
    /// The planned tool's name, flattened (`namespace__name`).
    pub tool_name: String,
    /// The arguments the tool receives.
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreToolUseDecision {
    Allow,
    /// The call does not run; `reason` is its model-visible error.
    Deny {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostToolUseDecision {
    Pass,
    /// Commit `fragments` (1 to
    /// [`crate::MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK`], each non-blank
    /// and at most [`crate::MAXIMUM_CONTINUATION_FRAGMENT_BYTES`]) after the
    /// step's tool outputs.
    AddContext {
        fragments: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolLifecycleHookError {
    Cancelled,
    Failed(String),
}

pub type PreToolUseFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PreToolUseDecision, ToolLifecycleHookError>> + Send + 'a>>;

pub type PostToolUseFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PostToolUseDecision, ToolLifecycleHookError>> + Send + 'a>>;

/// Runs around tool calls.
///
/// `applies` is a pure, bounded inspection: the kernel neither calls nor
/// reports a hook that does not apply to a call, so a hook with nothing to
/// do costs a call nothing. The other methods observe `cancellation`, which
/// the kernel cancels with the turn and the registry when the hook's plugin
/// stops; the kernel bounds each by the turn's tool idle timeout.
pub trait ToolLifecycleHook: Send + Sync {
    fn applies(&self, phase: ToolLifecyclePhase, call: &ToolLifecycleCall) -> bool;

    fn before_tool_use<'a>(
        &'a self,
        call: ToolLifecycleCall,
        cancellation: CancellationToken,
    ) -> PreToolUseFuture<'a>;

    fn after_tool_use<'a>(
        &'a self,
        call: ToolLifecycleCall,
        output: ToolOutput,
        cancellation: CancellationToken,
    ) -> PostToolUseFuture<'a>;
}

/// One hook as a plugin registers it. Its context fragments are committed
/// as continuation context under its identity, so it takes a continuation
/// hook's identity types.
#[derive(Clone)]
pub struct ToolLifecycleHookRegistration {
    pub provider_id: AgentContinuationHookProviderId,
    pub hook_id: AgentContinuationHookId,
    pub generation: AgentContinuationHookGeneration,
    pub priority: i32,
    pub hook: Arc<dyn ToolLifecycleHook>,
}
