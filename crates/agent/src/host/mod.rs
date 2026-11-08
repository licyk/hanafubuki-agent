//! The Agent's kardia plugin runtime: its configuration of the generic
//! module host (`hanafubuki_agent_host::ModuleHost`).
//!
//! The host mounts the kernel plugin, which publishes the kernel's
//! registries, and then every row of the plugin manifest. It starts from
//! async code on the Agent runtime: composition starts it explicitly at
//! application start, and the first model operation (a turn or a compaction)
//! starts it if nothing did (tests). The kernel reads the registries
//! directly; it never waits for the host.
//!
//! What is the Agent's here: the two core rows (host services and kernel),
//! the manifest with the host's rows in it, configuration derived from the
//! Agent's settings and the host's network settings, enablement from
//! `disabledPlugins`, the plugin groups, the contributions each plugin's
//! status lists, the plugin event sink, and the kernel fault sink a panic in
//! the host's own tasks reaches. The mechanics (mount order, bounds, retire
//! and remount, reconfigure, retry, safe mode, faults, shutdown) are the
//! generic host's (`docs/rules/plugins.md`).

mod contributions;
mod events;
mod manifest;
mod status;

use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent_config::{AgentConfig, AgentSettings};
use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_foundation::blocking::BlockingOperationPort;
use hanafubuki_agent_host::{HostProfile, ModuleHost, ModuleHostOptions, ModuleRow, RestartPolicy};
#[cfg(test)]
pub(crate) use hanafubuki_agent_host::{PLUGIN_START_TIMEOUT, PLUGIN_STOP_BOUND};
use hanafubuki_agent_kernel::{kernel_plugin, AgentKernelFaultSink, AgentKernelRegistries};
use kardia_core::{Context, PluginDef, Service, Settle};

pub use events::{AgentPluginEventSink, NoopAgentPluginEventSink};
pub(crate) use manifest::{agent_plugin_manifest, AgentPluginInputs};
pub use status::{
    AgentPluginContextContribution, AgentPluginContributions, AgentPluginGroup, AgentPluginLock,
    AgentPluginState, AgentPluginStatus,
};

/// How one Agent instance's host runs.
pub(crate) struct AgentHostOptions {
    /// Every row outside the core stays unmounted, whatever the settings
    /// say; the saved `disabledPlugins` is not changed.
    pub safe_mode: bool,
    /// Where a panic in the host's own tasks is reported.
    pub kernel_faults: Arc<dyn AgentKernelFaultSink>,
    /// When a plugin stopped by its fault starts again by itself: the
    /// application modules' policy (2 s, 10 s, 30 s, three in ten minutes).
    pub restart_policy: RestartPolicy,
}

impl Default for AgentHostOptions {
    fn default() -> Self {
        Self {
            safe_mode: false,
            kernel_faults: Arc::new(hanafubuki_agent_kernel::NoopAgentKernelFaultSink),
            restart_policy: RestartPolicy::default(),
        }
    }
}

/// One plugin to mount, its group, and where its configuration comes from.
/// A host builds its own rows (`AgentHostPorts::plugins`) with
/// `AgentPluginRow::unconfigured`.
pub type AgentPluginRow = ModuleRow<AgentPlugins>;

/// What the rows' configuration and enablement derive from: the Agent's
/// settings and the host's proxy.
pub struct AgentPluginSettings {
    pub(crate) agent: AgentSettings,
    pub(crate) proxy: Result<crate::AgentProxy, String>,
}

/// The Agent's profile of the generic host.
pub struct AgentPlugins {
    config: AgentConfig,
    network: Arc<dyn crate::NetworkSettingsPort>,
    events: Arc<dyn AgentPluginEventSink>,
    kernel_faults: Arc<dyn AgentKernelFaultSink>,
}

impl HostProfile for AgentPlugins {
    type Group = AgentPluginGroup;
    type Settings = AgentPluginSettings;

    const NOUN: &'static str = "Agent plugin";
    const HOST: &'static str = "the Agent";
    const SHUTDOWN_WORKER: &'static str = "agent-plugins";

    fn settings(&self) -> Option<AgentPluginSettings> {
        Some(AgentPluginSettings {
            agent: self.config.settings_snapshot()?,
            proxy: self.network.proxy(),
        })
    }

    fn disables(settings: &AgentPluginSettings, name: &str) -> bool {
        settings.agent.disabled_plugins.contains(name)
    }

    fn modules_changed(&self) {
        self.events.plugins_changed();
    }

    /// Kernel code stopped half-way: the slot rebuilds the Agent.
    fn host_task_panicked(&self, message: &str) {
        self.kernel_faults.kernel_fault("plugin host", message);
    }
}

pub(crate) struct AgentHost {
    registries: AgentKernelRegistries,
    host: Arc<ModuleHost<AgentPlugins>>,
}

impl AgentHost {
    /// Derives every row's configuration and enablement from the Agent's
    /// settings, which the caller has loaded, and the host's network
    /// settings. `registries` are the ones the
    /// kernel was constructed with; the kernel plugin publishes them, and
    /// their `PluginFaults` is where the plugins' code reports.
    pub(crate) fn new(
        registries: AgentKernelRegistries,
        blocking: Arc<dyn BlockingOperationPort>,
        config: AgentConfig,
        network: Arc<dyn crate::NetworkSettingsPort>,
        events: Arc<dyn AgentPluginEventSink>,
        manifest: Vec<AgentPluginRow>,
        options: AgentHostOptions,
    ) -> Result<Self, String> {
        let fixed = [
            AgentPluginRow::unconfigured(
                AgentPluginGroup::Core,
                host_services_plugin(Arc::new(BlockingService::new(blocking))),
            ),
            AgentPluginRow::unconfigured(AgentPluginGroup::Core, kernel_plugin(registries.clone())),
        ];
        let host = ModuleHost::new(
            AgentPlugins {
                config,
                network,
                events,
                kernel_faults: options.kernel_faults,
            },
            registries.faults.clone(),
            fixed.into_iter().chain(manifest).collect(),
            ModuleHostOptions {
                safe_mode: options.safe_mode,
                // A plugin's faults reach the host through its
                // `PluginFaults`; the module fault sink names application
                // modules, never plugins. A kernel fault restarts the whole
                // Agent instead (its slot).
                module_faults: false,
                automatic_restarts: Some(options.restart_policy),
            },
        )?;
        Ok(Self {
            registries,
            host: Arc::new(host),
        })
    }

    /// The registries the kernel plans from; they outlive every plugin.
    pub(crate) fn registries(&self) -> &AgentKernelRegistries {
        &self.registries
    }

    /// Mounts the kernel and the enabled manifest rows once, and from then
    /// on stops a plugin whose code panics. A plugin that fails to start is
    /// logged and left out; the Agent runs without its contributions.
    pub(crate) async fn ensure_started(&self) {
        self.host.ensure_started().await;
    }

    /// Every plugin's state and contributions, in mount order: the host
    /// services, the kernel, then the manifest.
    pub(crate) fn status(&self) -> Vec<AgentPluginStatus> {
        // Read the registries before the plugin lock: the two are never held
        // together.
        let mut contributions = contributions::contributions_by_plugin(&self.registries);
        self.host
            .status()
            .into_iter()
            .map(|status| {
                let contributions = contributions.remove(&status.name).unwrap_or_default();
                AgentPluginStatus::of(status, contributions)
            })
            .collect()
    }

    /// Checks that `name` is a plugin whose enablement may become `enabled`.
    pub(crate) fn check_enablement(&self, name: &str, enabled: bool) -> Result<(), String> {
        self.host.check_enablement(name, enabled)
    }

    /// Brings every row up to date with the latest settings; see
    /// `ModuleHost::reconfigure`. Must run inside the Agent runtime.
    pub(crate) fn reconfigure(&self) -> Vec<(String, Settle)> {
        self.host.reconfigure()
    }

    /// Starts a `Failed` plugin again with its current configuration.
    pub(crate) async fn retry(&self, name: &str) -> Result<(), String> {
        self.host.retry(name).await
    }

    /// Stops every plugin, bounded by `deadline`; see `ModuleHost::shutdown`.
    pub(crate) async fn shutdown(&self, deadline: Instant) -> Vec<WorkerShutdownDiagnostic> {
        self.host.shutdown(deadline).await
    }
}

/// Logs how a configuration update ended.
pub(crate) async fn report_reconfigured(name: String, settle: Settle) {
    ModuleHost::<AgentPlugins>::report_reconfigured(name, settle).await;
}

/// Publishes the application services plugins may use. They are created with
/// the host and never replaced, so no plugin restart reaches them.
fn host_services_plugin(blocking: Arc<BlockingService>) -> PluginDef {
    PluginDef::new(move |ctx: Context, _| {
        let blocking = blocking.clone();
        async move {
            ctx.provide_arc(blocking)?;
            Ok(())
        }
    })
    .name("agent-host-services")
    .provides([BlockingService::NAME])
}
