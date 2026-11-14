//! The Agent's settings, and its plugins: start-up, status, enablement,
//! retry and shutdown.

use hanafubuki_agent_config::AgentSettings;

use super::AgentApplicationService;

/// The Agent's settings, with their defaults. A settings page reads their
/// descriptions from the catalog ([`AgentApplicationService::settings_catalog`]).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettingsView {
    pub settings: AgentSettings,
    pub defaults: AgentSettings,
}

impl AgentSettingsView {
    fn of(settings: AgentSettings) -> Self {
        Self {
            settings,
            defaults: AgentSettings::default(),
        }
    }
}
use crate::host::{report_reconfigured, AgentPluginStatus};

impl AgentApplicationService {
    /// Starts the Agent's plugins. Composition calls this once at start-up
    /// from the async runtime; every model operation also starts them if
    /// nothing did, before the kernel plans tools from their registries.
    pub async fn start_plugins(&self) {
        self.host.ensure_started().await;
    }

    /// Brings the plugins up to date with the latest settings, the Agent's
    /// and the host's network settings: disables and enables the rows
    /// `disabledPlugins` changed for, and
    /// restarts the settings-derived plugins whose configuration changed.
    /// Returns once the transitions are issued and the MCP proxy follow-up
    /// is scheduled, never waiting for either: each transition (bounded by
    /// the host's start-up timeout) is logged when it ends and the plugin
    /// event sink hears of it; a proxy change reconnects the Streamable HTTP
    /// servers in the background. Each reads the latest settings under its
    /// own lock, so of two quick changes the later one wins whatever order
    /// their work runs in. The installed plugins are reloaded in the
    /// background too ([`Self::reload_packages`]). Runs on the Agent
    /// runtime.
    pub fn apply_settings_to_plugins(&self) {
        for (name, settle) in self.host.reconfigure() {
            tokio::spawn(report_reconfigured(name, settle));
        }
        self.spawn_package_reload();
        // `wakeOnBackgroundTasks` is in every tree's background work.
        self.wakes.feed.hint_all();
        match self.network.proxy() {
            Ok(proxy) => self
                .core
                .set_model_proxy(crate::network::model_http_proxy(proxy)),
            Err(error) => log::warn!(
                "the model Providers keep their proxy: the host's proxy is not known ({error})"
            ),
        }
        let mcp = self.mcp_application.clone();
        let network = self.network.clone();
        crate::kernel_tasks::spawn_kernel_task(&self.kernel_faults, "settings", async move {
            if let Err(error) = mcp.apply_proxy_settings(network.as_ref()).await {
                log::warn!("MCP proxy settings were not applied: {error}");
            }
        });
    }

    /// Every Agent plugin's state, enablement and current contributions, for
    /// the plugin manager and diagnostics. Reads in-memory snapshots only.
    pub fn plugin_status(&self) -> Vec<AgentPluginStatus> {
        self.host.status()
    }

    /// Turns a plugin on or off: records it in `disabledPlugins` and applies
    /// the settings to the plugins. A core plugin cannot be disabled and an
    /// unknown name is refused. Returns the persisted settings; the plugin's
    /// own transition ends later and is announced through the plugin event
    /// sink.
    pub async fn set_plugin_enabled(
        &self,
        name: String,
        enabled: bool,
    ) -> Result<AgentSettings, String> {
        self.host.check_enablement(&name, enabled)?;
        let config = self.config.clone();
        let settings = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent plugin enablement worker",
            move || config.set_plugin_disabled(&name, !enabled),
        )
        .await??;
        self.apply_settings_to_plugins();
        Ok(settings)
    }

    /// Writes `patch` into the options of the plugin `name`
    /// (`[pluginOptions.<name>]` of the user's `config.toml`), each value
    /// checked against its descriptor's schema before anything is written
    /// (`null` returns an option to its default), then applies the settings
    /// to the plugins: a plugin that takes the change in place keeps
    /// running, another restarts. Returns the persisted settings.
    pub async fn set_plugin_options(
        &self,
        name: String,
        patch: serde_json::Map<String, serde_json::Value>,
    ) -> Result<AgentSettings, String> {
        let schema = self.host.option_schema(&name)?;
        let config = self.config.clone();
        let settings = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent plugin options write worker",
            move || config.set_plugin_options(&name, &schema, &patch),
        )
        .await??;
        self.apply_settings_to_plugins();
        Ok(settings)
    }

    /// The Agent's settings, with their defaults.
    pub async fn settings_view(&self) -> Result<AgentSettingsView, String> {
        Ok(AgentSettingsView::of(self.load_agent_settings().await?))
    }

    /// Applies `patch` to the Agent's settings, as a whole or not at all,
    /// brings the plugins up to date with the result and signals a memory
    /// scan, which memory just turned on starts with.
    pub async fn write_settings(
        &self,
        patch: hanafubuki_agent_config::AgentSettingsPatch,
    ) -> Result<AgentSettingsView, String> {
        let config = self.config.clone();
        let settings = hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent settings write worker",
            move || config.write_settings(patch),
        )
        .await??;
        self.apply_settings_to_plugins();
        self.memory.signal_scan(None);
        Ok(AgentSettingsView::of(settings))
    }

    /// Starts a failed Agent plugin again with its current configuration.
    pub async fn retry_plugin(&self, name: &str) -> Result<(), String> {
        self.host.retry(name).await
    }

    /// Panics in a call into the plugin `name` guarded as every call into a
    /// plugin is, as a panic in that plugin's own code would: the host stops
    /// it, shows it `Failed` with the panic, and starts it again under its
    /// restart policy. For the end-to-end tests of a built-in row's fault.
    #[cfg(any(test, feature = "test-support"))]
    pub fn panic_in_plugin(&self, name: &str) {
        let _ = self.host.faults().guard(
            name,
            hanafubuki_agent_host::FaultPoint::new("test call"),
            || panic!("a test made the plugin {name} panic"),
        );
    }

    /// Stops the Agent's plugins after turns are stopped and MCP is shut
    /// down; see `site/content/docs/architecture.en.mdx`, "Start and
    /// shutdown".
    pub async fn shutdown_plugins(
        &self,
        deadline: std::time::Instant,
    ) -> Vec<hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic> {
        self.host.shutdown(deadline).await
    }
}
