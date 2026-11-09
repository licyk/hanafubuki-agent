//! The standalone terminal interface's preferences (`[tui]` of
//! `config.toml`). The Agent itself never reads them; a host's
//! configuration has the defaults.

use hanafubuki_agent_config::{AgentTuiPreferences, AgentTuiPreferencesPatch};

use super::AgentApplicationService;

impl AgentApplicationService {
    /// The preferences as the configuration's layers resolve them.
    pub async fn tui_preferences(&self) -> Result<AgentTuiPreferences, String> {
        let config = self.config.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent tui preferences read worker",
            move || config.tui_preferences(),
        )
        .await?
    }

    /// Writes the keys `patch` names into the user's `config.toml` and
    /// returns the preferences as the layers now resolve them. Nothing in
    /// the running Agent changes.
    pub async fn write_tui_preferences(
        &self,
        patch: AgentTuiPreferencesPatch,
    ) -> Result<AgentTuiPreferences, String> {
        let config = self.config.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent tui preferences write worker",
            move || config.write_tui_preferences(patch),
        )
        .await?
    }
}
