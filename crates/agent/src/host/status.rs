//! What the host reports about each plugin it runs.

/// Why an Agent plugin cannot be turned off.
pub use hanafubuki_agent_host::ModuleLock as AgentPluginLock;
/// The coarse state of one Agent plugin: a row's state on the generic host.
pub use hanafubuki_agent_host::ModuleState as AgentPluginState;
use hanafubuki_agent_host::{ModuleGroup, ModuleRestart, ModuleStatus};
use serde::Serialize;

/// Where a plugin sits in the Agent's composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentPluginGroup {
    /// Tools and tool sources the model can call, with their rules.
    Capability,
    /// Sections of the world state the model reads at turn admission.
    Context,
    /// The host services and the kernel; never disabled.
    Core,
}

impl ModuleGroup for AgentPluginGroup {
    fn is_core(self) -> bool {
        self == Self::Core
    }
}

/// One context contributor and the section kinds it declares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPluginContextContribution {
    pub id: String,
    pub kinds: Vec<String>,
}

/// What a plugin has registered in the kernel registries now, in
/// registration order. Empty while it is not running.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPluginContributions {
    /// Registered tool names (`ToolRegistry`).
    pub tools: Vec<String>,
    /// Tool source ids (`ToolSourceRegistry`); their tools vary per step.
    pub tool_sources: Vec<String>,
    pub context: Vec<AgentPluginContextContribution>,
    pub turn_admission: Vec<String>,
    pub prompt: Vec<String>,
    pub approval_rules: Vec<String>,
    pub observers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPluginStatus {
    pub name: String,
    pub group: AgentPluginGroup,
    pub state: AgentPluginState,
    pub error: Option<String>,
    /// Set while the plugin is active but the kernel left part of it out of
    /// turns (degraded): a context or turn admission contributor that
    /// failed, with what it said. Cleared when it works again.
    pub warning: Option<String>,
    /// Whether the settings let it run; a core plugin always is.
    pub enabled: bool,
    /// Set when the plugin cannot be disabled.
    pub lock: Option<AgentPluginLock>,
    /// A pending automatic restart after its fault, or the ones the window
    /// counts.
    pub restart: Option<ModuleRestart>,
    pub contributions: AgentPluginContributions,
}

impl AgentPluginStatus {
    /// The host's status of a row, with what it contributes now.
    pub(super) fn of(
        status: ModuleStatus<AgentPluginGroup>,
        contributions: AgentPluginContributions,
    ) -> Self {
        let ModuleStatus {
            name,
            group,
            state,
            error,
            warning,
            enabled,
            lock,
            restart,
            // The Agent's plugin manager does not show them.
            dependents: _,
        } = status;
        Self {
            name,
            group,
            state,
            error,
            warning,
            enabled,
            lock,
            restart,
            contributions,
        }
    }
}
