use super::*;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};

use hanafubuki_agent_execution::{
    AgentBackgroundTerminalStopOutcome, AgentExecutionId, AgentExecutionProcessPort,
};
use hanafubuki_agent_process::executor::{ExecutorInfo, ExecutorOutput, ExecutorStartRequest};
use hanafubuki_agent_process::ownership::{
    OwnedReleaseStatus, ProcessOwnerId, ProcessReceipt, ProcessReservation,
};

// A model wait that a confirmed stop takes away ends in `NotFound`, or in the
// terminal result the stop caused when the wait saw it first; never in a
// process or cleanup error. Each test pauses one side at a port call so the
// interleaving is fixed.

#[test]
#[cfg(unix)]
fn a_wait_holding_its_receipt_finds_a_stopped_terminal_gone() {
    let (service, scope, execution_id, gate) = start_paused_terminal(Pause::Info);
    gate.arm();
    let waiter = spawn_waiter(&service, &scope, execution_id);
    gate.wait_reached();
    let stopped = service.stop_background_terminals_confirmed(&scope);
    assert_eq!(stopped.disposed.len(), 1);
    assert!(stopped.retained.is_empty());
    gate.release();
    assert!(matches!(
        waiter.join().unwrap(),
        Err(AgentExecutionError::NotFound)
    ));
    assert_eq!(service.registry_count(), 0);
}

#[test]
#[cfg(unix)]
fn a_wait_that_sees_the_stop_terminate_first_ends_with_the_terminal_result() {
    let (service, scope, execution_id, gate) = start_paused_terminal(Pause::AfterTerminate);
    let waiter = spawn_waiter(&service, &scope, execution_id);
    gate.arm();
    let stop = spawn_stop(&service, &scope);
    gate.wait_reached();
    let result = waiter.join().unwrap().unwrap();
    assert_eq!(result.session_id, None);
    assert_eq!(result.exit_code, None);
    gate.release();
    let stopped = stop.join().unwrap();
    assert_eq!(stopped.disposed.len(), 1);
    assert!(stopped.retained.is_empty());
    assert_eq!(service.registry_count(), 0);
}

#[test]
#[cfg(unix)]
fn a_terminal_the_stop_is_disposing_is_gone_to_a_wait_and_disposed_once() {
    let (service, scope, execution_id, gate) = start_paused_terminal(Pause::BeforeDispose);
    gate.arm();
    let stop = spawn_stop(&service, &scope);
    gate.wait_reached();
    let waiter = spawn_waiter(&service, &scope, execution_id);
    assert!(matches!(
        waiter.join().unwrap(),
        Err(AgentExecutionError::NotFound)
    ));
    assert!(service
        .list_background_terminals(&scope)
        .unwrap()
        .is_empty());
    gate.release();
    let stopped = stop.join().unwrap();
    assert_eq!(stopped.disposed.len(), 1);
    assert!(stopped.retained.is_empty());
    let empty = service.stop_background_terminals_confirmed(&scope);
    assert!(empty.disposed.is_empty());
    assert!(empty.retained.is_empty());
}

#[test]
#[cfg(unix)]
fn background_terminal_listing_is_scope_and_kind_bound_with_bounded_recent_output() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(26).unwrap();
    let foreign = AgentExecutionScope::for_agent_session(27).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    process.set_output("agent-command-1", b"one\ntwo\nthree\nfour\n".to_vec());
    let ordinary = command_request();
    service
        .start(
            scope.clone(),
            ordinary.clone(),
            Some(&approved(&ordinary)),
            &CancellationToken::default(),
        )
        .unwrap();
    let terminals = service.list_background_terminals(&scope).unwrap();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0].execution_id, started.execution_id);
    assert_eq!(terminals[0].command, "printf unified");
    assert_eq!(terminals[0].recent_output, ["two", "three", "four"]);
    let encoded = serde_json::to_value(&terminals[0]).unwrap();
    assert!(encoded.get("executionId").is_some());
    assert!(encoded.get("createdAtMs").is_some());
    assert!(encoded.get("cleanupRequired").is_some());
    for forbidden in ["receipt", "owner", "executorId", "environment"] {
        assert!(!encoded.to_string().contains(forbidden));
    }
    assert!(service
        .list_background_terminals(&foreign)
        .unwrap()
        .is_empty());
    service.stop_background_terminals_confirmed(&scope);
}

#[test]
#[cfg(unix)]
fn background_stop_retains_failed_cleanup_and_retries_the_same_identity() {
    let process = FakeAgentExecutionProcessPort::shared();
    let service = AgentExecutionService::with_process_port(process.clone());
    let scope = AgentExecutionScope::for_agent_session(28).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    process.fail_next_terminations(1);
    let failed = service.stop_background_terminals_confirmed(&scope);
    assert!(failed.disposed.is_empty());
    assert_eq!(failed.retained.len(), 1);
    assert_eq!(failed.retained[0].execution_id, started.execution_id);
    assert_eq!(service.registry_count(), 1);
    let retried = service.stop_background_terminals_confirmed(&scope);
    assert_eq!(retried.disposed.len(), 1);
    assert!(retried.retained.is_empty());
    assert_eq!(service.registry_count(), 0);
}

#[derive(Clone, Copy, PartialEq)]
enum Pause {
    /// The first `info` after arming: a wait holding its receipt.
    Info,
    /// Right after the process is terminated, before its disposal.
    AfterTerminate,
    /// Inside the claimed disposal, before the process is disposed.
    BeforeDispose,
}

/// Holds the first thread to reach its pause once armed, until released.
struct Gate {
    armed: AtomicBool,
    reached: Mutex<mpsc::Sender<()>>,
    reached_rx: Mutex<mpsc::Receiver<()>>,
    release: Mutex<mpsc::Sender<()>>,
    release_rx: Mutex<mpsc::Receiver<()>>,
}

impl Gate {
    fn new() -> Self {
        let (reached, reached_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        Self {
            armed: AtomicBool::new(false),
            reached: Mutex::new(reached),
            reached_rx: Mutex::new(reached_rx),
            release: Mutex::new(release),
            release_rx: Mutex::new(release_rx),
        }
    }

    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn pass(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.reached.lock().unwrap().send(()).unwrap();
            self.release_rx.lock().unwrap().recv().unwrap();
        }
    }

    fn wait_reached(&self) {
        self.reached_rx.lock().unwrap().recv().unwrap();
    }

    fn release(&self) {
        self.release.lock().unwrap().send(()).unwrap();
    }
}

struct PausingPort {
    fake: Arc<FakeAgentExecutionProcessPort>,
    pause: Pause,
    gate: Arc<Gate>,
}

impl PausingPort {
    fn pause_at(&self, pause: Pause) {
        if self.pause == pause {
            self.gate.pass();
        }
    }
}

impl AgentExecutionProcessPort for PausingPort {
    fn reserve(
        &self,
        process_id: &str,
        owner: &ProcessOwnerId,
    ) -> Result<ProcessReservation, String> {
        self.fake.reserve(process_id, owner)
    }

    fn finalize_reservation(&self, reservation: &ProcessReservation) -> OwnedReleaseStatus {
        self.fake.finalize_reservation(reservation)
    }

    fn start_reserved(
        &self,
        request: ExecutorStartRequest,
        reservation: &ProcessReservation,
    ) -> Result<(ExecutorInfo, ProcessReceipt), String> {
        self.fake.start_reserved(request, reservation)
    }

    fn info(&self, receipt: &ProcessReceipt) -> Result<ExecutorInfo, String> {
        self.pause_at(Pause::Info);
        self.fake.info(receipt)
    }

    fn read(
        &self,
        receipt: &ProcessReceipt,
        from_offset: Option<u64>,
        max_bytes: Option<usize>,
    ) -> Result<ExecutorOutput, String> {
        self.fake.read(receipt, from_offset, max_bytes)
    }

    fn terminate(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        let terminated = self.fake.terminate(receipt);
        self.pause_at(Pause::AfterTerminate);
        terminated
    }

    fn dispose_confirmed(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        self.pause_at(Pause::BeforeDispose);
        self.fake.dispose_confirmed(receipt)
    }

    fn write(&self, receipt: &ProcessReceipt, bytes: Vec<u8>, close: bool) -> Result<(), String> {
        self.fake.write(receipt, bytes, close)
    }

    fn interrupt(&self, receipt: &ProcessReceipt) -> Result<(), String> {
        self.fake.interrupt(receipt)
    }

    fn resize(&self, receipt: &ProcessReceipt, cols: u16, rows: u16) -> Result<(), String> {
        self.fake.resize(receipt, cols, rows)
    }
}

fn start_paused_terminal(
    pause: Pause,
) -> (
    AgentExecutionService,
    AgentExecutionScope,
    AgentExecutionId,
    Arc<Gate>,
) {
    let gate = Arc::new(Gate::new());
    let service = AgentExecutionService::with_process_port(Arc::new(PausingPort {
        fake: FakeAgentExecutionProcessPort::shared(),
        pause,
        gate: gate.clone(),
    }));
    let scope = AgentExecutionScope::for_agent_session(32).unwrap();
    let request = exec_request(true);
    let action = service.prepare_exec_command_action(&request).unwrap();
    let started = service
        .start_exec_command(
            scope.clone(),
            request,
            Some(&approve_action(&action)),
            &CancellationToken::default(),
        )
        .unwrap();
    (service, scope, started.execution_id, gate)
}

fn spawn_waiter(
    service: &AgentExecutionService,
    scope: &AgentExecutionScope,
    execution_id: AgentExecutionId,
) -> std::thread::JoinHandle<Result<UnifiedExecResult, AgentExecutionError>> {
    let service = service.clone();
    let scope = scope.clone();
    std::thread::spawn(move || {
        service.wait_unified_execution(
            &scope,
            execution_id,
            5_000,
            Some(4096),
            &CancellationToken::default(),
        )
    })
}

fn spawn_stop(
    service: &AgentExecutionService,
    scope: &AgentExecutionScope,
) -> std::thread::JoinHandle<AgentBackgroundTerminalStopOutcome> {
    let service = service.clone();
    let scope = scope.clone();
    std::thread::spawn(move || service.stop_background_terminals_confirmed(&scope))
}
