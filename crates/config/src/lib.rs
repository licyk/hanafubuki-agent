//! The Agent's own configuration (site/content/docs/configuration.en.mdx).
//!
//! One directory the host names holds three TOML files: `config.toml` (the
//! settings), `providers.toml` (the model Provider registry, tokens
//! included) and `mcp.toml` (the MCP servers, credentials included); the two
//! with credentials are owner-only. Reads are tolerant: a value that cannot
//! be used takes its default, unknown keys are kept, and a file that needed
//! repair is copied into `.recovery/` first. Writes edit the document in
//! place, so comments and unknown keys survive, and publish it atomically.
//!
//! The standalone Agent finds the directory itself ([`AgentDirs`]) and
//! layers a profile, a trusted project's file and `-c` overrides over the
//! user's `config.toml` ([`ConfigLayers`],
//! site/content/docs/config-layers.en.mdx). Role definitions are read from
//! each layer's `agents/` and `[agents]` ([`RoleSource`]).

mod dirs;
mod fields;
mod file;
mod keys;
mod layers;
mod mcp;
mod network;
mod overrides;
mod plugin_options;
mod providers;
mod recovery;
mod roles;
mod sections;
mod settings;
mod standalone;
mod store;
mod tui;

pub use dirs::{AgentDirs, AGENT_HOME_ENV};
pub use file::{write_atomic, FileAccess};
pub use hanafubuki_agent_foundation::settings::{
    SettingApply, SettingChoice, SettingCondition, SettingKind, SettingProblem, SettingSpec,
    SettingValue,
};
pub use hanafubuki_agent_packages::{
    PluginOptionSchema, ResolvedPluginOption, ResolvedPluginOptions, MAX_PLUGIN_OPTIONS,
};
pub use layers::{ConfigLayerSource, ConfigLayers, SettingSource, PROJECT_CONFIG_DIR};
pub use mcp::McpSettingsStore;
pub use network::AgentProxy;
pub use overrides::{parse_config_value, ConfigOverrides};
pub use plugin_options::{PluginOptionKey, PluginOptionsSettings};
pub use providers::{
    validate_agent_model_policy, validate_agent_provider, validate_agent_provider_base_url,
    validate_agent_provider_credential, validate_agent_provider_registry, AgentImageInputPolicy,
    AgentInputCapabilitiesSettings, AgentModelImageInput, AgentModelOverrideSettings,
    AgentModelPolicySettings, AgentModelProtocol, AgentProviderHeaderSettings,
    AgentProviderRegistrySettings, AgentProviderSettings, AgentRemoteCompactionCapability,
    AgentTurnBudgetSettings, DEFAULT_AGENT_AUTO_COMPACT_THRESHOLD_PERCENT,
    MAX_AGENT_CONSECUTIVE_STALLED_STEPS, MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS,
    MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS, MAX_AGENT_HOOK_FRAGMENT_BYTES,
    MAX_AGENT_IDLE_TIMEOUT_SECONDS, MAX_AGENT_IN_FLIGHT_TOOLS, MAX_AGENT_PENDING_INPUT_ITEMS,
    MAX_AGENT_PROVIDERS, MAX_AGENT_PROVIDER_CREDENTIAL_BYTES, MAX_AGENT_PROVIDER_HEADERS,
    MAX_AGENT_TOOL_INVOCATIONS, MAX_AGENT_TOOL_OUTPUT_BYTES,
};
pub use recovery::{AgentConfigProblem, AgentConfigRecovery};
pub use roles::{
    is_role_name, read_role_files, InstructionsMode, RoleDefinition, RoleFiles, RoleLayer,
    RoleOrigin, RoleSource, MAX_ROLE_NAME_BYTES, ROLE_DIR,
};
pub use sections::{config_sections, SettingState, SettingsGroup, SettingsSection};
pub use settings::{
    AgentBackgroundCompactionSettings, AgentCompactionSettings, AgentCompactionStrategy,
    AgentHierarchicalCompactionSettings, AgentMemorySettings, AgentModelDiscovery,
    AgentModelMetadataSettings, AgentPermissionMode, AgentSandboxMode, AgentSandboxWorkspaceWrite,
    AgentSettings, AgentSettingsPatch, AgentWebSearchProvider, AgentWindowsSandbox,
    AgentWindowsSettings, MAX_AGENT_DISABLED_PLUGINS, MAX_AGENT_PLUGIN_NAME_BYTES,
};
pub use standalone::{
    AgentLogLevel, AgentNetworkSettings, AgentStandaloneSettings, AgentStandaloneSettingsPatch,
};
pub use store::{
    read_mcp_document, read_providers_document, read_settings_document, AgentConfig, SettingChange,
    SettingWriteOp, SettingsWrite, SettingsWriteOutcome, AGENT_CREDENTIAL_FILES, AGENT_MCP_FILE,
    AGENT_PROVIDERS_FILE, AGENT_SETTINGS_FILE,
};
pub use tui::{
    AgentAlternateScreen, AgentScrollbackMouse, AgentTuiPreferences, AgentTuiPreferencesPatch,
    AgentTuiPreset, MAX_PRESET_INPUT_BYTES,
};
