//! The reload of the installed plugins: the packages read again and the
//! role catalog rebuilt with their roles. The service runs it after each
//! change it makes (`reload_packages`), and the `plugins` row runs it in
//! the background when a linked package's directory changed.

use std::sync::Arc;

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_foundation::blocking::{run_blocking_operation, BlockingOperationPort};
use hanafubuki_agent_kernel::AgentKernelFaultSink;

use super::AgentPackages;
use crate::roles::AgentRoles;

#[derive(Clone)]
pub(crate) struct PackageReload {
    pub(crate) packages: AgentPackages,
    pub(crate) config: AgentConfig,
    pub(crate) blocking: Arc<dyn BlockingOperationPort>,
    pub(crate) roles: Arc<AgentRoles>,
    /// Where a panic of a reload in the background goes.
    pub(crate) faults: Arc<dyn AgentKernelFaultSink>,
}

impl PackageReload {
    /// Loads the packages again and rebuilds the role catalog, whether or
    /// not the packages loaded; a catalog that does not build stays as it
    /// was.
    pub(crate) async fn run(&self) -> Result<(), String> {
        let loaded = self
            .packages
            .reload(self.config.clone(), self.blocking.as_ref())
            .await;
        let (roles, config, packages) = (
            self.roles.clone(),
            self.config.clone(),
            self.packages.clone(),
        );
        let rebuilt = run_blocking_operation(
            self.blocking.as_ref(),
            "agent role catalog rebuild",
            move || roles.rebuild(&config, || packages.loaded().roles()),
        )
        .await;
        if let Err(error) = rebuilt {
            log::warn!("the role catalog was not read again: {error}");
        }
        loaded
    }

    /// [`Self::run`] in the background, which nothing waits for.
    pub(crate) fn spawn(&self) {
        let reload = self.clone();
        crate::kernel_tasks::spawn_kernel_task(&self.faults, "plugin packages", async move {
            if let Err(error) = reload.run().await {
                log::warn!("the installed plugins were not reloaded: {error}");
            }
        });
    }
}
