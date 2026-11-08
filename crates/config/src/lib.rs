//! The Agent's own configuration (docs/extraction-plan.md, §6).
//!
//! One directory the host names holds three TOML files: `config.toml` (the
//! settings), `providers.toml` (the model Provider registry, tokens
//! included) and `mcp.toml` (the MCP servers, credentials included); the two
//! with credentials are owner-only. Reads are tolerant: a value that cannot
//! be used takes its default, unknown keys are kept, and a file that needed
//! repair is copied into `.recovery/` first. Writes edit the document in
//! place, so comments and unknown keys survive, and publish it atomically.

mod fields;
mod file;
mod mcp;
mod providers;
mod settings;
mod store;

pub use file::{write_atomic, FileAccess};
pub use mcp::McpSettingsStore;
pub use providers::{
    validate_agent_model_policy, validate_agent_provider, validate_agent_provider_base_url,
    validate_agent_provider_credential, validate_agent_provider_registry, AgentImageInputPolicy,
    AgentInputCapabilitiesSettings, AgentModelPolicySettings, AgentModelProtocol,
    AgentProviderHeaderSettings, AgentProviderRegistrySettings, AgentProviderSettings,
    AgentRemoteCompactionCapability, AgentTurnBudgetSettings,
    DEFAULT_AGENT_AUTO_COMPACT_THRESHOLD_PERCENT, MAX_AGENT_CONSECUTIVE_STALLED_STEPS,
    MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS, MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS,
    MAX_AGENT_HOOK_FRAGMENT_BYTES, MAX_AGENT_IDLE_TIMEOUT_SECONDS, MAX_AGENT_IN_FLIGHT_TOOLS,
    MAX_AGENT_PENDING_INPUT_ITEMS, MAX_AGENT_PROVIDERS, MAX_AGENT_PROVIDER_CREDENTIAL_BYTES,
    MAX_AGENT_PROVIDER_HEADERS, MAX_AGENT_TOOL_INVOCATIONS, MAX_AGENT_TOOL_OUTPUT_BYTES,
};
pub use settings::{
    agent_settings_fields, AgentMemorySettings, AgentPermissionMode, AgentSettings,
    AgentSettingsPatch, AgentWebSearchProvider, MAX_AGENT_DISABLED_PLUGINS,
    MAX_AGENT_PLUGIN_NAME_BYTES,
};
pub use store::{
    read_mcp_document, read_providers_document, read_settings_document, AgentConfig,
    AGENT_CREDENTIAL_FILES, AGENT_MCP_FILE, AGENT_PROVIDERS_FILE, AGENT_SETTINGS_FILE,
};
