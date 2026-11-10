// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/config/src/types.rs` (`[plugins]` with `_default`, a plugin's
// MCP overlay, `[marketplaces]` and `allows_plugin`) at commit
// `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the keys are camelCase; remote, release,
// provisioning, app and approval-mode fields are not ported; a marketplace
// carries `enabled`, which turns its packages off; the configuration crate
// reads these tables tolerantly and owns their files.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::PluginId;

/// `[plugins]`: which installed packages are on. An unset value serializes
/// as `null`, which a configuration write removes from the file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PluginsSettings {
    /// `[plugins._default]`: for a package whose entry does not say.
    #[serde(rename = "_default")]
    pub default: Option<PluginDefaults>,
    /// Keyed by plugin id (`<plugin>@<marketplace>`).
    #[serde(flatten)]
    pub packages: BTreeMap<String, PluginSettings>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDefaults {
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettings {
    pub enabled: Option<bool>,
    /// The overlay on the package's MCP servers, by server name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp_servers: BTreeMap<String, PluginMcpServerSettings>,
}

/// A package's MCP server as the user restricts it; how it runs stays the
/// package's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMcpServerSettings {
    /// Unset is on.
    pub enabled: Option<bool>,
    pub enabled_tools: Option<Vec<String>>,
    pub disabled_tools: Option<Vec<String>>,
}

/// `[marketplaces.<name>]`: a marketplace the user added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceRecord {
    pub source_type: MarketplaceSourceType,
    /// A git URL, or a local marketplace's directory.
    pub source: String,
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
    pub sparse_paths: Option<Vec<String>>,
    /// When it was last added or upgraded (RFC 3339, UTC).
    pub last_updated: Option<String>,
    /// The git revision checked out.
    pub last_revision: Option<String>,
    /// Off turns off every package from it, whatever `[plugins]` says;
    /// unset is on.
    pub enabled: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceSourceType {
    Git,
    Local,
}

impl PluginsSettings {
    /// Whether the user's settings leave `id` on: its entry's `enabled`,
    /// else `_default`'s, else on.
    pub fn allows(&self, id: &PluginId) -> bool {
        self.packages
            .get(&id.key())
            .and_then(|settings| settings.enabled)
            .or(self.default.as_ref().map(|default| default.enabled))
            .unwrap_or(true)
    }
}

/// Whether the installed package `id` is on: its marketplace is not turned
/// off, and `[plugins]` allows it.
pub fn is_package_enabled(
    id: &PluginId,
    plugins: &PluginsSettings,
    marketplaces: &BTreeMap<String, MarketplaceRecord>,
) -> bool {
    marketplaces
        .get(&id.marketplace_name)
        .is_none_or(|marketplace| marketplace.enabled != Some(false))
        && plugins.allows(id)
}
