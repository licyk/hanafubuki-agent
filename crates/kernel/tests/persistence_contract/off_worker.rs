//! The turn loop, its admission included, queues its history appends and
//! awaits their acknowledgement: its async worker stays free while an
//! append is parked, and the staged commit settles where the store
//! acknowledges it, so a failed or dropped append rolls the record back.

use std::time::Instant;

use hanafubuki_agent_approval::ApprovalService;
use hanafubuki_agent_kernel::AgentTurnPhase;

use super::*;

pub(crate) fn service_over(store: Arc<RecordingHistoryStore>) -> (AgentService, AgentSessionId) {
    let service = AgentService::without_default_model_with_history_store_and_approval(
        AgentKernelRegistries::default(),
        store,
        ApprovalService::manual(),
    );
    let session = service
        .create_model_session(
            text_session_config(),
            hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_kernel::ModelRetryPolicy::default(),
            ModelCompactionCapability::LocalSummary,
            hanafubuki_agent_kernel::AgentSessionWorkspace::default(),
        )
        .unwrap();
    (service, session.id)
}

/// Runs a turn on its own single-threaded runtime and thread, so a session
/// left waiting for a commit fails the test instead of hanging it.
fn turn_on_its_own_thread(
    store: Arc<RecordingHistoryStore>,
) -> (
    Result<(), hanafubuki_agent_kernel::AgentError>,
    hanafubuki_agent_kernel::AgentSessionRuntimeSnapshot,
) {
    let (finished, outcome) = mpsc::channel();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (service, session_id) = service_over(store);
        let result = runtime.block_on(service.start_turn(session_id, "hello"));
        let _ = finished.send((result.map(|_| ()), service.snapshot(session_id).unwrap()));
    });
    outcome
        .recv_timeout(Duration::from_secs(5))
        .expect("the session was left waiting for a commit")
}

#[tokio::test(flavor = "current_thread")]
async fn a_parked_history_append_leaves_the_turn_runtime_free() {
    let store = Arc::new(RecordingHistoryStore::default());
    let (service, session_id) = service_over(store.clone());
    // The admission is a queued checkpoint; the first queued delta is the
    // sampling step's.
    let gate = Arc::new(AppendGate::new());
    *store.delta_gate.lock().unwrap() = Some(gate.clone());
    let parked = Arc::new(AtomicBool::new(false));
    let ran_while_parked = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (gate, parked, ran_while_parked) =
            (gate.clone(), parked.clone(), ran_while_parked.clone());
        thread::spawn(move || {
            gate.started.wait();
            parked.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(2);
            while !ran_while_parked.load(Ordering::SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(2));
            }
            let free = ran_while_parked.load(Ordering::SeqCst);
            gate.release.wait();
            free
        })
    };

    let turn = tokio::spawn({
        let service = service.clone();
        async move { service.start_turn(session_id, "hello").await }
    });
    // This single-threaded runtime can only get here while the append is
    // parked if the turn is not blocking it.
    while !parked.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    ran_while_parked.store(true, Ordering::SeqCst);
    let free = tokio::task::spawn_blocking(move || watcher.join().unwrap())
        .await
        .unwrap();
    turn.abort();

    assert!(
        free,
        "the turn blocked its runtime while the append was parked"
    );
    assert_eq!(store.deltas.lock().unwrap().len(), 1);
}

#[test]
fn a_failed_queued_append_rolls_the_staged_commit_back() {
    let store = Arc::new(RecordingHistoryStore::default());
    store.fail_next_queued_delta.store(true, Ordering::SeqCst);

    let (result, snapshot) = turn_on_its_own_thread(store.clone());

    assert!(
        matches!(&result, Err(hanafubuki_agent_kernel::AgentError::Persistence(message)) if message.contains("injected queued append failure")),
        "unexpected turn result: {result:?}"
    );
    let turn = snapshot.turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(turn.sampling_step, 0);
    assert!(store.deltas.lock().unwrap().is_empty());
}

#[test]
fn an_unacknowledged_append_rolls_the_staged_commit_back() {
    let store = Arc::new(RecordingHistoryStore::default());
    store.drop_next_queued_delta.store(true, Ordering::SeqCst);

    let (result, snapshot) = turn_on_its_own_thread(store.clone());

    assert!(
        matches!(&result, Err(hanafubuki_agent_kernel::AgentError::Persistence(message)) if message.contains("shutting down")),
        "unexpected turn result: {result:?}"
    );
    let turn = snapshot.turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(turn.sampling_step, 0);
}

/// Records whether a turn's start was emitted.
#[derive(Default)]
struct StartedSink(AtomicBool);

impl AgentEventSink for StartedSink {
    fn emit(&self, event: AgentEvent) {
        if matches!(event.kind, AgentEventKind::TurnStarted { .. }) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
}

/// `start_turn` stages its admission under the session's lock and awaits
/// the append with the lock released: while the append is parked the
/// runtime's only thread runs other work and other callers take the
/// session's lock, and the start is emitted only once it is durable.
#[test]
fn a_parked_admission_leaves_the_runtime_and_the_session_free() {
    let store = Arc::new(RecordingHistoryStore::default());
    let (service, session_id) = service_over(store.clone());
    let gate = Arc::new(AppendGate::new());
    *store.commit_gate.lock().unwrap() = Some(gate.clone());
    let sink = Arc::new(StartedSink::default());

    let (ran_while_parked, started_while_parked, started) = on_a_current_thread_runtime({
        let (service, sink) = (service.clone(), sink.clone());
        move || async move {
            let turn = tokio::spawn({
                let (service, sink) = (service.clone(), sink.clone());
                async move {
                    service
                        .start_turn_with_sink(session_id, "hello", sink)
                        .await
                }
            });
            meet(|gate| &gate.started, &gate).await;
            // This task runs on the runtime's only thread while the
            // admission is parked, and takes the session's lock (callers
            // that wait for the staged commit to settle still wait).
            let ran_while_parked = matches!(
                service.cancel_compaction(
                    session_id,
                    hanafubuki_agent_kernel::AgentCompactionId::new(1),
                    hanafubuki_agent_kernel::AgentCompactionGeneration::new(1),
                ),
                Err(hanafubuki_agent_kernel::AgentError::CompactionNotRunning)
            );
            let started_while_parked = sink.0.load(Ordering::SeqCst);
            meet(|gate| &gate.release, &gate).await;
            assert!(until(|| sink.0.load(Ordering::SeqCst)).await);
            turn.abort();
            let _ = turn.await;
            (
                ran_while_parked,
                started_while_parked,
                sink.0.load(Ordering::SeqCst),
            )
        }
    });

    assert!(ran_while_parked);
    assert!(
        !started_while_parked,
        "the start was emitted before it was durable"
    );
    assert!(started);
    let admitted = store.commits.lock().unwrap()[1].clone();
    assert_eq!(
        admitted.runtime.turn.map(|turn| turn.phase),
        Some(AgentTurnPhase::Sampling)
    );
}

/// A `start_turn` dropped while its admission is appended: the admission
/// still settles where the store acknowledges it, and the turn it admitted
/// is cancelled after it, so no admitted turn is left without a driver.
#[test]
fn a_start_dropped_during_its_admission_cancels_the_admitted_turn() {
    let store = Arc::new(RecordingHistoryStore::default());
    let (service, session_id) = service_over(store.clone());
    let gate = Arc::new(AppendGate::new());
    *store.commit_gate.lock().unwrap() = Some(gate.clone());

    let snapshot = on_a_current_thread_runtime({
        let service = service.clone();
        move || async move {
            let turn = tokio::spawn({
                let service = service.clone();
                async move { service.start_turn(session_id, "hello").await }
            });
            meet(|gate| &gate.started, &gate).await;
            turn.abort();
            assert!(turn.await.unwrap_err().is_cancelled());
            meet(|gate| &gate.release, &gate).await;
            service.snapshot(session_id).unwrap()
        }
    });

    let turn = snapshot.turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Cancelled);
    assert_eq!(turn.sampling_step, 0);
    let committed = store
        .commits
        .lock()
        .unwrap()
        .iter()
        .filter_map(|commit| commit.runtime.turn.as_ref().map(|turn| turn.phase))
        .collect::<Vec<_>>();
    assert_eq!(
        committed,
        vec![AgentTurnPhase::Sampling, AgentTurnPhase::Cancelled]
    );
}
