//! What a module host reports about each row it runs.

use std::fmt;

use kardia_core::{Fiber, FiberState};
use serde::Serialize;

use crate::PluginError;

/// The coarse state of one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModuleState {
    /// Not started yet, waiting for a dependency, or (re)starting.
    Pending,
    Active,
    /// Its last start failed, it faulted, or its teardown is stuck; a retry
    /// or a configuration change starts it again.
    Failed,
    /// Turned off in the settings: not mounted.
    Disabled,
    /// Stopped by shutdown.
    Disposed,
}

/// Why a row cannot be turned off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModuleLock {
    /// Its host does not run without it.
    Core,
    /// The application does not run without it, but it fails alone and
    /// restarts: a fault stops it until it is retried.
    Platform,
}

/// Where a row sits in its host's composition. Each host has its own
/// groups (the Agent's `AgentPluginGroup`); the host only asks which rows
/// are core and which cannot be turned off.
pub trait ModuleGroup: Copy + Eq + fmt::Debug + Send + Sync + 'static {
    /// A core row is always enabled (also in safe mode), cannot be disabled,
    /// and stays mounted, only logged, when it faults or does not start in
    /// time: the host's other rows do not run without it.
    fn is_core(self) -> bool;

    /// Why a row of this group cannot be turned off, if it cannot: such a
    /// row is always enabled (also in safe mode). A core row is locked; a
    /// locked row that is not core still stops on a fault until retried.
    fn lock(self) -> Option<ModuleLock> {
        self.is_core().then_some(ModuleLock::Core)
    }
}

/// A row's automatic restarts, on a host that restarts a faulted row by
/// itself (`ModuleHostOptions::automatic_restarts`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleRestart {
    /// Automatic restarts counted in the current window.
    pub restarts_in_window: u32,
    /// When the next automatic restart runs (Unix milliseconds); `None`
    /// when none is pending, which for a `Failed` row means the window's
    /// restarts are used up and only a retry starts it.
    pub next_attempt_at_unix_millis: Option<u64>,
}

/// One row's state as its host sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleStatus<G> {
    pub name: String,
    pub group: G,
    pub state: ModuleState,
    pub error: Option<String>,
    /// Set while the row is active but part of it was left out (degraded):
    /// its current issues (`PluginFaults::set_issue`), joined. Cleared when
    /// they are.
    pub warning: Option<String>,
    /// Whether the settings let it run; a core row always is.
    pub enabled: bool,
    /// Set when the row cannot be disabled.
    pub lock: Option<ModuleLock>,
    /// Set while an automatic restart is pending or the window counts one.
    pub restart: Option<ModuleRestart>,
    /// The rows that are not running while this one is not, in mount order:
    /// those that inject a service it provides, and theirs in turn. They
    /// wait for it and mount again with it.
    pub dependents: Vec<String>,
}

/// The state and error of a mounted fiber.
pub(crate) fn fiber_state(fiber: &Fiber) -> (ModuleState, Option<String>) {
    let state = match fiber.state() {
        FiberState::Active => ModuleState::Active,
        FiberState::Failed => ModuleState::Failed,
        FiberState::Disposed => ModuleState::Disposed,
        // A retired fiber that is still unloading is on its way out; any
        // other transition ends in a (re)started plugin.
        FiberState::Unloading if fiber.is_removed() => ModuleState::Disposed,
        FiberState::Pending | FiberState::Loading | FiberState::Unloading => ModuleState::Pending,
    };
    let error = fiber
        .error()
        .map(|error| PluginError::from(error).to_string());
    (state, error)
}
