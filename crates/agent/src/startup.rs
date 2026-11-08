//! Building and starting an Agent instance.
//!
//! Composition hands [`spawn_agent_start`] an [`AgentLaunch`] once the
//! application is ready; the slot keeps it, so the supervisor starts every
//! later instance (a restart, an automatic restart after a kernel fault, a
//! turn on) the same way. Each start claims the slot's next instance number,
//! builds the instance's runtime and runs there — leftover process groups of
//! earlier instances reaped, construction and recovery on its blocking pool,
//! plugins as its tasks, the MCP servers in the background — and leaves the
//! slot `Running` or `Failed`. A failure is logged at error and never ends
//! the application. Shutdown and a disable cancel a start under way
//! (`AgentSlot::close`): it stops at its next stage (construction and
//! recovery block and cannot be cancelled; the plugins' start is cancelled
//! where it stands) and tears down what it started, bounded, before its
//! runtime is stopped.
//!
//! [`AgentStartMarker`] detects a crash loop: every start records an attempt
//! before it constructs, and the attempt is cleared once the Agent has run for
//! [`AGENT_START_SETTLE`], when its start failed in an orderly way, when the
//! instance stopped in order (a restart, a turn off) or at an orderly
//! shutdown. A process that died while the Agent was starting or soon after
//! leaves its attempt behind, unless the run ended in a fatal panic, which is
//! never the Agent's (`hanafubuki_agent_foundation::panic::read_crash_record`,
//! of the record the process's hook writes); after [`CRASH_LOOP_STARTS`]
//! such starts the next one leaves the Agent off until
//! the user restarts it or turns its module off and on.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent_config::{write_atomic, FileAccess};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::panic::{read_crash_record, CrashRecord};
use hanafubuki_agent_history::{AgentHistoryRefusalSink, NoopAgentHistoryRefusalSink};
use hanafubuki_agent_kernel::{AgentKernelFaultSink, NoopAgentKernelFaultSink};
use hanafubuki_agent_process::executor::ExecutorManager;
use hanafubuki_agent_process::process_groups::{ProcessGroupRecord, ProcessOwnerGroup};
use serde::{Deserialize, Serialize};

use super::{
    AgentApplicationService, AgentDisabledReason, AgentHandle, AgentSlot, AgentStartOptions,
    AgentStartStage,
};
use crate::skill_watcher::SkillCatalogWatcher;
use hanafubuki_agent_runtime::{ContainedRuntime, RuntimeTaskFailure, AGENT_RUNTIME_THREAD_NAME};

/// Unfinished starts in a row after which the Agent starts off.
pub const CRASH_LOOP_STARTS: u32 = 3;
/// How long the Agent runs before its start counts as finished.
pub const AGENT_START_SETTLE: Duration = Duration::from_secs(60);

/// How long an Agent that will not run is given to stop, in all. Shorter
/// than the wait shutdown gives a start under way
/// (`super::shutdown::START_WAIT`), so a start that shutdown cancels has torn
/// down what it started before its runtime is stopped.
pub(super) const RETIRE_BOUND: Duration = Duration::from_secs(5);
/// How often a cancellable stage checks the start's cancellation.
const CANCELLATION_POLL: Duration = Duration::from_millis(20);

/// What one Agent instance is built with, besides what the constructor
/// captured: how it starts, where its kernel faults go and the Agent's
/// Executor for its processes.
#[derive(Clone)]
pub struct AgentInstanceInputs {
    pub options: AgentStartOptions,
    /// The slot's sink for this instance (`AgentSlot::fault_sink`).
    pub kernel_faults: Arc<dyn AgentKernelFaultSink>,
    /// Where the instance's history store tells a session it refuses once
    /// open (`AgentSlot::refusal_sink`), for the status's quarantine list.
    pub history_refusals: Arc<dyn AgentHistoryRefusalSink>,
    /// The Agent's own Executor, owned by the instance's process group:
    /// every process the instance starts belongs to it, so its stop sweeps
    /// what the instance's own owners left running.
    pub executor: ExecutorManager,
}

impl Default for AgentInstanceInputs {
    fn default() -> Self {
        Self {
            options: AgentStartOptions::default(),
            kernel_faults: Arc::new(NoopAgentKernelFaultSink),
            history_refusals: Arc::new(NoopAgentHistoryRefusalSink),
            executor: ExecutorManager::default().owned_by(ProcessOwnerGroup::new()),
        }
    }
}

/// Builds the Agent service of one instance on its runtime (the argument;
/// its blocking port is the Agent's blocking boundary); runs on that
/// runtime's blocking pool, once per start.
pub type AgentConstructor = Arc<
    dyn Fn(&ContainedRuntime, AgentInstanceInputs) -> Result<AgentApplicationService, String>
        + Send
        + Sync,
>;

/// What builds every Agent instance of the run.
#[derive(Clone)]
pub struct AgentLaunch {
    pub construct: AgentConstructor,
    /// The ledger of the Agent's process groups
    /// (`AgentStoragePaths::process_groups_file`); each start reaps what an
    /// earlier run left.
    pub process_groups_file: PathBuf,
}

/// The launch with the Agent's Executor, its Skill watcher, the crash-loop
/// marker and the settle time, as the slot keeps it.
pub(crate) struct AgentLauncher {
    launch: AgentLaunch,
    process_groups: ProcessGroupRecord,
    /// Every process of every instance; an instance's stop sweeps its own
    /// owner group.
    executor: ExecutorManager,
    skill_watcher: SkillCatalogWatcher,
    marker: AgentStartMarker,
    settle: Duration,
}

impl AgentLauncher {
    pub(crate) fn new(
        launch: AgentLaunch,
        executor: ExecutorManager,
        skill_watcher: SkillCatalogWatcher,
        marker: AgentStartMarker,
        settle: Duration,
    ) -> Self {
        Self {
            process_groups: ProcessGroupRecord::new(launch.process_groups_file.clone()),
            launch,
            executor,
            skill_watcher,
            marker,
            settle,
        }
    }

    pub(crate) fn marker(&self) -> &AgentStartMarker {
        &self.marker
    }

    pub(crate) fn executor(&self) -> &ExecutorManager {
        &self.executor
    }

    pub(crate) fn skill_watcher(&self) -> &SkillCatalogWatcher {
        &self.skill_watcher
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartMarkerDocument {
    unfinished_starts: u32,
    /// The run that recorded the last attempt, to pair it with a crash
    /// record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_attempt: Option<StartAttempt>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StartAttempt {
    pid: u32,
    at_unix_millis: u64,
}

/// The unfinished starts that count: the last one does not when its run
/// ended in a fatal panic, which is never on an Agent thread (they are
/// contained), so it was not the Agent's death.
fn counted_starts(document: &StartMarkerDocument, crash: Option<&CrashRecord>) -> u32 {
    match (&document.last_attempt, crash) {
        (Some(attempt), Some(crash))
            if crash.pid == attempt.pid && crash.at_unix_millis >= attempt.at_unix_millis =>
        {
            log::info!(
                "the last Agent start is not counted: its run ended in a fatal panic on thread \
                 '{}' at {}",
                crash.thread,
                crash.location
            );
            document.unfinished_starts.saturating_sub(1)
        }
        _ => document.unfinished_starts,
    }
}

/// The crash-loop start-up marker under the Agent data directory.
#[derive(Clone)]
pub struct AgentStartMarker {
    file: PathBuf,
    /// The crash record the panic hook writes (`configure_crash_record`).
    crash_record: Option<PathBuf>,
    /// Whether this run recorded an attempt that is not cleared yet.
    recorded: Arc<AtomicBool>,
}

impl AgentStartMarker {
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            crash_record: None,
            recorded: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The marker, discounting a start whose run ended in the fatal panic
    /// recorded at `crash_record`.
    pub fn with_crash_record(self, crash_record: PathBuf) -> Self {
        Self {
            crash_record: Some(crash_record),
            ..self
        }
    }

    /// Starts recorded and never cleared, less one whose run ended in a
    /// fatal panic. An unreadable marker counts none: it must not keep the
    /// Agent off.
    pub fn unfinished_starts(&self) -> u32 {
        let document = match std::fs::read(&self.file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(StartMarkerDocument::default())
            }
            Err(error) => Err(error.to_string()),
        };
        match document {
            Ok(document) => {
                let crash = self.crash_record.as_deref().and_then(read_crash_record);
                counted_starts(&document, crash.as_ref())
            }
            Err(error) => {
                log::warn!("the Agent start marker is unreadable and counts no starts: {error}");
                0
            }
        }
    }

    /// Whether the Agent starts this run, its module being enabled or not,
    /// and if not, why.
    pub fn disabled_reason(&self, module_enabled: bool) -> Option<AgentDisabledReason> {
        if !module_enabled {
            log::info!("the agent module is turned off in Settings; the Agent is not built");
            return Some(AgentDisabledReason::Setting);
        }
        let unfinished = self.unfinished_starts();
        if unfinished >= CRASH_LOOP_STARTS {
            log::warn!(
                "the Agent starts off: its last {unfinished} starts did not finish; \
                 restarting it or turning its module off and on clears this"
            );
            return Some(AgentDisabledReason::CrashLoop);
        }
        None
    }

    /// Records a start attempt before the Agent is constructed.
    pub(crate) fn record_attempt(&self) -> Result<(), String> {
        let unfinished = self.unfinished_starts().saturating_add(1);
        let at_unix_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        self.write(&StartMarkerDocument {
            unfinished_starts: unfinished,
            last_attempt: Some(StartAttempt {
                pid: std::process::id(),
                at_unix_millis,
            }),
        })?;
        self.recorded.store(true, Ordering::Release);
        Ok(())
    }

    /// Clears this run's attempt: the start finished, the instance stopped
    /// in order, or the application is leaving in order. Nothing to do when
    /// this run recorded none (the Agent was off), so a crash-loop stop
    /// survives an orderly exit.
    pub fn finish(&self) {
        if self.recorded.swap(false, Ordering::AcqRel) {
            if let Err(error) = self.write(&StartMarkerDocument::default()) {
                log::warn!("the Agent start marker was not cleared: {error}");
            }
        }
    }

    /// Forgets every recorded start: the user restarted the Agent or turned
    /// it on.
    pub fn reset(&self) -> Result<(), String> {
        self.recorded.store(false, Ordering::Release);
        self.write(&StartMarkerDocument::default())
    }

    #[cfg(test)]
    pub(crate) fn file_exists_for_test(&self) -> bool {
        self.file.exists()
    }

    #[cfg(test)]
    pub(crate) fn file_for_test(&self) -> &std::path::Path {
        &self.file
    }

    fn write(&self, document: &StartMarkerDocument) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(document).map_err(|error| error.to_string())?;
        write_atomic(&self.file, &bytes, FileAccess::Shared)
    }
}

/// Keeps `launcher` in `slot` and starts its first instance, once: a slot
/// that is off or already starting is left alone (a later turn on or
/// restart starts it with the kept launcher). Returns at once; the slot says
/// how the start went.
pub(crate) fn spawn_agent_start(slot: AgentSlot, launcher: AgentLauncher) {
    slot.set_launcher(Arc::new(launcher));
    launch_start(&slot);
}

/// Starts the slot's next instance with its launcher, if the slot is
/// starting and nothing claimed that start yet: the instance's runtime is
/// built and the start runs there as a task; once the instance runs, the
/// Skill watcher follows its roots.
pub(crate) fn launch_start(slot: &AgentSlot) {
    let Some(launcher) = slot.launcher() else {
        return;
    };
    let Some((runtime, claim)) = prepare_start(slot, &launcher) else {
        return;
    };
    let handle = runtime.handle().clone();
    let slot = slot.clone();
    handle.spawn(async move {
        let watching = slot.clone();
        let watcher = launcher.skill_watcher.clone();
        let roots = run_start(slot, runtime, launcher, claim)
            .await
            .and_then(|agent| agent.skill_discovery_roots());
        if let Some(roots) = roots {
            watcher.restart(&roots, Arc::new(watching));
        }
    });
}

/// [`spawn_agent_start`], awaited: returns the running Agent, or `None` when
/// there is none (the slot says why).
#[cfg(test)]
pub(crate) async fn start_agent(slot: AgentSlot, launcher: AgentLauncher) -> Option<AgentHandle> {
    slot.set_launcher(Arc::new(launcher));
    let launcher = slot.launcher()?;
    let (runtime, claim) = prepare_start(&slot, &launcher)?;
    run_start(slot, runtime, launcher, claim).await
}

/// What a start claimed: its instance's inputs and its cancellation.
struct Claim {
    generation: u64,
    inputs: AgentInstanceInputs,
    cancellation: CancellationToken,
}

/// Claims the slot's start and builds the Agent instance's runtime and
/// process owner group, which the slot keeps. `None` when there is nothing
/// to start.
fn prepare_start(slot: &AgentSlot, launcher: &AgentLauncher) -> Option<(ContainedRuntime, Claim)> {
    let claim = slot.begin_start()?;
    let runtime = match ContainedRuntime::start(AGENT_RUNTIME_THREAD_NAME) {
        Ok(runtime) => runtime,
        Err(reason) => {
            log::error!("the Agent did not start: {reason}");
            slot.fail(AgentStartStage::Construct, reason);
            return None;
        }
    };
    let process_owner = ProcessOwnerGroup::recorded_in(launcher.process_groups.clone());
    if let Err(runtime) = slot.attach_runtime(runtime.clone(), process_owner.clone()) {
        // Shutdown closed the slot first: nothing runs on this runtime yet.
        let _ = runtime.shutdown(Duration::ZERO);
        return None;
    }
    let inputs = AgentInstanceInputs {
        options: claim.options,
        kernel_faults: slot.fault_sink(claim.generation),
        history_refusals: slot.refusal_sink(claim.generation),
        executor: launcher.executor.owned_by(process_owner),
    };
    Some((
        runtime,
        Claim {
            generation: claim.generation,
            inputs,
            cancellation: claim.cancellation,
        },
    ))
}

/// Runs the start sequence as a task of the Agent instance's runtime. A
/// sequence that did not end (it panicked, or a stop stopped the runtime
/// under it) fails the start.
async fn run_start(
    slot: AgentSlot,
    runtime: ContainedRuntime,
    launcher: Arc<AgentLauncher>,
    claim: Claim,
) -> Option<AgentHandle> {
    let sequence = start_sequence(slot.clone(), runtime.clone(), launcher.clone(), claim);
    match runtime.run(sequence).await {
        Ok(started) => started,
        Err(failure) => {
            log::error!("the Agent start did not finish: {failure}");
            slot.fail(AgentStartStage::Start, failure.to_string());
            launcher.marker.finish();
            None
        }
    }
}

async fn start_sequence(
    slot: AgentSlot,
    runtime: ContainedRuntime,
    launcher: Arc<AgentLauncher>,
    claim: Claim,
) -> Option<AgentHandle> {
    let attempt = launcher.clone();
    let recorded = runtime
        .run_blocking(move || {
            let recorded = attempt.marker.record_attempt();
            reap_leftover_processes(&attempt.process_groups);
            recorded
        })
        .await;
    if let Err(error) = flatten(recorded) {
        log::warn!("the Agent start attempt was not recorded: {error}");
    }
    let generation = claim.generation;
    match start_stages(&slot, &runtime, &launcher, claim).await {
        Ok(handle) => {
            log::info!("the Agent is running generation={generation}");
            settle_after(slot, runtime, launcher, generation);
            Some(handle)
        }
        Err((stage, reason)) => {
            log::error!("the Agent did not start; its {stage} failed: {reason}");
            slot.fail(stage, reason);
            // An orderly failure: the process did not die with the Agent.
            launcher.marker.finish();
            None
        }
    }
}

/// Terminates the process groups an earlier Agent recorded and did not see
/// end (the application died under it); one whose leader cannot be
/// confirmed is left alone.
fn reap_leftover_processes(record: &ProcessGroupRecord) {
    let reap = record.reap_leftovers();
    if !reap.terminated.is_empty() || reap.unconfirmed > 0 {
        log::warn!(
            "leftover Agent processes from an earlier run: {} process group(s) terminated, {} \
             no longer confirmed and left alone",
            reap.terminated.len(),
            reap.unconfirmed
        );
    }
    for failure in reap.failed {
        log::warn!("a leftover Agent process group was not terminated: {failure}");
    }
}

async fn start_stages(
    slot: &AgentSlot,
    runtime: &ContainedRuntime,
    launcher: &AgentLauncher,
    claim: Claim,
) -> Result<AgentHandle, (AgentStartStage, String)> {
    let constructing = runtime.clone();
    let construct = launcher.launch.construct.clone();
    let inputs = claim.inputs;
    let service = flatten(
        runtime
            .run_blocking(move || construct(&constructing, inputs))
            .await,
    )
    .map_err(|reason| (AgentStartStage::Construct, reason))?;
    match start_built(
        slot,
        runtime,
        service.clone(),
        claim.generation,
        &claim.cancellation,
    )
    .await
    {
        Ok(handle) => Ok(handle),
        Err(failure) => {
            retire(service, runtime).await;
            Err(failure)
        }
    }
}

/// Recovers and starts a built Agent, then installs it in the slot. The
/// blocking stages (construction before this, recovery) cannot be cancelled,
/// so the start checks `cancellation` between stages; the plugins' start is
/// cancelled where it stands. The MCP servers connect in the background, so
/// the Agent runs without waiting for them; each server's status says how
/// its connection went.
async fn start_built(
    slot: &AgentSlot,
    runtime: &ContainedRuntime,
    service: AgentApplicationService,
    generation: u64,
    cancellation: &CancellationToken,
) -> Result<AgentHandle, (AgentStartStage, String)> {
    cancelled_before(cancellation, AgentStartStage::Recover)?;
    let recovering = service.clone();
    let report = flatten(runtime.run_blocking(move || recovering.recover()).await)
        .map_err(|reason| (AgentStartStage::Recover, reason))?;
    slot.set_quarantined(report.quarantined);
    cancelled_before(cancellation, AgentStartStage::Start)?;
    let starting = service.clone();
    let plugins = runtime
        .handle()
        .spawn(async move { starting.start_plugins().await });
    tokio::select! {
        started = plugins => started
            .map_err(|error| (AgentStartStage::Start, RuntimeTaskFailure::from(error).to_string()))?,
        () = cancelled(cancellation) => return Err(abandoned(AgentStartStage::Start)),
    }
    cancelled_before(cancellation, AgentStartStage::Start)?;
    service.spawn_mcp_start();
    slot.run(service, generation)
        .map_err(|reason| (AgentStartStage::Start, reason))
}

/// The failure of a start that shutdown or a disable cancelled before
/// `stage` ended.
fn abandoned(stage: AgentStartStage) -> (AgentStartStage, String) {
    (stage, "the start was cancelled".to_string())
}

fn cancelled_before(
    cancellation: &CancellationToken,
    stage: AgentStartStage,
) -> Result<(), (AgentStartStage, String)> {
    if cancellation.is_cancelled() {
        Err(abandoned(stage))
    } else {
        Ok(())
    }
}

/// Resolves once `cancellation` is cancelled.
async fn cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(CANCELLATION_POLL).await;
    }
}

/// Stops an Agent that was built but will not run (its recovery or start
/// failed, or shutdown cancelled or overtook it), in the order of the
/// application's shutdown stages and bounded by [`RETIRE_BOUND`] in all, so
/// its plugins, MCP connections, processes and workers do not outlive it.
async fn retire(service: AgentApplicationService, runtime: &ContainedRuntime) {
    let deadline = Instant::now() + RETIRE_BOUND;
    let stopping = service.clone();
    let mut diagnostics = runtime
        .run_blocking(move || stopping.stop_accepting_and_cancel_turns(deadline))
        .await
        .unwrap_or_default();
    if let Err(error) = service.shutdown_mcp(deadline).await {
        log::error!("an Agent that did not start kept its MCP connections: {error}");
    }
    diagnostics.extend(service.shutdown_plugins(deadline).await);
    diagnostics.extend(
        runtime
            .run_blocking(move || service.shutdown_workers_after_mcp(deadline))
            .await
            .unwrap_or_default(),
    );
    for diagnostic in diagnostics {
        log::warn!(
            "an Agent that did not start left worker {} running: {}",
            diagnostic.worker,
            diagnostic.message
        );
    }
}

/// Clears the start attempt once Agent `generation` has run for the
/// launcher's settle time ([`AGENT_START_SETTLE`] in the application).
fn settle_after(
    slot: AgentSlot,
    runtime: ContainedRuntime,
    launcher: Arc<AgentLauncher>,
    generation: u64,
) {
    let blocking = runtime.clone();
    runtime.handle().spawn(async move {
        tokio::time::sleep(launcher.settle).await;
        if slot.is_running(generation) {
            let _ = blocking
                .run_blocking(move || launcher.marker.finish())
                .await;
        }
    });
}

fn flatten<T>(result: Result<Result<T, String>, RuntimeTaskFailure>) -> Result<T, String> {
    result.unwrap_or_else(|failure| Err(failure.to_string()))
}

#[cfg(test)]
mod tests;
