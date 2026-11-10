//! The installed plugins' code: the user's grant of what it may reach
//! (`retry_package` also starts a guest that failed again).

use hanafubuki_agent_foundation::blocking::run_blocking_operation;
use hanafubuki_agent_packages::{PackagePermissions, PluginId};

use super::AgentApplicationService;

impl AgentApplicationService {
    /// Records that the user granted the code of the installed package `id`
    /// (`<plugin>@<marketplace>`) `permissions`, those its consent showed,
    /// then reloads the packages: a guest that waited for them starts, if
    /// its package is on. A guest starts only when what its manifest asks
    /// for is granted; an update that asks for more waits for a new grant.
    pub async fn grant_package_permissions(
        &self,
        id: &str,
        permissions: PackagePermissions,
    ) -> Result<(), String> {
        let id = PluginId::parse(id).map_err(|error| error.to_string())?;
        let packages = self.packages.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent plugin package grant",
            move || packages.grant(&id, permissions),
        )
        .await??;
        self.reload_packages().await
    }
}
