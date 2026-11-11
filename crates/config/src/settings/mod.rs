//! The Agent's settings: how it may act, what it remembers, which of its
//! plugins and installed packages run, how its web tools reach the network
//! and how it reduces a session's context.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub use hanafubuki_agent_foundation::agent_settings::{
    AgentPermissionMode, AgentSandboxMode, AgentWindowsSandbox,
};
use hanafubuki_agent_foundation::url_policy::{UrlPolicy, MAX_URL_BYTES};
use hanafubuki_agent_packages::{is_package_enabled, MarketplaceRecord, PluginId, PluginsSettings};
use serde::{Deserialize, Deserializer, Serialize};

use crate::plugin_options::PluginOptionsSettings;

mod compaction;
mod decode;
mod model_metadata;
mod packages;
mod schema;

pub use compaction::{
    AgentBackgroundCompactionSettings, AgentCompactionSettings, AgentCompactionStrategy,
    AgentHierarchicalCompactionSettings,
};
pub(crate) use decode::decode_settings;
pub use model_metadata::{AgentModelDiscovery, AgentModelMetadataSettings};
pub use schema::agent_settings_fields;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    pub permission_mode: AgentPermissionMode,
    /// What the commands the model runs can reach. `None` is the
    /// platform's default (`hanafubuki_agent_sandbox::platform_default_mode`),
    /// which is never written to the file, so it follows the platforms that
    /// gain a sandbox.
    pub sandbox_mode: Option<AgentSandboxMode>,
    pub sandbox_workspace_write: AgentSandboxWorkspaceWrite,
    pub windows: AgentWindowsSettings,
    pub memory: AgentMemorySettings,
    /// Names of the Agent plugins the user turned off. The plugin host leaves
    /// them unmounted and disposes one that is running when it is added; a
    /// name no plugin carries is kept but means nothing, and the core plugins
    /// ignore it. Changed through the plugin enablement call, never patched.
    pub disabled_plugins: BTreeSet<String>,
    pub web_search_provider: AgentWebSearchProvider,
    pub searxng_base_url: Option<String>,
    pub web_fetch_use_jina_reader: bool,
    pub compaction: AgentCompactionSettings,
    /// `[modelMetadata]`: whether the Agent lists the Providers' models by
    /// itself, and whether it downloads models.dev's catalogue. Read where
    /// it acts.
    pub model_metadata: AgentModelMetadataSettings,
    /// `[plugins]`: which installed packages are on. Changed by the package
    /// lifecycle ([`crate::AgentConfig::set_package_settings`]) or by hand,
    /// never patched.
    pub plugins: PluginsSettings,
    /// `[marketplaces]`: the marketplaces the user added, by name. Changed
    /// by the marketplace calls ([`crate::AgentConfig::set_marketplace`]).
    pub marketplaces: BTreeMap<String, MarketplaceRecord>,
    /// `[pluginOptions]`: the built-in plugins' options, by plugin and key.
    /// Changed by [`crate::AgentConfig::set_plugin_options`], which checks
    /// them against the plugin's schema, never patched.
    pub plugin_options: PluginOptionsSettings,
}

impl AgentSettings {
    /// Whether the installed package `id` is on: its marketplace is not
    /// turned off, and its entry's `enabled`, else `[plugins._default]`'s,
    /// else on.
    pub fn package_enabled(&self, id: &PluginId) -> bool {
        is_package_enabled(id, &self.plugins, &self.marketplaces)
    }
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            // The most restrictive mode, so an unreadable mode never grants
            // more than the user chose.
            permission_mode: AgentPermissionMode::Manual,
            sandbox_mode: None,
            sandbox_workspace_write: AgentSandboxWorkspaceWrite::default(),
            windows: AgentWindowsSettings::default(),
            memory: AgentMemorySettings::default(),
            disabled_plugins: BTreeSet::new(),
            web_search_provider: AgentWebSearchProvider::Auto,
            searxng_base_url: None,
            web_fetch_use_jina_reader: false,
            compaction: AgentCompactionSettings::default(),
            model_metadata: AgentModelMetadataSettings::default(),
            plugins: PluginsSettings::default(),
            marketplaces: BTreeMap::new(),
            plugin_options: PluginOptionsSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWebSearchProvider {
    Auto,
    Duckduckgo,
    Searxng,
}

impl AgentWebSearchProvider {
    pub const ALL: &'static [Self] = &[Self::Auto, Self::Duckduckgo, Self::Searxng];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Duckduckgo => "duckduckgo",
            Self::Searxng => "searxng",
        }
    }
}

/// Codex's `[sandbox_workspace_write]`: what workspace-write allows beside
/// the workspace roots and the temporary directories.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSandboxWorkspaceWrite {
    /// More directories commands may write in, each absolute.
    pub writable_roots: Vec<PathBuf>,
    /// Whether commands may use the network.
    pub network_access: bool,
}

/// Codex's `[windows]`: the Windows sandbox `sandboxMode` runs in. Unset,
/// a mode other than full access cannot be given on Windows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentWindowsSettings {
    pub sandbox: Option<AgentWindowsSandbox>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMemorySettings {
    pub enabled: bool,
    pub use_memories: bool,
    pub generate_memories: bool,
    pub exclude_external_context: bool,
    pub extraction_model: Option<String>,
    pub consolidation_model: Option<String>,
    pub max_source_age_days: u32,
    pub minimum_idle_hours: u32,
    pub max_sessions_per_pass: usize,
    pub max_stage1_outputs: usize,
    pub max_unused_days: u32,
}

impl Default for AgentMemorySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            use_memories: true,
            generate_memories: true,
            exclude_external_context: false,
            extraction_model: None,
            consolidation_model: None,
            max_source_age_days: 10,
            minimum_idle_hours: 6,
            max_sessions_per_pass: 2,
            max_stage1_outputs: 256,
            max_unused_days: 30,
        }
    }
}

/// At most this many names in `disabledPlugins`.
pub const MAX_AGENT_DISABLED_PLUGINS: usize = 64;
/// The longest Agent plugin name `disabledPlugins` holds.
pub const MAX_AGENT_PLUGIN_NAME_BYTES: usize = 128;

/// A change to the settings from the settings page: each field present is
/// replaced, and the result is validated as a whole before it is written.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSettingsPatch {
    pub permission_mode: Option<AgentPermissionMode>,
    /// `null` returns to the platform's default; a missing key leaves it.
    #[serde(default, deserialize_with = "present")]
    pub sandbox_mode: Option<Option<AgentSandboxMode>>,
    pub sandbox_workspace_write: Option<AgentSandboxWorkspaceWrite>,
    pub windows: Option<AgentWindowsSettings>,
    pub memory: Option<AgentMemorySettings>,
    pub web_search_provider: Option<AgentWebSearchProvider>,
    /// `null` clears the endpoint; a missing key leaves it.
    #[serde(default, deserialize_with = "present")]
    pub searxng_base_url: Option<Option<String>>,
    pub web_fetch_use_jina_reader: Option<bool>,
    pub compaction: Option<AgentCompactionSettings>,
    pub model_metadata: Option<AgentModelMetadataSettings>,
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

impl AgentSettingsPatch {
    pub(crate) fn apply(self, settings: &mut AgentSettings) -> Result<(), String> {
        if let Some(mode) = self.permission_mode {
            settings.permission_mode = mode;
        }
        if let Some(mode) = self.sandbox_mode {
            settings.sandbox_mode = mode;
        }
        if let Some(workspace_write) = self.sandbox_workspace_write {
            validate_sandbox_workspace_write(&workspace_write)?;
            settings.sandbox_workspace_write = workspace_write;
        }
        if let Some(windows) = self.windows {
            settings.windows = windows;
        }
        if let Some(memory) = self.memory {
            validate_memory(&memory)?;
            settings.memory = memory;
        }
        if let Some(provider) = self.web_search_provider {
            settings.web_search_provider = provider;
        }
        if let Some(url) = self.searxng_base_url {
            settings.searxng_base_url = url
                .map(|url| {
                    if url.trim().is_empty() {
                        return Err("searxngBaseUrl cannot be empty or whitespace-only; use null to clear it".to_string());
                    }
                    normalize_search_endpoint(&url).map_err(|error| format!("searxngBaseUrl {error}"))
                })
                .transpose()?;
        }
        if let Some(enabled) = self.web_fetch_use_jina_reader {
            settings.web_fetch_use_jina_reader = enabled;
        }
        if let Some(compaction) = self.compaction {
            compaction::validate_compaction(&compaction)?;
            settings.compaction = compaction;
        }
        if let Some(model_metadata) = self.model_metadata {
            settings.model_metadata = model_metadata;
        }
        validate_search_pair(settings)
    }
}

pub(crate) fn validate_sandbox_workspace_write(
    settings: &AgentSandboxWorkspaceWrite,
) -> Result<(), String> {
    if settings
        .writable_roots
        .iter()
        .any(|root| !root.is_absolute())
    {
        return Err("sandboxWorkspaceWrite.writableRoots must hold absolute paths".to_string());
    }
    Ok(())
}

pub(crate) fn validate_memory(settings: &AgentMemorySettings) -> Result<(), String> {
    if !(1..=3650).contains(&settings.max_source_age_days)
        || settings.minimum_idle_hours > 8760
        || !(1..=64).contains(&settings.max_sessions_per_pass)
        || !(1..=4096).contains(&settings.max_stage1_outputs)
        || !(1..=3650).contains(&settings.max_unused_days)
    {
        return Err("memory numeric settings are outside their supported bounds".into());
    }
    for (field, value) in [
        ("extractionModel", settings.extraction_model.as_deref()),
        (
            "consolidationModel",
            settings.consolidation_model.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.trim().is_empty() || value.len() > 256) {
            return Err(format!(
                "memory.{field} must contain 1 to 256 bytes or be null"
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_search_pair(settings: &AgentSettings) -> Result<(), String> {
    if settings.web_search_provider == AgentWebSearchProvider::Searxng
        && settings.searxng_base_url.is_none()
    {
        return Err("searxngBaseUrl is required when webSearchProvider is searxng".to_string());
    }
    Ok(())
}

/// A plugin name `disabledPlugins` can hold: 1 to
/// [`MAX_AGENT_PLUGIN_NAME_BYTES`] bytes without surrounding whitespace or
/// control characters. The plugin host decides what a name means.
pub(crate) fn is_plugin_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_AGENT_PLUGIN_NAME_BYTES
        && name.trim() == name
        && !name.chars().any(char::is_control)
}

pub(crate) fn validate_disabled_plugins(names: &BTreeSet<String>) -> Result<(), String> {
    if names.len() > MAX_AGENT_DISABLED_PLUGINS {
        return Err(format!(
            "disabledPlugins holds at most {MAX_AGENT_DISABLED_PLUGINS} plugin names"
        ));
    }
    if !names.iter().all(|name| is_plugin_name(name)) {
        return Err(format!(
            "disabledPlugins names must contain 1 to {MAX_AGENT_PLUGIN_NAME_BYTES} bytes without surrounding whitespace or control characters"
        ));
    }
    Ok(())
}

/// The SearXNG endpoint: an endpoint URL (no credentials, query or fragment)
/// within the URL bound, without a trailing `/`. This is the address's one
/// check: web search takes it as valid. The error completes a sentence that
/// starts with the field's name.
pub(crate) fn normalize_search_endpoint(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("cannot be empty".to_string());
    }
    UrlPolicy::ENDPOINT
        .within(MAX_URL_BYTES)
        .parse(raw)
        .map_err(|rejection| rejection.to_string())?;
    Ok(raw.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests;
