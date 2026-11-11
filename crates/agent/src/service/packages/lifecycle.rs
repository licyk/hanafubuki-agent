// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core-plugins/src/manager.rs` (`install_plugin`: the store's
// install, then `enabled = true` in `[plugins]`; `uninstall_plugin`: the
// store's uninstall, then the entry cleared) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: a package is installed from an added
// marketplace or from a directory (filed under `local` unless a
// marketplace is named), or a directory is linked; uninstall keeps the
// package's data unless asked to purge it, and a linked package's
// directory always; enable, disable, update (the same source again, never
// for a linked package) and retry are calls of their own; each change
// reloads the installed plugins and is told to the plugin event sink;
// remote plugins and apps are not ported.

use std::path::{Path, PathBuf};

use hanafubuki_agent_packages::{
    load_manifest, InstallOutcome, LinkOutcome, PackageSource, PackageStore, PluginId,
    PluginSettings,
};

use super::{plugin_id, AgentApplicationService};
use crate::packages::{
    configured_marketplace, AgentPackageChangeKind, AgentPackageInstall, AgentPackageSource,
    AgentPackageStatus, AgentPackageUninstall, AgentPackages, LOCAL_MARKETPLACE,
};
use hanafubuki_agent_config::AgentConfig;

impl AgentApplicationService {
    /// Installs a package from an added marketplace or a directory, or
    /// links a directory, and turns it on (`[plugins."<id>"] enabled =
    /// true`), then reloads: its Skills and MCP servers are there when this
    /// returns, and its hooks once trusted. Returns what it asks for: the
    /// hooks to trust.
    pub async fn install_package(
        &self,
        source: AgentPackageSource,
    ) -> Result<AgentPackageInstall, String> {
        let id = source_id(&source);
        let result = self.install(source).await;
        self.report_change(id.as_ref(), AgentPackageChangeKind::Installed, result)
    }

    async fn install(&self, source: AgentPackageSource) -> Result<AgentPackageInstall, String> {
        let outcome = self
            .with_packages("agent plugin install", move |packages, config| {
                let store = packages.store();
                let outcome: Installed = match source {
                    AgentPackageSource::Marketplace {
                        marketplace,
                        plugin,
                    } => install_from_marketplace(store, config, &marketplace, &plugin)?.into(),
                    AgentPackageSource::Directory { path, marketplace } => {
                        let id = directory_id(&path, marketplace.as_deref())?;
                        store
                            .install_directory(&path, &id)
                            .map_err(|error| error.to_string())?
                            .into()
                    }
                    AgentPackageSource::Linked { path, marketplace } => {
                        let id = directory_id(&path, marketplace.as_deref())?;
                        store
                            .link_directory(&path, &id)
                            .map_err(|error| error.to_string())?
                            .into()
                    }
                };
                config.set_package_settings(&outcome.id, Some(&enabled(true)))?;
                Ok(outcome)
            })
            .await?;
        self.installed(outcome).await
    }

    /// Installs the package `id` again from where it came from: its added
    /// marketplace (as that marketplace is now), else the directory it was
    /// installed from. It stays on or off as it was; a changed package's
    /// hooks wait to be trusted again. A linked package is refused: it is
    /// its directory as it is, and is never updated.
    pub async fn update_package(&self, id: &str) -> Result<AgentPackageInstall, String> {
        let id = plugin_id(id)?;
        let result = self.update(id.clone()).await;
        self.report_change(Some(&id), AgentPackageChangeKind::Updated, result)
    }

    async fn update(&self, id: PluginId) -> Result<AgentPackageInstall, String> {
        let outcome = self
            .with_packages("agent plugin update", move |packages, config| {
                let store = packages.store();
                let installed = store.read(&id).map_err(|error| error.to_string())?;
                let added = config.settings()?.marketplaces.contains_key(&id.marketplace_name);
                let source = installed.record.and_then(|record| record.source);
                match source {
                    Some(PackageSource::Linked { path }) => Err(format!(
                        "plugin `{id}` is linked to {}: it is that directory as it is, and is never updated",
                        path.display()
                    )),
                    _ if added => install_from_marketplace(
                        store,
                        config,
                        &id.marketplace_name,
                        &id.plugin_name,
                    ),
                    Some(PackageSource::Local { path }) => store
                        .install_directory(&path, &id)
                        .map_err(|error| error.to_string()),
                    _ => Err(format!(
                        "plugin `{id}` cannot be updated: marketplace `{}` is not added and it was not installed from a directory",
                        id.marketplace_name
                    )),
                }
            })
            .await?;
        self.installed(outcome.into()).await
    }

    /// Removes the installed package `id` and its `[plugins]` entry, then
    /// reloads. Its data directory is kept unless `purge_data`; a linked
    /// package's directory is always kept.
    pub async fn uninstall_package(
        &self,
        id: &str,
        purge_data: bool,
    ) -> Result<AgentPackageUninstall, String> {
        let id = plugin_id(id)?;
        let result = self.uninstall(id.clone(), purge_data).await;
        self.report_change(Some(&id), AgentPackageChangeKind::Removed, result)
    }

    async fn uninstall(
        &self,
        id: PluginId,
        purge_data: bool,
    ) -> Result<AgentPackageUninstall, String> {
        let outcome = self
            .with_packages("agent plugin uninstall", move |packages, config| {
                let outcome = packages
                    .store()
                    .uninstall(&id, purge_data)
                    .map_err(|error| error.to_string())?;
                config.set_package_settings(&id, None)?;
                if !outcome.removed {
                    return Err(format!("plugin `{id}` is not installed"));
                }
                Ok(outcome)
            })
            .await?;
        self.reload_packages().await?;
        Ok(AgentPackageUninstall {
            id: outcome.id.key(),
            purged_data: outcome.purged_data,
        })
    }

    /// Turns the installed package `id` on or off in `[plugins]` and
    /// reloads. A package whose marketplace is turned off cannot be turned
    /// on.
    pub async fn set_package_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<AgentPackageStatus, String> {
        let id = plugin_id(id)?;
        let kind = if enabled {
            AgentPackageChangeKind::Enabled
        } else {
            AgentPackageChangeKind::Disabled
        };
        let result = self.set_enabled(id.clone(), enabled).await;
        self.report_change(Some(&id), kind, result)
    }

    async fn set_enabled(&self, id: PluginId, on: bool) -> Result<AgentPackageStatus, String> {
        let key = id.key();
        self.with_packages("agent plugin enablement", move |packages, config| {
            packages
                .store()
                .read(&id)
                .map_err(|error| error.to_string())?;
            let marketplace_off = config
                .settings()?
                .marketplaces
                .get(&id.marketplace_name)
                .is_some_and(|marketplace| marketplace.enabled == Some(false));
            if on && marketplace_off {
                return Err(format!(
                    "plugin `{id}` stays off: its marketplace `{}` is turned off",
                    id.marketplace_name
                ));
            }
            config.set_package_settings(&id, Some(&enabled(on)))?;
            Ok(())
        })
        .await?;
        self.reload_packages().await?;
        self.installed_status(&key).await
    }

    /// Loads the installed plugins again and returns the package `id` as
    /// it is then: what a package's failure needs to be tried again. A
    /// package of content has nothing running, so a reload is all; a code
    /// package's guest that failed (its start, or more often than the
    /// restart policy allows) starts again, with an empty restart window.
    pub async fn retry_package(&self, id: &str) -> Result<AgentPackageStatus, String> {
        let plugin = plugin_id(id)?;
        self.reload_packages().await?;
        // A guest that has not failed, or a package without code, has
        // nothing to retry.
        let _ = self.packages.guests().retry(&plugin).await;
        self.installed_status(&plugin.key()).await
    }

    async fn installed_status(&self, id: &str) -> Result<AgentPackageStatus, String> {
        self.package(id)
            .await?
            .ok_or_else(|| format!("plugin `{id}` is not installed"))
    }

    /// Reloads after an install or a link of `outcome`, and reports it with
    /// the package's hooks that wait to be trusted.
    async fn installed(&self, outcome: Installed) -> Result<AgentPackageInstall, String> {
        self.reload_packages().await?;
        let id = outcome.id.clone();
        let (package, hooks) = self
            .with_packages("agent plugin install report", move |packages, _| {
                Ok((status_of(packages, &id), packages.hooks_to_trust(&id)))
            })
            .await?;
        let package = package.ok_or_else(|| format!("plugin `{}` is not installed", outcome.id))?;
        // What its code asks for and was not granted: the consent to ask.
        let permissions = package
            .guest
            .as_ref()
            .and_then(|guest| guest.ungranted())
            .cloned();
        Ok(AgentPackageInstall {
            package,
            root: outcome.root,
            migrated_commands: outcome.migrated_commands,
            issues: outcome.issues,
            hooks_to_trust: hooks.iter().map(super::super::hooks::agent_hook).collect(),
            permissions,
        })
    }
}

/// What an install or a link did, as its report needs it.
struct Installed {
    id: PluginId,
    root: PathBuf,
    migrated_commands: Vec<String>,
    issues: Vec<String>,
}

impl From<InstallOutcome> for Installed {
    fn from(outcome: InstallOutcome) -> Self {
        Self {
            id: outcome.id,
            root: outcome.root,
            migrated_commands: outcome.migrated_commands,
            issues: outcome.issues,
        }
    }
}

impl From<LinkOutcome> for Installed {
    fn from(outcome: LinkOutcome) -> Self {
        Self {
            id: outcome.id,
            root: outcome.root,
            migrated_commands: Vec::new(),
            issues: outcome.issues,
        }
    }
}

/// The id `source` installs, when it is known before the install reads
/// the package.
fn source_id(source: &AgentPackageSource) -> Option<PluginId> {
    match source {
        AgentPackageSource::Marketplace {
            marketplace,
            plugin,
        } => PluginId::new(plugin, marketplace).ok(),
        AgentPackageSource::Directory { path, marketplace }
        | AgentPackageSource::Linked { path, marketplace } => {
            directory_id(path, marketplace.as_deref()).ok()
        }
    }
}

/// The id of the package in the directory `path`: its manifest's name,
/// under `marketplace` ([`LOCAL_MARKETPLACE`] when `None`).
fn directory_id(path: &Path, marketplace: Option<&str>) -> Result<PluginId, String> {
    let name = load_manifest(path)
        .map_err(|error| error.to_string())?
        .manifest
        .name;
    PluginId::new(&name, marketplace.unwrap_or(LOCAL_MARKETPLACE))
        .map_err(|error| error.to_string())
}

/// Installs the entry `plugin` of the added marketplace `marketplace`.
fn install_from_marketplace(
    store: &PackageStore,
    config: &AgentConfig,
    marketplace: &str,
    plugin: &str,
) -> Result<InstallOutcome, String> {
    let settings = config.settings()?;
    let record = settings
        .marketplaces
        .get(marketplace)
        .ok_or_else(|| format!("marketplace `{marketplace}` is not added"))?;
    let marketplace =
        configured_marketplace(store, marketplace, record).map_err(|error| error.to_string())?;
    store
        .install_from_marketplace(&marketplace, plugin)
        .map_err(|error| error.to_string())
}

fn status_of(packages: &AgentPackages, id: &PluginId) -> Option<AgentPackageStatus> {
    let key = id.key();
    packages
        .status()
        .into_iter()
        .find(|package| package.id == key)
}

/// A `[plugins]` entry that sets `enabled` alone: the write merges it into
/// the user's file, so the entry's `mcpServers` stay as written.
fn enabled(enabled: bool) -> PluginSettings {
    PluginSettings {
        enabled: Some(enabled),
        ..PluginSettings::default()
    }
}
