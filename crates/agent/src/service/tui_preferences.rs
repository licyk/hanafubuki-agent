//! The standalone terminal interface's preferences (`[tui]` of
//! `config.toml`). The Agent itself never reads them; a host's
//! configuration has the defaults.

use hanafubuki_agent_config::{
    AgentStandaloneSettings, AgentStandaloneSettingsPatch, AgentTuiPreferences,
    AgentTuiPreferencesPatch, ConfigLayerSource,
};

use super::AgentApplicationService;
use crate::host::AgentHost;

/// The standalone settings the terminal interface shows around its Agent
/// settings. Embedded hosts have no use for this view: they supply the
/// logger, proxy and interface themselves.
#[derive(Clone, Debug)]
pub struct AgentStandaloneSettingsView {
    pub settings: AgentStandaloneSettings,
    pub layers: Vec<ConfigLayerSource>,
    pub warnings: Vec<String>,
}

impl AgentStandaloneSettingsView {
    /// The view of `config`, whose warnings include the plugin options
    /// `host`'s rows cannot use.
    fn of(config: &hanafubuki_agent_config::AgentConfig, host: &AgentHost) -> Result<Self, String> {
        let mut warnings = config.layer_warnings()?;
        warnings.extend(host.option_issues(&config.settings()?));
        Ok(Self {
            settings: config.standalone_settings()?,
            layers: config.layer_sources()?,
            warnings,
        })
    }
}

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

    /// The standalone settings and configuration layers the terminal
    /// interface's configuration center shows.
    pub async fn standalone_settings_view(&self) -> Result<AgentStandaloneSettingsView, String> {
        let config = self.config.clone();
        let host = self.host.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent standalone settings read worker",
            move || AgentStandaloneSettingsView::of(&config, &host),
        )
        .await?
    }

    /// Writes standalone keys through the running Agent's configuration and
    /// brings its network consumers up to date.
    pub async fn write_standalone_settings(
        &self,
        patch: AgentStandaloneSettingsPatch,
    ) -> Result<AgentStandaloneSettingsView, String> {
        let config = self.config.clone();
        let host = self.host.clone();
        let written = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent standalone settings write worker",
            move || {
                config.write_standalone_settings(patch)?;
                AgentStandaloneSettingsView::of(&config, &host)
            },
        )
        .await??;
        self.apply_settings_to_plugins();
        Ok(written)
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
