//! The Agent slot: what the application holds in place of the Agent.
//!
//! The application is built and presented without the Agent, which composition
//! builds afterwards on the Agent runtime (`super::startup`). Everything that
//! reaches the Agent — its commands, Task Center, diagnostics, the settings
//! command, the Skill watcher — holds this slot and resolves it per call, so a
//! missing, failed or restarted Agent is never kept alive by a stale clone. A
//! caller that finds no running Agent gets [`AgentUnavailable`], whose message
//! is the one the interface shows.
//!
//! The slot is also the Agent's supervisor (`supervisor`): a kernel fault
//! stops the running Agent and restarts it with backoff, and the user can
//! restart it (normally, in safe mode, with the history index rebuilt), turn
//! it off and on again. Each start is one Agent instance with its own number
//! (`generation`), runtime and process owner group.

mod state;
mod supervisor;
mod task_activity;

use std::ops::Deref;
use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::panic::catch_panic;
use hanafubuki_agent_kernel::AgentQuarantinedSession;
use hanafubuki_agent_process::process_groups::ProcessOwnerGroup;
use parking_lot::{Condvar, Mutex};

use super::startup::AgentLauncher;
use super::{
    AgentApplicationService, AgentPluginState, AgentPluginStatus, SkillCatalogInvalidationPort,
};
pub use hanafubuki_agent_host::{AUTOMATIC_RESTART_DELAYS, AUTOMATIC_RESTART_WINDOW};
use hanafubuki_agent_runtime::{ContainedRuntime, RuntimeTaskFailure};
pub use state::{
    AgentAvailability, AgentDisabledReason, AgentStartOptions, AgentStartStage, AgentStatus,
    AgentUnavailable,
};
pub use supervisor::AgentRestartPolicy;
use supervisor::Supervisor;

/// The running Agent, as one call holds it.
#[derive(Clone)]
pub struct AgentHandle {
    service: Arc<AgentApplicationService>,
    runtime: ContainedRuntime,
    generation: u64,
}

impl AgentHandle {
    /// The number of the Agent this handle reaches (see [`AgentStatus`]).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// This Agent's own runtime: every call into it runs there.
    pub fn runtime(&self) -> &ContainedRuntime {
        &self.runtime
    }
}

impl Deref for AgentHandle {
    type Target = AgentApplicationService;

    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

/// Told whenever the slot's state changes; the interface then reads
/// `agent_status` again.
pub trait AgentStatusEventSink: Send + Sync {
    fn status_changed(&self);
}

pub struct NoopAgentStatusEventSink;

impl AgentStatusEventSink for NoopAgentStatusEventSink {
    fn status_changed(&self) {}
}

/// A kernel fault that stopped the Agent.
#[derive(Clone)]
struct FaultState {
    reason: String,
    at_unix_millis: u64,
    restarts_in_window: u32,
    next_restart_at_unix_millis: Option<u64>,
}

enum AgentState {
    Disabled(AgentDisabledReason),
    Starting,
    Running(AgentHandle),
    Faulted(FaultState),
    Stopping,
    Failed {
        stage: AgentStartStage,
        reason: String,
    },
    Stopped,
}

impl AgentState {
    fn availability(&self) -> AgentAvailability {
        match self {
            Self::Disabled(reason) => AgentAvailability::Disabled { reason: *reason },
            Self::Starting => AgentAvailability::Starting,
            Self::Running(_) => AgentAvailability::Running,
            Self::Faulted(fault) => AgentAvailability::Faulted {
                reason: fault.reason.clone(),
                at: fault.at_unix_millis,
                restarts_in_window: fault.restarts_in_window,
                next_restart_at: fault.next_restart_at_unix_millis,
            },
            Self::Stopping => AgentAvailability::Stopping,
            Self::Failed { stage, reason } => AgentAvailability::Failed {
                stage: *stage,
                reason: reason.clone(),
            },
            Self::Stopped => AgentAvailability::Stopped,
        }
    }
}

struct SlotState {
    state: AgentState,
    /// Whether the start of the current instance was claimed, so a shutdown
    /// waits only for one under way.
    start_begun: bool,
    /// Cancelled by [`AgentSlot::close`] (and a disable): the start under
    /// way stops at its next stage and tears down what it started.
    start_cancellation: Option<CancellationToken>,
    /// The runtime of the current Agent instance, from its start until a
    /// stop takes it ([`AgentSlot::take_runtime`]) to stop it last.
    runtime: Option<ContainedRuntime>,
    /// The owner group of the current instance's processes, swept when it
    /// stops.
    process_owner: Option<ProcessOwnerGroup>,
    /// Starts claimed so far; the current instance's number.
    generation: u64,
    /// How the current instance was started.
    options: AgentStartOptions,
    /// A kernel fault reported while the current instance was starting: the
    /// start fails instead of installing it.
    start_fault: Option<String>,
    quarantined: Vec<AgentQuarantinedSession>,
    /// What builds an instance, from the first start on.
    launcher: Option<Arc<AgentLauncher>>,
    supervisor: Supervisor,
}

struct SlotInner {
    state: Mutex<SlotState>,
    changed: Condvar,
    events: Arc<dyn AgentStatusEventSink>,
}

/// What shutdown found in the slot when it closed it.
pub enum AgentClosing {
    /// The running (or faulted, not yet stopped) Agent, for the Agent's
    /// shutdown stages.
    Running(AgentHandle),
    /// A start, or a restart's stop, under way did not settle within the
    /// wait; it is abandoned and can no longer install an Agent.
    Abandoned,
    /// No Agent runs or is being built; the slot's state when it closed.
    NotRunning(AgentAvailability),
}

/// A start the slot handed out: the instance's number, how it starts, and
/// the cancellation it observes.
pub(crate) struct StartClaim {
    pub(crate) generation: u64,
    pub(crate) options: AgentStartOptions,
    pub(crate) cancellation: CancellationToken,
}

#[derive(Clone)]
pub struct AgentSlot {
    inner: Arc<SlotInner>,
}

impl AgentSlot {
    /// A slot whose Agent will start (`disabled` is `None`) or stays off.
    pub fn new(
        disabled: Option<AgentDisabledReason>,
        events: Arc<dyn AgentStatusEventSink>,
    ) -> Self {
        Self::with_restart_policy(disabled, events, AgentRestartPolicy::default())
    }

    /// [`Self::new`] with the automatic restart policy `policy`.
    pub fn with_restart_policy(
        disabled: Option<AgentDisabledReason>,
        events: Arc<dyn AgentStatusEventSink>,
        policy: AgentRestartPolicy,
    ) -> Self {
        Self {
            inner: Arc::new(SlotInner {
                state: Mutex::new(SlotState {
                    state: disabled.map_or(AgentState::Starting, AgentState::Disabled),
                    start_begun: false,
                    start_cancellation: None,
                    runtime: None,
                    process_owner: None,
                    generation: 0,
                    options: AgentStartOptions::default(),
                    start_fault: None,
                    quarantined: Vec::new(),
                    launcher: None,
                    supervisor: Supervisor::new(policy),
                }),
                changed: Condvar::new(),
                events,
            }),
        }
    }

    /// The running Agent, or why there is none.
    pub fn agent(&self) -> Result<AgentHandle, AgentUnavailable> {
        let state = self.inner.state.lock();
        match &state.state {
            AgentState::Running(handle) => Ok(handle.clone()),
            other => Err(AgentUnavailable {
                availability: other.availability(),
            }),
        }
    }

    pub fn status(&self) -> AgentStatus {
        let (status, running) = {
            let state = self.inner.state.lock();
            let current = matches!(state.state, AgentState::Starting | AgentState::Running(_));
            let running = match &state.state {
                AgentState::Running(handle) => Some(handle.clone()),
                _ => None,
            };
            (
                AgentStatus {
                    availability: state.state.availability(),
                    generation: state.generation,
                    safe_mode: current && state.options.safe_mode,
                    degraded: false,
                    quarantined_sessions: state.quarantined.clone(),
                },
                running,
            )
        };
        // The plugin status is read outside the slot's lock: the host's lock
        // is never taken under it.
        let degraded = running.is_some_and(|agent| {
            catch_panic(|| agent.plugin_status())
                .unwrap_or_default()
                .iter()
                .any(|plugin| {
                    plugin.state == AgentPluginState::Failed
                        || (plugin.state == AgentPluginState::Disabled && plugin.error.is_some())
                        || plugin.warning.is_some()
                })
        });
        AgentStatus { degraded, ..status }
    }

    /// Claims the start of the current instance and returns its number, how
    /// it starts and the cancellation it observes; `None` when the Agent is
    /// not to start now or a start was already claimed.
    pub(crate) fn begin_start(&self) -> Option<StartClaim> {
        let mut state = self.inner.state.lock();
        if !matches!(state.state, AgentState::Starting) || state.start_begun {
            return None;
        }
        state.start_begun = true;
        state.start_fault = None;
        state.generation = state.generation.saturating_add(1);
        let cancellation = CancellationToken::default();
        state.start_cancellation = Some(cancellation.clone());
        Some(StartClaim {
            generation: state.generation,
            options: state.options,
            cancellation,
        })
    }

    /// Records the runtime and the process owner group the start built for
    /// the instance, so its stop stops and sweeps them. Refused, handing the
    /// runtime back, once the slot is closed.
    pub(crate) fn attach_runtime(
        &self,
        runtime: ContainedRuntime,
        process_owner: ProcessOwnerGroup,
    ) -> Result<(), ContainedRuntime> {
        let mut state = self.inner.state.lock();
        if matches!(state.state, AgentState::Stopped) {
            return Err(runtime);
        }
        state.runtime = Some(runtime);
        state.process_owner = Some(process_owner);
        Ok(())
    }

    /// Takes the current Agent instance's runtime, for its stop to stop it
    /// last of the Agent's stages. `None` when no start built one, or a stop
    /// took it.
    pub fn take_runtime(&self) -> Option<ContainedRuntime> {
        self.inner.state.lock().runtime.take()
    }

    /// Takes the current instance's process owner group, for its stop's
    /// sweep.
    pub(crate) fn take_process_owner(&self) -> Option<ProcessOwnerGroup> {
        self.inner.state.lock().process_owner.take()
    }

    pub(crate) fn launcher(&self) -> Option<Arc<AgentLauncher>> {
        self.inner.state.lock().launcher.clone()
    }

    /// Keeps what builds an instance, for every later start; the first one
    /// set stays.
    pub(crate) fn set_launcher(&self, launcher: Arc<AgentLauncher>) {
        self.inner.state.lock().launcher.get_or_insert(launcher);
    }

    /// Whether the slot was closed.
    #[cfg(test)]
    pub(crate) fn is_stopped(&self) -> bool {
        matches!(self.inner.state.lock().state, AgentState::Stopped)
    }

    pub(crate) fn set_quarantined(&self, quarantined: Vec<AgentQuarantinedSession>) {
        self.inner.state.lock().quarantined = quarantined;
        self.inner.events.status_changed();
    }

    /// Adds a session the history of instance `generation` refused while it
    /// ran; ignored once a later start was claimed (that start's recovery
    /// judges the session again) and for a session already listed.
    pub(crate) fn add_quarantined(&self, generation: u64, session: AgentQuarantinedSession) {
        {
            let mut state = self.inner.state.lock();
            if state.generation != generation
                || state
                    .quarantined
                    .iter()
                    .any(|listed| listed.session_id == session.session_id)
            {
                return;
            }
            state.quarantined.push(session);
        }
        self.inner.events.status_changed();
    }

    /// Installs the started instance `generation`, which runs on the
    /// attached runtime. Refused with why when the slot is no longer
    /// starting that instance (shutdown closed it, a disable cancelled it)
    /// or kernel code of the instance panicked while it started.
    pub(crate) fn run(
        &self,
        service: AgentApplicationService,
        generation: u64,
    ) -> Result<AgentHandle, String> {
        let handle = {
            let mut state = self.inner.state.lock();
            let Some(runtime) = state.runtime.clone() else {
                return Err("the start was cancelled".to_string());
            };
            if !matches!(state.state, AgentState::Starting) || state.generation != generation {
                return Err("the start was cancelled".to_string());
            }
            if let Some(fault) = state.start_fault.take() {
                return Err(format!(
                    "its kernel code panicked while it started: {fault}"
                ));
            }
            let handle = AgentHandle {
                service: Arc::new(service),
                runtime,
                generation,
            };
            state.state = AgentState::Running(handle.clone());
            handle
        };
        self.changed();
        Ok(handle)
    }

    /// Marks a start that failed at `stage`; ignored once the slot left
    /// `Starting`.
    pub(crate) fn fail(&self, stage: AgentStartStage, reason: String) {
        {
            let mut state = self.inner.state.lock();
            if !matches!(state.state, AgentState::Starting) {
                return;
            }
            state.state = AgentState::Failed { stage, reason };
        }
        self.changed();
    }

    /// Whether the Agent `generation` is still the running one.
    pub(crate) fn is_running(&self, generation: u64) -> bool {
        let state = self.inner.state.lock();
        matches!(&state.state, AgentState::Running(handle) if handle.generation == generation)
    }

    /// The error of a call on Agent `generation` that did not finish because
    /// that Agent no longer runs, or `None` while it does (or the
    /// application is shutting down, which the caller reports itself).
    pub fn interrupted(&self, generation: u64, command: &str) -> Option<String> {
        let state = self.inner.state.lock();
        match &state.state {
            AgentState::Running(handle) if handle.generation == generation => None,
            AgentState::Stopped => None,
            AgentState::Disabled(_) => Some(format!(
                "the Agent was turned off; {command} did not finish"
            )),
            AgentState::Faulted(FaultState {
                next_restart_at_unix_millis: None,
                ..
            }) => Some(format!(
                "the Agent stopped after an internal error; {command} did not finish"
            )),
            _ => Some(format!("the Agent restarted; {command} did not finish")),
        }
    }

    /// Closes the slot for shutdown: cancels a start under way, which stops
    /// at its next stage and tears down what it started, waits until
    /// `deadline` for it (and for a restart's stop under way) to settle,
    /// then marks the slot `Stopped` and returns what it held. Later calls
    /// find the Agent unavailable and nothing restarts. The instance's
    /// runtime stays in the slot for [`Self::take_runtime`].
    pub fn close(&self, deadline: Instant) -> AgentClosing {
        let closing = {
            let mut state = self.inner.state.lock();
            if let Some(cancellation) = state.start_cancellation.take() {
                cancellation.cancel();
            }
            state.supervisor.pending = None;
            while ((matches!(state.state, AgentState::Starting) && state.start_begun)
                || state.supervisor.stopping)
                && !self
                    .inner
                    .changed
                    .wait_until(&mut state, deadline)
                    .timed_out()
            {}
            let start_abandoned = state.start_begun;
            let stop_abandoned = state.supervisor.stopping;
            let retiring = state.supervisor.retiring.take();
            match (
                std::mem::replace(&mut state.state, AgentState::Stopped),
                retiring,
            ) {
                (AgentState::Running(handle), _) | (_, Some(handle)) => {
                    AgentClosing::Running(handle)
                }
                (AgentState::Starting, None) if start_abandoned => AgentClosing::Abandoned,
                (_, None) if stop_abandoned => AgentClosing::Abandoned,
                (previous, None) => AgentClosing::NotRunning(previous.availability()),
            }
        };
        self.changed();
        closing
    }

    /// Wakes every waiter on the slot and tells the interface.
    fn changed(&self) {
        self.inner.changed.notify_all();
        self.inner.events.status_changed();
    }
}

/// The settings command hands saved settings to whichever Agent runs.
impl AgentSlot {
    /// Schedules the running Agent's reconfiguration from the saved settings
    /// on its runtime, where kardia must run and a panic stays contained,
    /// and returns at once: a plugin restart or an MCP reconnect never holds
    /// the settings, which are saved either way. Two quick changes apply in
    /// order, since each step reads the latest settings under its own lock
    /// (`AgentApplicationService::apply_settings_to_plugins`). With no Agent
    /// running there is nothing to reconfigure: an Agent built later reads
    /// the settings then.
    pub fn apply_saved_settings(&self) {
        let agent = match self.agent() {
            Ok(agent) => agent,
            Err(unavailable) => {
                log::debug!("saved settings not applied to the Agent: {unavailable}");
                return;
            }
        };
        let handle = agent.runtime().handle().clone();
        let work = handle.spawn(async move {
            agent.apply_settings_to_plugins();
            agent.memory().signal_scan(None);
        });
        handle.spawn(async move {
            if let Err(error) = work.await {
                log::error!(
                    "applying the saved settings to the Agent did not complete: {}",
                    RuntimeTaskFailure::from(error)
                );
            }
        });
    }
}

/// Diagnostics reads the Agent from its own worker, so each read resolves
/// the slot and is guarded: with no Agent running there is nothing to read
/// (the bundle's `app/agent-status.json` says why), and a panic in a read is
/// that capture's issue, not the application's end.
impl AgentSlot {
    /// Flushes the running Agent's history, so the bundle copies a coherent
    /// store.
    pub fn flush_for_diagnostics(&self) -> Result<(), String> {
        let Ok(agent) = self.agent() else {
            return Ok(());
        };
        catch_panic(|| agent.flush_for_diagnostics())
            .unwrap_or_else(|message| Err(format!("Agent history flush panicked: {message}")))
    }

    pub fn plugin_status_for_diagnostics(&self) -> Vec<AgentPluginStatus> {
        let Ok(agent) = self.agent() else {
            return Vec::new();
        };
        catch_panic(|| agent.plugin_status()).unwrap_or_else(|message| {
            log::error!("reading the Agent plugin status panicked: {message}");
            Vec::new()
        })
    }
}

/// The Skill watcher reaches the catalog of whichever Agent runs; with none
/// running there is nothing to invalidate.
impl SkillCatalogInvalidationPort for AgentSlot {
    fn invalidate_root(&self, root: &std::path::Path) -> Result<(), String> {
        match self.agent() {
            Ok(agent) => SkillCatalogInvalidationPort::invalidate_root(&*agent, root),
            Err(_) => Ok(()),
        }
    }

    fn set_watcher_diagnostic(&self, message: Option<String>) {
        if let Ok(agent) = self.agent() {
            SkillCatalogInvalidationPort::set_watcher_diagnostic(&*agent, message);
        }
    }
}

#[cfg(test)]
mod tests;
