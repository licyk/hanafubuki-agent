//! The Agent's settings: how it may act, what it remembers, which of its
//! plugins and installed packages run, how its web tools reach the network
//! and how it reduces a session's context.

use std::collections::{BTreeMap, BTreeSet};

pub use hanafubuki_agent_foundation::agent_settings::{
    AgentPermissionMode, AgentSandboxMode, AgentWindowsSandbox,
};
use hanafubuki_agent_foundation::settings::{SettingChoice, SettingKind, SettingSpec};
use hanafubuki_agent_packages::{is_package_enabled, MarketplaceRecord, PluginId, PluginsSettings};
use serde::{Deserialize, Deserializer, Serialize};

use crate::plugin_options::PluginOptionsSettings;
use crate::sections::{check_specs, described, SettingsGroup, SettingsSection};

pub(crate) mod compaction;
mod decode;
pub(crate) mod memory;
pub(crate) mod model_metadata;
mod packages;
pub(crate) mod sandbox;
pub(crate) mod web;

pub use compaction::{
    AgentBackgroundCompactionSettings, AgentCompactionSettings, AgentCompactionStrategy,
    AgentHierarchicalCompactionSettings,
};
pub(crate) use decode::decode_settings;
pub use memory::AgentMemorySettings;
pub use model_metadata::{AgentModelDiscovery, AgentModelMetadataSettings};
pub use sandbox::{AgentSandboxWorkspaceWrite, AgentWindowsSettings};
pub use web::AgentWebSearchProvider;

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
    /// Whether a conversation is resumed by itself when its background
    /// work reports (a sub-agent finishes or sends it a message). Each
    /// session takes it at its next activation.
    pub wake_on_background_tasks: bool,
    pub web_search_provider: AgentWebSearchProvider,
    pub searxng_base_url: Option<String>,
    pub web_fetch_use_jina_reader: bool,
    /// Whether a web fetch through a proxy URL leaves a page's name this
    /// machine cannot resolve to the proxy, instead of failing. Off by
    /// default: the Agent cannot check where such a name points.
    pub web_fetch_resolve_through_proxy: bool,
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
            wake_on_background_tasks: true,
            web_search_provider: AgentWebSearchProvider::Auto,
            searxng_base_url: None,
            web_fetch_use_jina_reader: false,
            web_fetch_resolve_through_proxy: false,
            compaction: AgentCompactionSettings::default(),
            model_metadata: AgentModelMetadataSettings::default(),
            plugins: PluginsSettings::default(),
            marketplaces: BTreeMap::new(),
            plugin_options: PluginOptionsSettings::default(),
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
    pub wake_on_background_tasks: Option<bool>,
    pub web_search_provider: Option<AgentWebSearchProvider>,
    /// `null` clears the endpoint; a missing key leaves it.
    #[serde(default, deserialize_with = "present")]
    pub searxng_base_url: Option<Option<String>>,
    pub web_fetch_use_jina_reader: Option<bool>,
    pub web_fetch_resolve_through_proxy: Option<bool>,
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
            sandbox::validate_sandbox_workspace_write(&workspace_write)?;
            settings.sandbox_workspace_write = workspace_write;
        }
        if let Some(windows) = self.windows {
            settings.windows = windows;
        }
        if let Some(memory) = self.memory {
            check_specs("memory", &memory)?;
            memory::validate_memory(&memory)?;
            settings.memory = memory;
        }
        if let Some(enabled) = self.wake_on_background_tasks {
            settings.wake_on_background_tasks = enabled;
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
                    web::normalize_search_endpoint(&url).map_err(|error| format!("searxngBaseUrl {error}"))
                })
                .transpose()?;
        }
        if let Some(enabled) = self.web_fetch_use_jina_reader {
            settings.web_fetch_use_jina_reader = enabled;
        }
        if let Some(enabled) = self.web_fetch_resolve_through_proxy {
            settings.web_fetch_resolve_through_proxy = enabled;
        }
        if let Some(compaction) = self.compaction {
            compaction::validate_compaction(&compaction)?;
            settings.compaction = compaction;
        }
        if let Some(model_metadata) = self.model_metadata {
            settings.model_metadata = model_metadata;
        }
        web::validate_search_pair(settings)
    }
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

/// `permissionMode`, `wakeOnBackgroundTasks` and `disabledPlugins`.
/// `logLevel` is the standalone Agent's ([`crate::standalone`]).
pub(crate) fn general_section() -> SettingsSection {
    SettingsSection::new(
        "general",
        "General",
        SettingsGroup::General,
        10,
        vec![
            SettingSpec {
                keywords: vec!["approval".into(), "permissions".into()],
                ..described(
                    SettingSpec::new(
                        "permissionMode",
                        "Permission mode",
                        SettingKind::Choice {
                            values: vec![
                                SettingChoice::new("manual", "Manual"),
                                SettingChoice::new("auto_review", "Auto review"),
                                SettingChoice::new("full_access", "Full access"),
                            ],
                        },
                    ),
                    "Which tool calls need an approval",
                )
            },
            SettingSpec {
                keywords: vec!["sub-agents".into(), "resume".into(), "wake".into()],
                ..described(
                    SettingSpec::new(
                        "wakeOnBackgroundTasks",
                        "Wake on background tasks",
                        SettingKind::Boolean,
                    ),
                    "Resume a conversation by itself when a sub-agent it started finishes or sends it a message",
                )
            },
            SettingSpec {
                advanced: true,
                ..described(
                    SettingSpec::new(
                        "disabledPlugins",
                        "Disabled plugins",
                        SettingKind::TextList {
                            maximum_items: Some(MAX_AGENT_DISABLED_PLUGINS as u32),
                            maximum_item_chars: Some(MAX_AGENT_PLUGIN_NAME_BYTES as u32),
                        },
                    ),
                    "The Agent plugins turned off",
                )
            },
        ],
    )
}

#[cfg(test)]
mod tests;
