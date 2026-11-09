//! Tauri-free Agent application orchestration.
//!
//! The Agent's composition: its plugin host, its own plugins, slot and
//! service facade over the kernel, and the host interface an embedding
//! application supplies (`AgentHostPorts`: its plugin rows, workspace roots,
//! steering context and instructions). It knows no host domain; the
//! non-Agent application never depends on it.

mod attachments;
mod contracts;
mod data_dir_lock;
mod desktop;
mod file_search;
mod host;
mod host_inputs;
mod kernel_tasks;
pub mod logging;
mod memory;
mod network;
/// The Agent's half of the host's panic hook: its threads and guards are
/// marked in its own foundation, which the host's hook cannot see, so the
/// host's hook treats a panic as contained when this says so too.
///
/// The standalone binary installs the Agent foundation's hook itself:
/// `configure_crash_record(AgentStoragePaths::crash_record_file)`, then
/// `install_panic_hook(logging::logging_blocked)`.
pub mod panic {
    pub use hanafubuki_agent_foundation::panic::{
        configure_crash_record, install_panic_hook, is_panic_contained, recorded_panics,
        PanicFaultSummary,
    };
}
mod plugins;
mod service;
mod shutdown;
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
use contracts::{agent_base_instructions, model_api_protocol, model_compaction_capability};
pub use contracts::{AgentHostInstructions, AgentModelInfo};
pub use data_dir_lock::{DataDirLock, DATA_DIR_LOCK_FILE};
pub use desktop::DesktopAgent;
pub use file_search::{AgentFileCandidate, AgentFileSearch, AgentFileSearchResult};
/// The secure store a host hands the Agent (`AgentHostPorts::secrets`), and
/// the blocking boundary its calls run on (`ContainedRuntime::blocking_port`).
pub use hanafubuki_agent_foundation::blocking::{run_blocking_operation, BlockingOperationPort};
pub use hanafubuki_agent_foundation::secrets::{
    SecretFuture, SecretKey, SecretStore, SecretStoreError,
};
pub use host::{
    AgentPluginContextContribution, AgentPluginContributions, AgentPluginEventSink,
    AgentPluginGroup, AgentPluginLock, AgentPluginRow, AgentPluginState, AgentPluginStatus,
    AgentPlugins, NoopAgentPluginEventSink,
};
pub use host_inputs::{AgentSteeringRequest, AgentTurnRequest, HostContextPort, NoHostContext};
pub use memory::AgentMemoryApplication;
pub use network::{AgentProxy, NetworkSettingsPort};
pub use service::{
    AgentApplicationService, AgentHostPorts, AgentModelPolicyWriteRequest,
    AgentProviderDeleteRequest, AgentProviderDraft, AgentProviderEnabledRequest,
    AgentProviderModelCatalogRequest, AgentProviderModelCatalogResult,
    AgentProviderRegistrySnapshot, AgentProviderWriteRequest, AgentRunProjection,
    AgentServiceInputs, AgentSessionModelSelectionRequest, AgentSessionReasoningEffortRequest,
    AgentSettingsView, AgentSkillSessionState, LocalSkillInstallRequest, McpApplicationSnapshot,
    McpCredentialPresence, McpCredentialsWriteRequest, McpLocalPolicySummary, McpLocalServerStatus,
    McpLocalStatusSnapshot, McpOAuthCallbackRequest, McpOAuthCancelRequest, McpOAuthLoginRequest,
    McpServerDeleteRequest, McpServerWriteRequest, McpToolSnapshot, SkillMcpDependencyStatus,
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
    AgentConfig, AgentDirs, AgentLogLevel, AgentModelProtocol, AgentSettings, AgentSettingsPatch,
    AgentStandaloneSettings, ConfigLayerSource, ConfigLayers, ConfigOverrides,
    AGENT_CREDENTIAL_FILES, AGENT_HOME_ENV, AGENT_MCP_FILE, AGENT_PROVIDERS_FILE,
    AGENT_SETTINGS_FILE,
};
pub use hanafubuki_agent_execution::{
    AgentBackgroundTerminalSnapshot, AgentBackgroundTerminalStopFailure,
    AgentBackgroundTerminalStopOutcome,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
pub use hanafubuki_agent_kernel::{
    AgentActivitySnapshot, AgentCompactionGeneration, AgentCompactionId, AgentCompactionOutcome,
    AgentCompactionStatus, AgentEvent, AgentEventBatch, AgentEventKind, AgentEventSequence,
    AgentEventSink, AgentGeneration, AgentHistorySortDirection, AgentHostContext,
    AgentImageAttachmentRef, AgentMessageId, AgentMessagePhase, AgentPermissionMode,
    AgentPlanImplementationRequest, AgentReasoningPhase, AgentReasoningSnapshot, AgentRevision,
    AgentSessionForkOutcome, AgentSessionForkRequest, AgentSessionHeadForkOutcome,
    AgentSessionHeadForkRequest, AgentSessionId, AgentSessionLifecycle, AgentSessionListRequest,
    AgentSessionMode, AgentSessionOpenResult, AgentSessionPage, AgentSessionRuntimeSnapshot,
    AgentSessionSummary, AgentSessionTitle, AgentSessionTitleRenameRequest,
    AgentSessionTitleSource, AgentSkillRef, AgentSteeringAccepted, AgentTranscriptPage,
    AgentTranscriptPageRequest, AgentTurnAccepted, AgentTurnCancellationOutcome, AgentTurnId,
    AgentTurnInput, AgentTurnOutcome, AgentTurnPageRequest, AgentTurnPhase,
    AgentTurnTerminalReason, AgentUserInputAccepted, AgentUserInputAnswer, AgentUserInputRequest,
    AgentUserInputRequestId, ApprovalRequest, ApprovalRequestId, ApprovalUserDecision,
    ConfirmedAgentSessionDeletion, ConversationItem, ModelCatalogEntry, MAXIMUM_HOST_CONTEXT_BYTES,
};
pub use hanafubuki_agent_kernel::{AgentQuarantinedSession, AgentRecoveryReport};
use hanafubuki_agent_kernel::{AgentService, ModelCompactionCapability};
/// What a turn's events carry, which a terminal interface renders: the task
/// checklist, the questions the model asks, the approvals it needs and the
/// output of a committed tool call.
pub use hanafubuki_agent_kernel::{
    AgentTaskPlanSnapshot, AgentTaskPlanStep, AgentTaskPlanStepStatus, AgentToolPresentation,
    AgentUserInputOption, AgentUserInputQuestion, ApprovalAction, ApprovalActionKind,
};
/// What the Agent tells a host about its MCP servers, and the servers it is
/// told to connect.
pub use hanafubuki_agent_mcp::{
    McpConnectionPhase, McpEventSink, McpServerConfig, McpServerId, McpStdioConfig,
    McpStreamableHttpConfig, McpTransportConfig, NoopMcpEventSink,
};
pub use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use hanafubuki_agent_workspace_fs::WorkspaceFsService;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;
