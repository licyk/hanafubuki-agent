//! Installed plugins for the Agent: packages in Codex's formats (and the
//! Claude Code and Cursor ones Codex reads, the Agent Plugins 1.0.0 one, and
//! this project's own `.hanafubuki-plugin`), installed from a directory or a
//! marketplace into a versioned cache under the data directory's
//! `plugins/`, with their content hash in `install.json`.
//!
//! A package is identified as `<plugin>@<marketplace>` ([`PluginId`]). Its
//! manifest is read field by field, as an external producer's: a field or a
//! component that cannot be used is left out with an issue
//! ([`load_manifest`]). An install copies the package into staging, writes
//! the marketplace entry's manifest when the package has none, checks the
//! staged manifest against the source, turns `commands/` into Skills and
//! swaps the copy in, keeping the previous version until the swap succeeded
//! ([`PackageStore`]). Marketplaces are added from a directory or a git
//! repository, upgraded and removed; the store returns their
//! `[marketplaces]` records and never writes the configuration
//! ([`PluginsSettings`], [`MarketplaceRecord`]). `git` runs through the
//! Agent's `ExecutorManager`.

mod commands;
mod error;
mod git;
mod hash;
mod id;
mod manifest;
mod marketplace;
mod options;
mod settings;
mod store;
mod version;

pub use commands::migrated_command_skills_root;
pub use error::PackageError;
pub use hash::content_hash;
pub use id::{validate_marketplace_name, validate_plugin_name, PluginId, PluginIdError};
pub use manifest::{
    find_manifest, load_manifest, parse_manifest, ContributedSection, ContributedTool,
    HooksDeclaration, LoadedManifest, ManifestError, ManifestFormat, ManifestInterface,
    ManifestPaths, McpServersDeclaration, PackageCode, PackageContributes, PackageManifest,
    PackagePermissions, PackageRuntime, ProcessRuntime, RuntimeKind, WasmRuntime,
    WasmRuntimeLimits, WorkspaceAccess, AGENT_PLUGINS_SCHEMA, DEFAULT_WASM_MEMORY_BYTES,
};
pub use marketplace::{
    find_marketplace_manifest, list_marketplaces, load_marketplace, InstallationPolicy,
    ManifestFallback, Marketplace, MarketplaceAddRequest, MarketplaceAdded, MarketplaceListing,
    MarketplacePlugin, MarketplaceUpgrade, PackageSource,
};
pub use options::{
    lower_config_schema, PluginOption, PluginOptionKind, PluginOptionSchema, PluginOptionValue,
    ResolvedPluginOption, ResolvedPluginOptions, MAX_PLUGIN_OPTIONS,
};
pub use settings::{
    is_package_enabled, MarketplaceRecord, MarketplaceSourceType, PluginDefaults,
    PluginMcpServerSettings, PluginSettings, PluginsSettings,
};
pub use store::{InstallOutcome, InstallRecord, InstalledPackage, PackageStore, UninstallOutcome};
pub use version::{active_version, compare_versions, validate_version_segment, LOCAL_VERSION};
