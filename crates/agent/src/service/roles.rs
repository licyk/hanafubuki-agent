//! The role catalog of the instance (site/content/docs/roles.en.mdx).

use std::sync::Arc;

use super::AgentApplicationService;
use crate::roles::AgentRoleCatalog;

impl AgentApplicationService {
    /// The roles as the last settings or package publication left them:
    /// read when the Agent is built and again by every
    /// [`Self::reload_packages`], which each settings write runs. No
    /// session uses a role yet.
    pub fn roles(&self) -> Arc<AgentRoleCatalog> {
        self.roles.current()
    }

    /// Reads the roles again on the blocking pool; a failure keeps the
    /// catalog as it was.
    pub(super) async fn reload_roles(&self) {
        let roles = self.roles.clone();
        let config = self.config.clone();
        let rebuilt = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent role catalog rebuild",
            move || roles.rebuild(&config),
        )
        .await;
        if let Err(error) = rebuilt {
            log::warn!("the role catalog was not read again: {error}");
        }
    }
}
