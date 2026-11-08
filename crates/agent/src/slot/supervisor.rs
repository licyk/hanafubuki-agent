//! The Agent's supervisor: kernel faults, restarts, turning it off and on.
//!
//! A kernel fault — a panic a kernel-level catch caught: a turn's task, an
//! Agent command's guard, a member of the Agent's worker groups (history
//! writers, the memory supervisor, MCP stdio), the host's own tasks — means
//! kernel code stopped half-way, and the Agent's locks do not roll anything
//! back, so its in-memory state is not trusted. The instance reports it
//! through the sink its start was given ([`AgentSlot::fault_sink`]); the slot
//! marks the Agent `Faulted` at once (calls find it unavailable), stops the
//! instance and, while the restart window allows, starts a new one from disk
//! after a backoff ([`AgentRestartPolicy`]). A plugin's own fault is not one:
//! the plugin host stops that plugin, shows it `Failed` and starts it again
//! under the same policy (`PluginFaults`).
//!
//! Every transition runs on the slot's supervisor thread (`agent-supervisor`,
//! contained), never on the Agent runtime it stops nor on a caller: a stop
//! takes up to `AGENT_SHUTDOWN_BUDGET` and its last stage stops the
//! instance's runtime, which cannot be done from inside it. The thread
//! exists while there is work and ends when there is none.

use std::sync::{Arc, Weak};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent_foundation::panic::{catch_panic, mark_thread_contained};
use hanafubuki_agent_history::AgentHistoryRefusalSink;
use hanafubuki_agent_host::{RestartPolicy, RestartWindow};
use hanafubuki_agent_kernel::{AgentKernelFaultSink, AgentQuarantinedSession};

use super::{
    AgentDisabledReason, AgentHandle, AgentSlot, AgentStartOptions, AgentState, FaultState,
    SlotInner, SlotState,
};
use crate::shutdown::{stop_instance, AGENT_SHUTDOWN_BUDGET, START_WAIT};

/// When a kernel fault restarts the Agent on its own: the module host's
/// restart policy (`hanafubuki_agent_host::RestartPolicy`); once its
/// restarts are used up the Agent stays `Faulted` until the user restarts
/// it, which also empties the window.
pub type AgentRestartPolicy = RestartPolicy;

/// The supervisor's part of the slot's state.
pub(super) struct Supervisor {
    policy: AgentRestartPolicy,
    /// When the automatic restarts in the current window ran.
    restarts: RestartWindow,
    /// A faulted (or restarting) instance the thread is to stop.
    pub(super) retiring: Option<AgentHandle>,
    /// The next transition, once it is due.
    pub(super) pending: Option<Pending>,
    /// Whether the supervisor thread runs.
    thread: bool,
    /// Whether the thread is stopping an instance now.
    pub(super) stopping: bool,
    /// A turn off is pending or under way: a restart would undo it.
    turning_off: bool,
}

impl Supervisor {
    pub(super) fn new(policy: AgentRestartPolicy) -> Self {
        Self {
            policy,
            restarts: RestartWindow::default(),
            retiring: None,
            pending: None,
            thread: false,
            stopping: false,
            turning_off: false,
        }
    }
}

pub(super) struct Pending {
    due: Instant,
    transition: Transition,
}

#[derive(Clone, Copy)]
enum Transition {
    /// Stop what is left of the current instance and start a new one.
    Start {
        options: AgentStartOptions,
        /// The user asked: the crash-loop marker is reset first.
        by_user: bool,
    },
    /// Stop the Agent and leave it off (its module was turned off).
    Disable,
}

enum Job {
    /// Stop a faulted or restarting instance now, whatever comes next.
    Retire(AgentHandle),
    Apply(Transition),
}

/// The kernel fault sink of one Agent instance; a report from an instance
/// that is no longer the current one is stale and ignored.
struct SlotFaultSink {
    slot: Weak<SlotInner>,
    generation: u64,
}

impl AgentKernelFaultSink for SlotFaultSink {
    fn kernel_fault(&self, site: &str, message: &str) {
        if let Some(inner) = self.slot.upgrade() {
            AgentSlot { inner }.report_kernel_fault(self.generation, site, message);
        }
    }
}

/// The history refusal sink of one Agent instance; a report from an
/// instance that is no longer the current one is stale and ignored.
struct SlotRefusalSink {
    slot: Weak<SlotInner>,
    generation: u64,
}

impl AgentHistoryRefusalSink for SlotRefusalSink {
    fn session_refused(&self, session: AgentQuarantinedSession) {
        if let Some(inner) = self.slot.upgrade() {
            AgentSlot { inner }.add_quarantined(self.generation, session);
        }
    }
}

fn unix_millis(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

impl AgentSlot {
    /// Where the history store of instance `generation` reports a session
    /// it refuses once open.
    pub(crate) fn refusal_sink(&self, generation: u64) -> Arc<dyn AgentHistoryRefusalSink> {
        Arc::new(SlotRefusalSink {
            slot: Arc::downgrade(&self.inner),
            generation,
        })
    }

    /// Where instance `generation` reports its kernel faults.
    pub(crate) fn fault_sink(&self, generation: u64) -> Arc<dyn AgentKernelFaultSink> {
        Arc::new(SlotFaultSink {
            slot: Arc::downgrade(&self.inner),
            generation,
        })
    }

    /// Kernel code of instance `generation` panicked at `site`. A running
    /// instance becomes `Faulted` and is stopped; it is started again after
    /// the policy's next delay, unless the window's restarts are used up. An
    /// instance still starting fails its start instead. Returns at once: it
    /// is called on the thread that caught the panic.
    pub fn report_kernel_fault(&self, generation: u64, site: &str, message: &str) {
        let reason = format!("{site} panicked: {message}");
        {
            let mut state = self.inner.state.lock();
            let state = &mut *state;
            match &state.state {
                AgentState::Running(handle) if handle.generation == generation => {}
                AgentState::Starting if state.start_begun && state.generation == generation => {
                    log::error!("the Agent faulted while it started: {reason}");
                    state.start_fault.get_or_insert(reason);
                    return;
                }
                _ => {
                    log::warn!(
                        "a kernel fault of Agent {generation}, which no longer runs, is ignored: \
                         {reason}"
                    );
                    return;
                }
            }
            let AgentState::Running(handle) =
                std::mem::replace(&mut state.state, AgentState::Stopping)
            else {
                return;
            };
            let now = Instant::now();
            let supervisor = &mut state.supervisor;
            let (made, delay) = supervisor.restarts.next(&supervisor.policy, now);
            match delay {
                Some(delay) => log::error!(
                    "the Agent stopped after a kernel fault and restarts in {} ms: {reason}",
                    delay.as_millis()
                ),
                None => log::error!(
                    "the Agent stopped after a kernel fault and stays stopped: its {made} \
                     automatic restarts in the window are used up: {reason}"
                ),
            }
            supervisor.retiring = Some(handle);
            supervisor.pending = delay.map(|delay| Pending {
                due: now + delay,
                transition: Transition::Start {
                    options: AgentStartOptions {
                        rebuild_history_index: false,
                        ..state.options
                    },
                    by_user: false,
                },
            });
            state.state = AgentState::Faulted(FaultState {
                reason,
                at_unix_millis: unix_millis(SystemTime::now()),
                restarts_in_window: u32::try_from(made).unwrap_or(u32::MAX),
                next_restart_at_unix_millis: delay
                    .map(|delay| unix_millis(SystemTime::now() + delay)),
            });
            self.ensure_supervisor(state);
        }
        self.changed();
    }

    /// Stops the Agent (if it runs) and starts it again with `options`. The
    /// user's restart: the crash-loop marker and the automatic restart window
    /// are reset. Works while the Agent runs, is faulted, failed or off after
    /// a crash loop; refused while it starts, while the application shuts
    /// down and while its module is off or being turned off. Returns once
    /// the restart is scheduled; the status follows it.
    pub fn restart(&self, options: AgentStartOptions) -> Result<(), String> {
        {
            let mut state = self.inner.state.lock();
            let state = &mut *state;
            if state.launcher.is_none() {
                return Err("the Agent cannot start before the application is ready".to_string());
            }
            match &state.state {
                AgentState::Stopped => {
                    return Err("the application is shutting down".to_string());
                }
                AgentState::Starting => {
                    return Err("the Agent is starting; restart it once it runs".to_string());
                }
                AgentState::Disabled(AgentDisabledReason::Setting) => {
                    return Err("the Agent is turned off in Settings".to_string());
                }
                _ if state.supervisor.turning_off => {
                    return Err("the Agent is being turned off".to_string());
                }
                AgentState::Running(_)
                | AgentState::Faulted(_)
                | AgentState::Stopping
                | AgentState::Failed { .. }
                | AgentState::Disabled(AgentDisabledReason::CrashLoop) => {}
            }
            if let AgentState::Running(handle) =
                std::mem::replace(&mut state.state, AgentState::Stopping)
            {
                state.supervisor.retiring = Some(handle);
            }
            state.supervisor.restarts.clear();
            state.supervisor.pending = Some(Pending {
                due: Instant::now(),
                transition: Transition::Start {
                    options,
                    by_user: true,
                },
            });
            log::info!("the Agent restarts on request {options:?}");
            self.ensure_supervisor(state);
        }
        self.changed();
        Ok(())
    }

    /// Starts an Agent its module turned off (the module mounted again): the
    /// crash-loop marker is reset first. Nothing to do when it is not off,
    /// or off after a crash loop, which only the user's restart, or turning
    /// the module off and on, ends.
    pub fn enable(&self) -> Result<(), String> {
        {
            let mut state = self.inner.state.lock();
            let state = &mut *state;
            match &state.state {
                AgentState::Disabled(AgentDisabledReason::Setting) => {}
                AgentState::Stopped => return Err("the application is shutting down".to_string()),
                _ if state.supervisor.turning_off => {}
                _ => return Ok(()),
            }
            if state.launcher.is_none() {
                // Composition starts the Agent once the application is
                // ready, and reads the setting then.
                state.state = AgentState::Starting;
            } else {
                if matches!(state.state, AgentState::Disabled(_)) {
                    state.state = AgentState::Stopping;
                }
                state.supervisor.restarts.clear();
                state.supervisor.turning_off = false;
                state.supervisor.pending = Some(Pending {
                    due: Instant::now(),
                    transition: Transition::Start {
                        options: AgentStartOptions::default(),
                        by_user: true,
                    },
                });
                log::info!("the Agent is turned on");
                self.ensure_supervisor(state);
            }
        }
        self.changed();
        Ok(())
    }

    /// Stops the Agent in order and leaves it off (its module was turned
    /// off): a start under way is cancelled first. The state is then
    /// `Disabled(Setting)`.
    pub fn disable(&self) -> Result<(), String> {
        {
            let mut state = self.inner.state.lock();
            let state = &mut *state;
            let idle = state.supervisor.pending.is_none();
            match &state.state {
                AgentState::Stopped => return Err("the application is shutting down".to_string()),
                AgentState::Disabled(_) if idle => {
                    state.state = AgentState::Disabled(AgentDisabledReason::Setting);
                }
                AgentState::Starting if !state.start_begun && idle => {
                    state.state = AgentState::Disabled(AgentDisabledReason::Setting);
                }
                _ => {
                    if let Some(cancellation) = &state.start_cancellation {
                        cancellation.cancel();
                    }
                    match std::mem::replace(&mut state.state, AgentState::Stopping) {
                        AgentState::Running(handle) => state.supervisor.retiring = Some(handle),
                        // A start under way settles before the stop.
                        AgentState::Starting => state.state = AgentState::Starting,
                        _ => {}
                    }
                    state.supervisor.turning_off = true;
                    state.supervisor.pending = Some(Pending {
                        due: Instant::now(),
                        transition: Transition::Disable,
                    });
                    log::info!("the Agent is turned off");
                    self.ensure_supervisor(state);
                }
            }
        }
        self.changed();
        Ok(())
    }

    /// The `agent` module's stop: turns the Agent off ([`Self::disable`])
    /// and waits until `deadline` for it to be off. A stop still under way
    /// then goes on on the supervisor thread, and a later turn on starts the
    /// Agent after it. Nothing to do once shutdown closed the slot: it
    /// stopped the Agent first.
    pub fn turn_off(&self, deadline: Instant) -> Result<(), String> {
        if self.disable().is_err() {
            return Ok(());
        }
        let off = |state: &SlotState| {
            matches!(state.state, AgentState::Disabled(_) | AgentState::Stopped)
                && !state.supervisor.turning_off
        };
        let mut state = self.inner.state.lock();
        while !off(&state) {
            if self
                .inner
                .changed
                .wait_until(&mut state, deadline)
                .timed_out()
                && !off(&state)
            {
                return Err("the Agent is still stopping and finishes on its own".to_string());
            }
        }
        Ok(())
    }

    /// Starts the supervisor thread unless it runs; it takes the work from
    /// the state.
    fn ensure_supervisor(&self, state: &mut SlotState) {
        if state.supervisor.thread {
            return;
        }
        let slot = self.clone();
        match std::thread::Builder::new()
            .name("agent-supervisor".to_string())
            .spawn(move || {
                // Its stops call into the Agent, whose code survives a panic.
                mark_thread_contained();
                slot.supervise();
            }) {
            Ok(_) => state.supervisor.thread = true,
            Err(error) => {
                log::error!("the Agent supervisor did not start: {error}");
                state.supervisor.pending = None;
            }
        }
    }

    fn supervise(&self) {
        while let Some(job) = self.next_job() {
            if let Err(message) = catch_panic(|| self.run_job(job)) {
                log::error!("an Agent supervisor step panicked: {message}");
            }
            self.inner.state.lock().supervisor.stopping = false;
            self.changed();
        }
    }

    /// Waits for the next job: a stop now, or the pending transition once
    /// due. `None` ends the thread (nothing left, or the slot closed).
    fn next_job(&self) -> Option<Job> {
        let mut state = self.inner.state.lock();
        loop {
            if matches!(state.state, AgentState::Stopped) {
                state.supervisor.pending = None;
                state.supervisor.thread = false;
                return None;
            }
            if let Some(handle) = state.supervisor.retiring.take() {
                state.supervisor.stopping = true;
                return Some(Job::Retire(handle));
            }
            let Some(due) = state.supervisor.pending.as_ref().map(|pending| pending.due) else {
                state.supervisor.thread = false;
                return None;
            };
            if Instant::now() >= due {
                let pending = state.supervisor.pending.take()?;
                state.supervisor.stopping = true;
                return Some(Job::Apply(pending.transition));
            }
            let _ = self.inner.changed.wait_until(&mut state, due);
        }
    }

    fn run_job(&self, job: Job) {
        match job {
            Job::Retire(handle) => self.stop_current(Some(&handle)),
            Job::Apply(Transition::Start { options, by_user }) => {
                let running = self.take_running();
                self.stop_current(running.as_ref());
                if by_user {
                    if let Some(launcher) = self.launcher() {
                        if let Err(error) = launcher.marker().reset() {
                            log::warn!("the Agent crash-loop marker was not reset: {error}");
                        }
                    }
                }
                if self.reopen(options, !by_user) {
                    crate::startup::launch_start(self);
                }
            }
            Job::Apply(Transition::Disable) => {
                // A start that began after the disable was asked for (a
                // restart's) is cancelled too.
                if let Some(cancellation) = &self.inner.state.lock().start_cancellation {
                    cancellation.cancel();
                }
                self.wait_for_start(Instant::now() + START_WAIT);
                let running = self.take_running();
                self.stop_current(running.as_ref());
                let mut state = self.inner.state.lock();
                // A turn on since then overrode this turn off: it starts next.
                if state.supervisor.turning_off && !matches!(state.state, AgentState::Stopped) {
                    state.supervisor.turning_off = false;
                    state.state = AgentState::Disabled(AgentDisabledReason::Setting);
                }
            }
        }
    }

    /// Stops what is left of the current instance — the Agent `agent`, its
    /// processes, its runtime — within the Agent's shutdown budget.
    fn stop_current(&self, agent: Option<&AgentHandle>) {
        for diagnostic in stop_instance(self, agent, AGENT_SHUTDOWN_BUDGET) {
            log::warn!(
                "the stopped Agent left {} unfinished: {}",
                diagnostic.worker,
                diagnostic.message
            );
        }
    }

    /// The running Agent, which is stopping from now on.
    fn take_running(&self) -> Option<AgentHandle> {
        let mut state = self.inner.state.lock();
        if !matches!(state.state, AgentState::Running(_)) {
            return None;
        }
        match std::mem::replace(&mut state.state, AgentState::Stopping) {
            AgentState::Running(handle) => Some(handle),
            _ => None,
        }
    }

    /// Opens the slot for the next instance's start with `options`;
    /// `automatic` counts it in the restart window. `false` once the slot
    /// closed.
    fn reopen(&self, options: AgentStartOptions, automatic: bool) -> bool {
        {
            let mut state = self.inner.state.lock();
            if matches!(state.state, AgentState::Stopped) {
                return false;
            }
            if automatic {
                state.supervisor.restarts.record(Instant::now());
            }
            state.state = AgentState::Starting;
            state.start_begun = false;
            state.start_cancellation = None;
            state.options = options;
            state.quarantined.clear();
        }
        self.changed();
        true
    }

    /// Waits until `deadline` for a start under way to settle.
    fn wait_for_start(&self, deadline: Instant) {
        let mut state = self.inner.state.lock();
        while matches!(state.state, AgentState::Starting)
            && state.start_begun
            && !self
                .inner
                .changed
                .wait_until(&mut state, deadline)
                .timed_out()
        {}
    }
}
