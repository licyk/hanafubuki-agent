//! Stopping an Agent instance: one sequence under one budget.
//!
//! The application's shutdown hands the Agent [`AGENT_SHUTDOWN_BUDGET`] and
//! the Agent never holds the exit longer; a restart stops the old instance
//! with the same sequence and budget ([`stop_instance`]). The application's
//! shutdown stops the Skill watcher, then closes the slot, which cancels a
//! start under way and waits up to [`START_WAIT`] for it to tear down what
//! it started. A running Agent then stops in order — turns, MCP, plugins,
//! workers and history — each stage bounded by its own deadline and by what
//! is left of the budget. Then every process still in the instance's owner
//! group of the Agent's own Executor is terminated and disposed (the sweep:
//! what the instance's own owners did not stop). Last, the instance's
//! runtime is stopped: the hard stop that drops whatever that instance still
//! runs, including a call still in flight on it. A stage that does not end
//! by its deadline is reported and the next one runs.
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::WorkerShutdownDiagnostic;
use hanafubuki_agent_process::executor::manager::terminate_owner_group;

use super::{AgentClosing, AgentHandle, AgentSlot, AgentStartMarker, AGENT_RUNTIME_THREAD_NAME};

/// The most the Agent holds the application's exit.
pub const AGENT_SHUTDOWN_BUDGET: Duration = Duration::from_secs(20);
/// How long a start under way is given to settle once it is cancelled:
/// a construction or recovery that blocks finishes its stage, and the start
/// then retires what it built (`super::startup::RETIRE_BOUND`).
pub(crate) const START_WAIT: Duration = Duration::from_secs(8);
/// Turns and compactions are cancelled and their commits awaited.
const TURNS_BOUND: Duration = Duration::from_secs(5);
/// Background connects are cancelled, then every connection is closed and
/// its process confirmed gone.
const MCP_BOUND: Duration = Duration::from_secs(4);
/// The kardia root is disposed: registrations drain (`DRAIN_GRACE`), their
/// calls are cancelled (`RETIRE_CANCEL_BOUND`) and scoped workers stop.
const PLUGINS_BOUND: Duration = Duration::from_secs(5);
/// Memory workers, history writers, the Agent's worker threads and its
/// executions.
const WORKERS_BOUND: Duration = Duration::from_secs(3);
/// Each process left in the instance's owner group is given this to stop.
const SWEEP_BOUND: Duration = Duration::from_secs(1);
/// The Skill watcher's thread is given this to end.
const WATCHER_BOUND: Duration = Duration::from_secs(2);
/// The runtime's blocking pool is given this to finish.
const RUNTIME_BOUND: Duration = Duration::from_secs(2);

/// Shuts the Agent of `slot` down within `budget` and clears this run's
/// start attempt (an orderly exit is not a crash). Runs on the application's
/// shutdown thread, outside any async context. Returns what did not stop
/// cleanly.
pub(crate) fn shutdown_agent(
    slot: &AgentSlot,
    marker: &AgentStartMarker,
    budget: Duration,
) -> Vec<WorkerShutdownDiagnostic> {
    let total = Instant::now() + budget;
    let mut diagnostics = Vec::new();
    if let Some(launcher) = slot.launcher() {
        diagnostics.extend(
            launcher
                .skill_watcher()
                .shutdown((Instant::now() + WATCHER_BOUND).min(total)),
        );
    }
    let running = match slot.close((Instant::now() + START_WAIT).min(total)) {
        AgentClosing::Running(agent) => Some(agent),
        AgentClosing::Abandoned => {
            log::warn!(
                "the Agent was still starting or restarting when shutdown stopped waiting; its \
                 runtime is stopped with whatever it still runs"
            );
            diagnostics.push(diagnostic(
                "agent-start",
                "the Agent was still starting or restarting and was abandoned",
            ));
            None
        }
        AgentClosing::NotRunning(state) => {
            log::info!("no Agent to shut down: {state:?}");
            None
        }
    };
    let remaining = total.saturating_duration_since(Instant::now());
    diagnostics.extend(stop_instance(slot, running.as_ref(), remaining));
    marker.finish();
    diagnostics
}

/// Stops the current instance of `slot` within `budget`: the stages of the
/// running Agent `agent` (when there is one), the sweep of the instance's
/// owner group, then its runtime. What a stop already took (its runtime, its
/// owner group) is not there to stop again. Clears the instance's start
/// attempt: an orderly stop is not a crash. Blocks the caller, which is
/// outside any async context (the application's shutdown thread, the slot's
/// supervisor thread). Returns what did not stop cleanly.
pub(crate) fn stop_instance(
    slot: &AgentSlot,
    agent: Option<&AgentHandle>,
    budget: Duration,
) -> Vec<WorkerShutdownDiagnostic> {
    let total = Instant::now() + budget;
    let within = |bound: Duration| (Instant::now() + bound).min(total);
    let mut diagnostics = Vec::new();
    if let Some(agent) = agent {
        stop_running(agent, &within, &mut diagnostics);
    }
    let launcher = slot.launcher();
    if let (Some(owner), Some(launcher)) = (slot.take_process_owner(), &launcher) {
        let bound = SWEEP_BOUND.min(total.saturating_duration_since(Instant::now()));
        for failure in terminate_owner_group(launcher.executor(), &owner, bound) {
            diagnostics.push(diagnostic("agent-processes", failure));
        }
    }
    if let Some(launcher) = &launcher {
        launcher.marker().finish();
    }
    if let Some(runtime) = slot.take_runtime() {
        let remaining = total.saturating_duration_since(Instant::now());
        if let Err(message) = runtime.shutdown(RUNTIME_BOUND.min(remaining)) {
            diagnostics.push(diagnostic(AGENT_RUNTIME_THREAD_NAME, message));
        }
    }
    diagnostics
}

/// The running Agent's stages, in order: turns, MCP, plugins, workers.
fn stop_running(
    agent: &AgentHandle,
    within: &impl Fn(Duration) -> Instant,
    diagnostics: &mut Vec<WorkerShutdownDiagnostic>,
) {
    diagnostics.extend(agent.stop_accepting_and_cancel_turns(within(TURNS_BOUND)));

    let deadline = within(MCP_BOUND);
    let stopping = agent.clone();
    match agent.runtime().block_on_until(
        async move { stopping.shutdown_mcp(deadline).await },
        deadline,
    ) {
        Some(Ok(Ok(()))) => {}
        Some(Ok(Err(error))) => {
            log::error!("MCP shutdown failed and retained its cleanup obligation: {error}");
            diagnostics.push(diagnostic("agent-mcp", error));
        }
        Some(Err(failure)) => diagnostics.push(diagnostic(
            "agent-mcp",
            format!("MCP shutdown did not complete: {failure}"),
        )),
        None => diagnostics.push(diagnostic(
            "agent-mcp",
            "MCP shutdown did not end before its deadline",
        )),
    }

    let deadline = within(PLUGINS_BOUND);
    let stopping = agent.clone();
    match agent.runtime().block_on_until(
        async move { stopping.shutdown_plugins(deadline).await },
        deadline,
    ) {
        Some(Ok(plugins)) => diagnostics.extend(plugins),
        Some(Err(failure)) => diagnostics.push(diagnostic(
            "agent-plugins",
            format!("plugin shutdown did not complete: {failure}"),
        )),
        None => diagnostics.push(diagnostic(
            "agent-plugins",
            "Agent plugins did not stop before the shutdown deadline",
        )),
    }

    diagnostics.extend(agent.shutdown_workers_after_mcp(within(WORKERS_BOUND)));
}

fn diagnostic(worker: &str, message: impl Into<String>) -> WorkerShutdownDiagnostic {
    WorkerShutdownDiagnostic {
        worker: worker.to_string(),
        message: message.into(),
    }
}
