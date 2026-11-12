//! Extension-point contracts of the Agent plugin architecture.
//!
//! The Agent kernel and its plugins meet only here: the kernel owns the
//! registries and reads them as per-step snapshots, and plugins contribute to
//! them through registrations that kardia reverts when the registering plugin
//! stops. The registry, fault and worker mechanisms are the module host's
//! (`hanafubuki-agent-host`), re-exported here so plugins depend on
//! this crate alone. Domain crates never depend on this crate or on kardia.
//! See `site/content/docs/plugin-rules.en.mdx`.

pub mod activity;
pub mod admission;
pub mod approval;
pub mod context;
pub mod continuation;
pub mod fault;
pub mod host;
pub mod ids;
pub mod image;
pub mod memory;
pub mod observer;
pub mod progress;
pub mod prompt;
pub mod session;
pub mod skills;
pub mod tokens;
pub mod tool_lifecycle;
pub mod tools;

pub use activity::{AgentActivityAction, AgentActivityPresentation, AgentToolPresentation};
pub use admission::{
    TurnAdmissionContributor, TurnAdmissionFuture, TurnAdmissionRegistry, TurnAdmissionRequest,
};
pub use approval::{ApprovalRule, ApprovalRuleRegistry, ApprovalRuleSnapshot, ApprovalRuleVerdict};
pub use context::{
    ContextContributor, ContextFuture, ContextRegistry, ContextRenderer, ContextRendererRegistry,
    ContextRendererSnapshot, ContextRequest, ContextRole, ContextSection, ContextSectionKind,
    ContextSlot, WORLD_STATE_PREAMBLE,
};
pub use continuation::{
    AgentContinuationContext, AgentContinuationHook, AgentContinuationHookDecision,
    AgentContinuationHookError, AgentContinuationHookFailurePolicy, AgentContinuationHookFuture,
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    AgentContinuationHookRegistration, ContinuationHookRegistry,
    MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK, MAXIMUM_CONTINUATION_FRAGMENT_BYTES,
    MAXIMUM_CONTINUATION_HOOKS,
};
pub use fault::ExtensionPoint;
pub use hanafubuki_agent_foundation::agent_settings::AgentMemoryContributionMode;
/// What a contributor's call is cancelled through.
pub use hanafubuki_agent_foundation::cancellation::CancellationToken;
pub use hanafubuki_agent_host::{
    spawn_scoped_worker, EntryLease, PluginError, PluginFault, PluginFaults, PluginPanicked,
    Register, RegistrationError, Registry, RegistryEntry, RegistryService, RegistrySnapshot,
    DRAIN_GRACE, RETIRE_CANCEL_BOUND, WORKER_STOP_BOUND,
};
pub use host::BlockingService;
pub use ids::{
    AgentActivityId, AgentCompactionGeneration, AgentCompactionId, AgentContextWindowId,
    AgentEventSequence, AgentGeneration, AgentMessageId, AgentModelSessionGeneration,
    AgentReasoningId, AgentRevision, AgentSessionId, AgentSteeringSequence, AgentTurnId,
    AgentUserInputRequestId,
};
pub use image::{AgentImageDetail, PreparedAgentImage};
pub use memory::{
    AgentMemoryCitationSnapshot, AgentMemoryContributionCause, AgentMemoryContributionState,
    AgentMemoryStep, AgentMemoryStepPolicy, AgentProcessedMemoryAnswer,
};
pub use observer::{AgentNotification, AgentObserver, ObserverRegistry};
pub use progress::{
    AgentExecutionProgressStatus, AgentToolProgressSink, AgentToolProgressUpdate,
    NoopAgentToolProgressSink,
};
pub use prompt::{
    PromptContributor, PromptContributorRegistry, PromptContributorSnapshot, PromptFragment,
    PromptPrepareFuture, PromptRequest, PromptStep,
};
pub use session::{AgentSessionMode, AgentSessionOrigin, AgentTurnOrigin};
pub use skills::{SelectableSkill, SelectableSkills};
pub use tokens::{estimate_tokens, token_budget_bytes};
pub use tool_lifecycle::{
    PostToolUseDecision, PostToolUseFuture, PreToolUseDecision, PreToolUseFuture,
    ToolLifecycleCall, ToolLifecycleHook, ToolLifecycleHookError, ToolLifecycleHookRegistration,
    ToolLifecycleHookRegistry, ToolLifecyclePhase, MAXIMUM_TOOL_LIFECYCLE_HOOKS,
};
pub use tools::{
    decode_arguments, object_arguments, output_text, structured_output, verify_tool_approval,
    AgentListingEntry, AgentListingEntryKind, AgentMemoryContextEffect, AgentMutationKind,
    AgentPatchFile, AgentPatchLine, AgentPatchLineKind, AgentPatchOperation, AgentPatchPreview,
    AgentSearchMatch, AgentToolDisplay, AgentToolError, AgentToolHandler, AgentToolOmission,
    AgentToolPresenter, AgentToolSource, AgentWebResult, ArgumentSubjects, SourcedTool,
    ToolAvailability, ToolCallContext, ToolCallValidationError, ToolContextFragment,
    ToolDescriptor, ToolEffect, ToolFuture, ToolInvocation, ToolPlanContext, ToolPlanLease,
    ToolPresenterRegistry, ToolRegistry, ToolResult, ToolSourceContribution, ToolSourceRegistry,
    ToolSourceStep, ToolSourceTrust, TurnExtensions, MAX_AGENT_TOOL_OMISSIONS,
    MAX_TOOL_SUBJECT_CHARS, RESERVED_TOOL_NAMES,
};

/// Helpers for other crates' tests.
#[cfg(feature = "test-support")]
pub mod test_support {
    pub use hanafubuki_agent_host::test_support::detached_entry;
}
