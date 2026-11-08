use std::collections::HashMap;
use std::time::Duration;

use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_foundation::environment::ChildProcessEnvironment;
use hanafubuki_agent_process::executor::ExecutorManager;

use super::{
    shutdown_agent, spawn_agent_start, AgentLaunch, AgentLauncher, AgentSlot, AgentStartMarker,
    AgentStatus, AGENT_START_SETTLE,
};
use crate::skill_watcher::SkillCatalogWatcher;

/// The Agent beside the desktop application: its slot, its crash-loop
/// marker and the environment of its processes.
/// Composition holds it next to `DesktopApplication`, which runs without it,
/// and starts it once the application is presented. The Agent's processes
/// are its own: its Executor, built at the start, is not the host's.
#[derive(Clone)]
pub struct DesktopAgent {
    slot: AgentSlot,
    marker: AgentStartMarker,
    child_environment: ChildProcessEnvironment,
}

impl DesktopAgent {
    /// An Agent whose slot [`Self::start`] fills, whose processes inherit
    /// `inherited_environment` in place of this process's environment, as
    /// the host's own do.
    pub fn new(
        slot: AgentSlot,
        marker: AgentStartMarker,
        inherited_environment: HashMap<String, Option<String>>,
    ) -> Result<Self, String> {
        let child_environment =
            ChildProcessEnvironment::with_inherited_overlay(inherited_environment)
                .map_err(|error| format!("invalid Agent child process environment: {error}"))?;
        Ok(Self {
            slot,
            marker,
            child_environment,
        })
    }

    /// Replaces the overlay applied last to every process the Agent starts
    /// from now on (the host's proxy), as the host does for its own.
    pub fn replace_child_environment(
        &self,
        entries: HashMap<String, Option<String>>,
    ) -> Result<(), String> {
        self.child_environment
            .replace(entries)
            .map_err(|error| format!("invalid Agent child process environment: {error}"))
    }

    #[cfg(test)]
    pub(crate) fn child_environment_for_test(&self) -> HashMap<String, Option<String>> {
        self.child_environment.snapshot()
    }

    /// The slot itself, for a holder that resolves the Agent per call.
    pub fn slot(&self) -> &AgentSlot {
        &self.slot
    }

    pub fn status(&self) -> AgentStatus {
        self.slot.status()
    }

    /// Hands the slot `launch`, which builds every Agent instance of the
    /// run, with the Agent's own Executor and Skill watcher, and starts the
    /// first (a slot that is off starts it when it is turned on); the
    /// watcher follows each instance that runs. Returns at once: the Agent instance gets its own runtime and the start
    /// runs there — construction and recovery on its blocking pool, the
    /// plugins as its tasks, the MCP servers in the background. A failure
    /// leaves the slot `Failed`.
    pub fn start(&self, launch: AgentLaunch) {
        spawn_agent_start(
            self.slot.clone(),
            AgentLauncher::new(
                launch,
                ExecutorManager::new(self.child_environment.clone()),
                SkillCatalogWatcher::idle(),
                self.marker.clone(),
                AGENT_START_SETTLE,
            ),
        );
    }

    /// Shuts the Agent down within `budget` (`AGENT_SHUTDOWN_BUDGET`): stops
    /// the Skill watcher, cancels a start under way, stops a running Agent's
    /// stages in order and sweeps its processes, then stops its runtime. Clears this run's start attempt: an orderly
    /// exit is not a crash. Blocks the caller, which is outside any async
    /// context.
    pub fn shutdown(&self, budget: Duration) -> Vec<WorkerShutdownDiagnostic> {
        shutdown_agent(&self.slot, &self.marker, budget)
    }
}
