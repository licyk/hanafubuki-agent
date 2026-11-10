//! The installed plugins' calls: their status, the reload that brings the
//! Agent's Skills, MCP servers and hooks up to date with them, and their
//! lifecycle (install, uninstall, enable, disable, update, retry) and the
//! marketplaces'. Every change goes through the one store, writes the
//! configuration, reloads, and tells the plugin event sink.

mod lifecycle;
mod marketplaces;

use std::sync::Arc;

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_hooks::HooksEngine;
use hanafubuki_agent_packages::{PackageStore, PluginId};
use hanafubuki_agent_plugin_packages::{GuestHost, GuestPackage};
use hanafubuki_agent_process::executor::ExecutorManager;

use super::{hooks, AgentApplicationService, AgentMcpApplication};
use crate::packages::{
    AgentPackageChange, AgentPackageChangeKind, AgentPackageStatus, AgentPackages,
};
use crate::skills::AgentSkillApplication;
use crate::AgentStoragePaths;

impl AgentApplicationService {
    /// Loads the installed plugins again as the settings leave them on and
    /// brings the Agent up to date without a restart: the Skill catalog
    /// reads the enabled packages' Skills, the next turn's hooks include
    /// their hooks, and their MCP servers run (one that was added or
    /// changed has connected, or failed, when this returns; one that is gone
    /// is disconnected). The facade calls it after a settings change and
    /// after every change to the installed packages; a caller that changes
    /// `[plugins]` or the store by other means calls it too.
    pub async fn reload_packages(&self) -> Result<(), String> {
        self.packages
            .reload(self.config.clone(), self.blocking.as_ref())
            .await
    }

    /// Every installed package, on or off, with the content it contributes,
    /// its hooks' trust and its issues, as last loaded.
    pub async fn packages(&self) -> Result<Vec<AgentPackageStatus>, String> {
        self.with_packages("agent plugin packages status", |packages, _| {
            Ok(packages.status())
        })
        .await
    }

    /// The installed package `id`; `None` when it is not installed.
    pub async fn package(&self, id: &str) -> Result<Option<AgentPackageStatus>, String> {
        let id = plugin_id(id)?.key();
        Ok(self
            .packages()
            .await?
            .into_iter()
            .find(|package| package.id == id))
    }

    /// Runs `work` on the blocking pool with the packages and the
    /// configuration: the store's changes, `git` and the configuration's
    /// writes all block.
    async fn with_packages<T: Send + 'static>(
        &self,
        label: &'static str,
        work: impl FnOnce(&AgentPackages, &AgentConfig) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let packages = self.packages.clone();
        let config = self.config.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            label,
            move || work(&packages, &config),
        )
        .await?
    }

    /// Tells the plugin event sink that `id` changed (`kind`) or, when
    /// `result` is an error, that the call on it failed; returns `result`.
    fn report_change<T>(
        &self,
        id: Option<&PluginId>,
        kind: AgentPackageChangeKind,
        result: Result<T, String>,
    ) -> Result<T, String> {
        if let Some(id) = id {
            let (kind, error) = match &result {
                Ok(_) => (kind, None),
                Err(error) => (AgentPackageChangeKind::Failed, Some(error.clone())),
            };
            self.plugin_events.package_changed(&AgentPackageChange {
                id: id.key(),
                kind,
                error,
            });
        }
        result
    }

    /// [`Self::reload_packages`] in the background, for the settings and
    /// `mcp.toml` writes, which do not wait for it.
    pub(super) fn spawn_package_reload(&self) {
        let service = self.clone();
        crate::kernel_tasks::spawn_kernel_task(
            &self.kernel_faults,
            "plugin packages",
            async move {
                if let Err(error) = service.reload_packages().await {
                    log::warn!("the installed plugins were not reloaded: {error}");
                }
            },
        );
    }
}

/// The package store under the data directory and the content of its
/// enabled packages, which the Skill catalog, the hooks (built here) and
/// the MCP side follow from the start, and the guest host of its code
/// packages.
pub(super) fn open_packages(
    storage_paths: &AgentStoragePaths,
    safe_mode: bool,
    executor: &ExecutorManager,
    config: &AgentConfig,
    skills: &AgentSkillApplication,
    mcp: &AgentMcpApplication,
    guests: impl FnOnce(Vec<GuestPackage>) -> GuestHost,
) -> Result<(AgentPackages, Arc<HooksEngine>), String> {
    let hooks = Arc::new(HooksEngine::new(
        hooks::hook_sources(config)?,
        storage_paths.hooks_state_file.clone(),
        executor.clone(),
    ));
    let packages = AgentPackages::open(
        PackageStore::open(storage_paths.plugins_root.clone(), executor.clone()),
        safe_mode,
        config,
        skills.clone(),
        mcp.clone(),
        hooks.clone(),
        guests,
    )?;
    Ok((packages, hooks))
}

/// `id`, `<plugin>@<marketplace>`.
fn plugin_id(id: &str) -> Result<PluginId, String> {
    PluginId::parse(id).map_err(|error| error.to_string())
}
