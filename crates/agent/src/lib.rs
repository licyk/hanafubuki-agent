//! Tauri-free Agent application orchestration.
//!
//! The Agent's composition: its plugin host, its own plugins, slot and
//! service facade over the kernel, and the host interface an embedding
//! application supplies (`AgentHostPorts`: its plugin rows, workspace roots,
//! steering context and instructions). It knows no host domain; the
//! non-Agent application never depends on it.

mod attachments;
mod contracts;
mod conversation_reader;
mod data_dir_lock;
mod desktop;
mod file_search;
mod goals;
mod host;
mod host_inputs;
mod kernel_tasks;
pub mod logging;
mod memory;
mod network;
mod packages;
/// The Agent's half of the host's panic hook: its threads and guards are
/// marked in its own foundation, which the host's hook cannot see, so the
/// host's hook treats a panic as contained when this says so too.
///
/// The standalone binary installs the Agent foundation's hook itself:
/// `configure_crash_record(AgentStoragePaths::crash_record_file)`, then
/// `install_panic_hook(logging::logging_blocked)`.
pub mod panic {
    pub use hanafubuki_agent_foundation::panic::{
        catch_panic, catch_panic_async, configure_crash_record, install_panic_hook,
        is_panic_contained, recorded_panics, PanicFaultSummary,
    };
}
mod plugins;
mod roles;
/// The sandbox of the commands the model runs. A program that starts the
/// Agent calls [`sandbox::helper_dispatch`] first in its `main`, before any
/// thread starts: a sandboxed command re-executes that program as the
/// sandbox helper, and without the call every command that asks for a
/// sandbox fails.
pub mod sandbox {
    pub use hanafubuki_agent_sandbox::{
        helper_dispatch, platform_default_mode, Sandbox, SandboxStatus,
    };
}
mod service;
mod session_driver;
mod session_workspace;
mod shutdown;
mod side_conversation;
mod skill_watcher;
mod skills;
mod slot;
mod standalone;
mod startup;
mod storage_paths;
mod supervisor_runtime;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod titles;
mod workspace_instructions;
mod workspace_roots;

use attachments::{prepare_and_stage_images, prepare_view_image};
pub use attachments::{
    AgentImagePreview, DesktopAgentImageInput, MAXIMUM_IMAGE_DIMENSION, MAXIMUM_IMAGE_PIXELS,
    MAXIMUM_NORMALIZED_IMAGE_BYTES, MAXIMUM_SOURCE_IMAGES_BYTES, MAXIMUM_SOURCE_IMAGE_BYTES,
};
use contracts::{
    agent_base_instructions, model_api_protocol, model_compaction_capability,
    session_compaction_strategy, warn_unreachable_background_threshold,
};
pub use contracts::{AgentHostInstructions, AgentModelInfo};
pub use data_dir_lock::{DataDirLock, DATA_DIR_LOCK_FILE};
pub use desktop::DesktopAgent;
pub use file_search::{AgentFileCandidate, AgentFileSearch, AgentFileSearchResult};
pub use hanafubuki_agent_foundation::agent_settings::ModelReasoningEffort;
/// The secure store a host hands the Agent (`AgentHostPorts::secrets`), and
/// the blocking boundary its calls run on (`ContainedRuntime::blocking_port`).
pub use hanafubuki_agent_foundation::blocking::{run_blocking_operation, BlockingOperationPort};
pub use hanafubuki_agent_foundation::secrets::{
    SecretFuture, SecretKey, SecretStore, SecretStoreError,
};
/// What a hook in [`AgentHook`] is for, where it comes from and whether it
/// is trusted.
pub use hanafubuki_agent_hooks::{HookEventName, HookSource, HookTrust};
pub use hanafubuki_agent_memory::{AgentMemoryContributionMode, AgentMemoryStatus};
pub use hanafubuki_agent_packages::{
    ManifestFormat, MarketplaceAddRequest, MarketplaceSourceType, PackagePermissions,
    PackageSource, RuntimeKind, WorkspaceAccess,
};
pub use hanafubuki_agent_plugin_packages::{
    GuestExitCause, GuestExitReport, GuestState, GuestStatus,
};
pub use host::{
    agent_plugin_descriptors, AgentPluginContextContribution, AgentPluginContributions,
    AgentPluginDescriptor, AgentPluginEventSink, AgentPluginGroup, AgentPluginLock,
    AgentPluginOptionsStatus, AgentPluginRow, AgentPluginSettings, AgentPluginState,
    AgentPluginStatus, AgentPlugins, NoopAgentPluginEventSink,
};
pub use host_inputs::{
    AgentSteeringRequest, AgentTurnInput, AgentTurnRequest, HostContextPort, NoHostContext,
};
pub use memory::AgentMemoryApplication;
pub use network::{AgentProxy, NetworkSettingsPort};
pub use packages::validate_package;
pub use packages::{
    AgentMarketplace, AgentMarketplaceAdded, AgentMarketplacePlugin, AgentMarketplaceUpgrade,
    AgentPackageChange, AgentPackageChangeKind, AgentPackageContent, AgentPackageGuest,
    AgentPackageInstall, AgentPackageSource, AgentPackageStatus, AgentPackageUninstall,
    AgentPackageValidation, LOCAL_MARKETPLACE,
};
pub use plugins::{
    AgentGoalBudgets, AgentRecap, AgentReviewCodeLocation, AgentReviewCommit, AgentReviewFinding,
    AgentReviewLineRange, AgentReviewOutput, AgentReviewTarget, AgentReviewTargets,
    AgentSubagentState, AgentSubagentStatus, MAXIMUM_GOAL_OBJECTIVE_CHARS,
};
pub use roles::{
    AgentResolvedRole, AgentRoleCatalog, AgentRoleIssue, AgentRoleLink, AgentRoleSource,
    AgentUnusableRole, DEFAULT_ROLE, EXPLORER_ROLE, MAX_ROLE_CHAIN,
};
pub use service::{
    AgentApplicationService, AgentEditFromTurn, AgentHook, AgentHookHandler, AgentHooks,
    AgentHostPorts, AgentModelPolicyWriteRequest, AgentProviderDeleteRequest, AgentProviderDraft,
    AgentProviderEnabledRequest, AgentProviderModelCatalogRequest, AgentProviderModelCatalogResult,
    AgentProviderRegistrySnapshot, AgentProviderWriteRequest, AgentRunProjection,
    AgentServiceInputs, AgentSessionModelSelectionRequest, AgentSessionReasoningEffortRequest,
    AgentSettingsView, AgentSkillSessionState, AgentStandaloneSettingsView, AgentWorktree,
    LocalSkillInstallRequest, McpApplicationSnapshot, McpCredentialPresence,
    McpCredentialsWriteRequest, McpLocalPolicySummary, McpLocalServerStatus,
    McpLocalStatusSnapshot, McpOAuthCallbackRequest, McpOAuthCancelRequest, McpOAuthLoginRequest,
    McpPluginServer, McpPluginServerSnapshot, McpServerDeleteRequest, McpServerWriteRequest,
    McpToolSnapshot, SkillMcpDependencyStatus,
};
pub(crate) use service::{ResolvedAgentProvider, ResolvedModelPolicy};
pub(crate) use shutdown::shutdown_agent;
pub use shutdown::AGENT_SHUTDOWN_BUDGET;
pub use skills::{
    SkillAssetRead, SkillAvailability, SkillAvailabilityState, SkillCatalogEntry,
    SkillCatalogInvalidationPort, SkillCatalogListRequest, SkillCatalogSnapshot,
    SkillCreateRequest, SkillDeleteFileRequest, SkillDuplicateRequest, SkillError, SkillFileRead,
    SkillId, SkillInstallRequest, SkillInterface, SkillLoadIssue, SkillMcpTransport, SkillRead,
    SkillResourceDescriptor, SkillResourceKind, SkillResourceRead, SkillRoots, SkillScope,
    SkillSourceInspection, SkillToolDependency, SkillWriteFileRequest,
};
pub use slot::{
    AgentAvailability, AgentClosing, AgentDisabledReason, AgentHandle, AgentRestartPolicy,
    AgentSlot, AgentStartOptions, AgentStartStage, AgentStatus, AgentStatusEventSink,
    AgentUnavailable, NoopAgentStatusEventSink, AUTOMATIC_RESTART_DELAYS, AUTOMATIC_RESTART_WINDOW,
};
pub use standalone::{
    probe_keyring, AgentRuntime, AgentRuntimeHost, AgentRuntimeOptions, KEYRING_SERVICE,
};
#[cfg(test)]
pub(crate) use startup::start_agent;
pub(crate) use startup::{spawn_agent_start, AgentLauncher};
pub use startup::{
    AgentConstructor, AgentInstanceInputs, AgentLaunch, AgentStartMarker, AGENT_START_SETTLE,
    CRASH_LOOP_STARTS,
};
pub use storage_paths::AgentStoragePaths;
pub use workspace_roots::{agent_workspace_roots_from, AgentWorkspaceRoots, WorkspaceRootsPort};

/// The interface renders the backend's reading of a patch, so the shape of that
/// reading crosses the application boundary with it.
pub use hanafubuki_agent_workspace_fs::{
    PatchPreview, PatchPreviewFile, PatchPreviewLine, PatchPreviewLineKind, PatchPreviewOperation,
};

use hanafubuki_agent_runtime::AGENT_RUNTIME_THREAD_NAME;
use std::path::Path;

/// The Agent's configuration, which the host opens in the directory it names
/// and the standalone Agent layers.
pub use hanafubuki_agent_config::{
    parse_config_value, AgentAlternateScreen, AgentBackgroundCompactionSettings,
    AgentCompactionSettings, AgentCompactionStrategy, AgentConfig, AgentConfigProblem,
    AgentConfigRecovery, AgentDirs, AgentHierarchicalCompactionSettings, AgentImageInputPolicy,
    AgentInputCapabilitiesSettings, AgentLogLevel, AgentMemorySettings, AgentModelPolicySettings,
    AgentModelProtocol, AgentNetworkSettings, AgentProviderHeaderSettings, AgentProviderSettings,
    AgentRemoteCompactionCapability, AgentSandboxMode, AgentSandboxWorkspaceWrite, AgentSettings,
    AgentSettingsPatch, AgentStandaloneSettings, AgentStandaloneSettingsPatch, AgentTuiPreferences,
    AgentTuiPreferencesPatch, AgentTuiPreset, AgentTurnBudgetSettings, AgentWebSearchProvider,
    AgentWindowsSandbox, AgentWindowsSettings, ConfigLayerSource, ConfigLayers, ConfigOverrides,
    InstructionsMode, PluginOption, PluginOptionKey, PluginOptionKind, PluginOptionSchema,
    PluginOptionValue, PluginOptionsSettings, ResolvedPluginOption, ResolvedPluginOptions,
    RoleLayer, RoleOrigin, AGENT_CREDENTIAL_FILES, AGENT_HOME_ENV, AGENT_MCP_FILE,
    AGENT_PROVIDERS_FILE, AGENT_SETTINGS_FILE, MAX_AGENT_CONSECUTIVE_STALLED_STEPS,
    MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS, MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS,
    MAX_AGENT_HOOK_FRAGMENT_BYTES, MAX_AGENT_IDLE_TIMEOUT_SECONDS, MAX_AGENT_IN_FLIGHT_TOOLS,
    MAX_AGENT_PENDING_INPUT_ITEMS, MAX_AGENT_TOOL_INVOCATIONS, MAX_AGENT_TOOL_OUTPUT_BYTES,
    MAX_PLUGIN_OPTIONS,
};
pub use hanafubuki_agent_execution::{
    AgentBackgroundTerminalSnapshot, AgentBackgroundTerminalStopFailure,
    AgentBackgroundTerminalStopOutcome,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
pub use hanafubuki_agent_kernel::AgentError;
pub use hanafubuki_agent_kernel::{
    compress_call_ranges, AgentActivitySnapshot, AgentBackgroundCompactionState,
    AgentCompactionGeneration, AgentCompactionId, AgentCompactionOutcome, AgentCompactionStatus,
    AgentCompactionTrigger, AgentCompressRange, AgentCompressionStats, AgentCompressionStatus,
    AgentCompressionSummary, AgentContentStoreStats, AgentEvent, AgentEventBatch, AgentEventKind,
    AgentEventSequence, AgentEventSink, AgentGeneration, AgentGoal, AgentGoalOrigin,
    AgentGoalStatus, AgentHistorySortDirection, AgentHostContext, AgentImageAttachmentRef,
    AgentMessageId, AgentMessagePhase, AgentPermissionMode, AgentPlanImplementationRequest,
    AgentReasoningId, AgentReasoningPhase, AgentReasoningSnapshot, AgentRevision,
    AgentSessionForkOutcome, AgentSessionForkRequest, AgentSessionHeadForkOutcome,
    AgentSessionHeadForkRequest, AgentSessionId, AgentSessionLifecycle, AgentSessionListRequest,
    AgentSessionMode, AgentSessionOpenResult, AgentSessionOrigin, AgentSessionPage,
    AgentSessionParentFilter, AgentSessionRuntimeSnapshot, AgentSessionSummary, AgentSessionTitle,
    AgentSessionTitleRenameRequest, AgentSessionTitleSource, AgentSkillRef, AgentSteeringAccepted,
    AgentTranscriptPage, AgentTranscriptPageRequest, AgentTurnAccepted,
    AgentTurnCancellationOutcome, AgentTurnId, AgentTurnOutcome, AgentTurnPageRequest,
    AgentTurnPhase, AgentTurnTerminalReason, AgentUserInputAccepted, AgentUserInputAnswer,
    AgentUserInputRequest, AgentUserInputRequestId, ApprovalRequest, ApprovalRequestId,
    ApprovalUserDecision, ConfirmedAgentSessionDeletion, ConversationItem, ModelCatalogEntry,
    MAXIMUM_HOST_CONTEXT_BYTES,
};
/// What a role narrows (`AgentResolvedRole::scope`), and what a session's
/// snapshot shows of it.
pub use hanafubuki_agent_kernel::{
    AgentCapabilityScope, AgentCapabilityScopeSummary, AgentToolScope, ToolIdentity, ToolPattern,
};
pub use hanafubuki_agent_kernel::{AgentQuarantinedSession, AgentRecoveryReport};
use hanafubuki_agent_kernel::{AgentService, ModelCompactionCapability};
/// What a turn's events carry, which a terminal interface renders: the task
/// checklist, the questions the model asks, the approvals it needs, the
/// automatic reviews' verdicts and the output of a committed tool call.
pub use hanafubuki_agent_kernel::{
    AgentTaskPlanSnapshot, AgentTaskPlanStep, AgentTaskPlanStepStatus, AgentToolPresentation,
    AgentUserInputOption, AgentUserInputQuestion, ApprovalAction, ApprovalActionKind,
    PermissionReviewStatus, SandboxEscalation,
};
/// What the Agent tells a host about its MCP servers, and the servers it is
/// told to connect.
pub use hanafubuki_agent_mcp::{
    McpApprovalPolicy, McpConnectionPhase, McpCredentialKey, McpEventSink, McpExposurePolicy,
    McpOAuthPhase, McpOAuthPolicy, McpProtocolMode, McpServerConfig, McpServerId, McpServerStatus,
    McpStdioConfig, McpStreamableHttpConfig, McpTransportConfig, NoopMcpEventSink,
};
pub use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use hanafubuki_agent_workspace_fs::WorkspaceFsService;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;
