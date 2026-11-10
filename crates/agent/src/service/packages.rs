//! The installed plugins' calls: their status, and the reload that brings
//! the Agent's Skills, MCP servers and hooks up to date with them.

use std::sync::Arc;

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_hooks::HooksEngine;
use hanafubuki_agent_packages::PackageStore;
use hanafubuki_agent_process::executor::ExecutorManager;

use super::{hooks, AgentApplicationService, AgentMcpApplication};
use crate::packages::{AgentPackageStatus, AgentPackages};
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

    /// Every installed package, on or off, with the content it contributes
    /// and its issues, as last loaded.
    pub async fn packages(&self) -> Result<Vec<AgentPackageStatus>, String> {
        let packages = self.packages.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent plugin packages status",
            move || packages.status(),
        )
        .await
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
/// the MCP side follow from the start.
pub(super) fn open_packages(
    storage_paths: &AgentStoragePaths,
    executor: &ExecutorManager,
    config: &AgentConfig,
    skills: &AgentSkillApplication,
    mcp: &AgentMcpApplication,
) -> Result<(AgentPackages, Arc<HooksEngine>), String> {
    let hooks = Arc::new(HooksEngine::new(
        hooks::hook_sources(config)?,
        storage_paths.hooks_state_file.clone(),
        executor.clone(),
    ));
    let packages = AgentPackages::open(
        PackageStore::open(storage_paths.plugins_root.clone(), executor.clone()),
        config,
        skills.clone(),
        mcp.clone(),
        hooks.clone(),
    )?;
    Ok((packages, hooks))
}
