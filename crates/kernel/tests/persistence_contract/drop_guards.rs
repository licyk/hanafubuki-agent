//! A dropped turn or compaction ends without its drop guard waiting: the
//! guard hands its terminal commit to the history store, deferring it behind
//! a commit still in flight, and the store's acknowledgement publishes it,
//! wakes the terminal waiters and makes it visible. Each test runs on a
//! single-threaded runtime, where a guard that waited would hold the only
//! thread.

use std::time::Instant;

use hanafubuki_agent_kernel::{AgentCompactionStatus, AgentTurnPhase};

use super::off_worker::service_over;
use super::*;

/// Every event kind it is given, in order.
#[derive(Default)]
struct RecordingSink(Mutex<Vec<AgentEventKind>>);

impl AgentEventSink for RecordingSink {
    fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event.kind);
    }
}

impl RecordingSink {
    fn saw(&self, kind: impl Fn(&AgentEventKind) -> bool) -> bool {
        self.0.lock().unwrap().iter().any(kind)
    }
}

fn cancelled(kind: &AgentEventKind) -> bool {
    matches!(kind, AgentEventKind::TurnCancelled { .. })
}

/// Answers the first request with a final answer (unless it starts
/// `answered`) and never starts a response to a later one.
#[derive(Default)]
struct AnswerThenPendingModel {
    answered: AtomicBool,
}

impl AnswerThenPendingModel {
    fn pending() -> Self {
        Self {
            answered: AtomicBool::new(true),
        }
    }
}

impl ModelPort for AnswerThenPendingModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        if self.answered.swap(true, Ordering::SeqCst) {
            return Box::pin(std::future::pending());
        }
        let events = final_answer_events("Done.");
        Box::pin(async move { Ok(Box::new(ScriptedPersistenceStream { events }) as ModelStream) })
    }
}

/// Holds the acknowledgement `gate` parks until `returned` is set (two
/// seconds at most), then releases it. Joins to whether `returned` was set
/// while the acknowledgement was still parked.
fn release_after_return(
    gate: Arc<AppendGate>,
    parked: Arc<AtomicBool>,
    returned: Arc<AtomicBool>,
) -> thread::JoinHandle<bool> {
    thread::spawn(move || {
        gate.started.wait();
        parked.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !returned.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        let returned_while_parked = returned.load(Ordering::SeqCst);
        gate.release.wait();
        returned_while_parked
    })
}

fn last_committed_turn_phase(store: &RecordingHistoryStore) -> Option<AgentTurnPhase> {
    store
        .commits
        .lock()
        .unwrap()
        .last()
        .and_then(|commit| commit.runtime.turn.as_ref().map(|turn| turn.phase))
}

/// Aborted while its sampling step's append is parked: the guard defers the
/// cancellation behind that commit and returns at once; the commit's
/// settlement stages the cancellation and hands it back to the store as a
/// follow-up.
#[test]
fn an_aborted_turn_defers_its_cancellation_behind_its_parked_append() {
    let store = Arc::new(RecordingHistoryStore::default());
    let gate = Arc::new(AppendGate::new());
    *store.delta_gate.lock().unwrap() = Some(gate.clone());
    let (service, session_id) = service_over(store.clone());
    let sink = Arc::new(RecordingSink::default());
    let (parked, returned) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let watcher = release_after_return(gate, parked.clone(), returned.clone());

    let snapshot = on_a_current_thread_runtime({
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
            assert!(until(|| parked.load(Ordering::SeqCst)).await);
            turn.abort();
            assert!(turn.await.unwrap_err().is_cancelled());
            // Nothing is visible before the store acknowledges.
            assert!(!sink.saw(cancelled));
            returned.store(true, Ordering::SeqCst);
            assert!(
                until(|| sink.saw(cancelled)).await,
                "the cancellation never became visible"
            );
            service.snapshot(session_id).unwrap()
        }
    });

    assert!(
        watcher.join().unwrap(),
        "the drop guard waited for the parked append"
    );
    let turn = snapshot.turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Cancelled);
    // The parked append was published before the cancellation followed it.
    assert_eq!(turn.sampling_step, 1);
    assert_eq!(store.deltas.lock().unwrap().len(), 1);
    assert_eq!(
        last_committed_turn_phase(&store),
        Some(AgentTurnPhase::Cancelled)
    );
}

/// Aborted while it waits for the model, with nothing in flight: the guard
/// stages the cancellation, hands it to the store and returns; the
/// cancellation becomes visible when the store acknowledges it.
#[test]
fn an_aborted_turn_hands_its_cancellation_to_the_store_without_waiting() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(AnswerThenPendingModel::pending()),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session_id = service.create_session(text_session_config()).unwrap().id;
    let sink = Arc::new(RecordingSink::default());
    let gate = Arc::new(AppendGate::new());
    let (parked, returned) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let watcher = release_after_return(gate.clone(), parked.clone(), returned.clone());

    let snapshot = on_a_current_thread_runtime({
        let (service, store, sink) = (service.clone(), store.clone(), sink.clone());
        move || async move {
            let turn = tokio::spawn({
                let (service, sink) = (service.clone(), sink.clone());
                async move {
                    service
                        .start_turn_with_sink(session_id, "hello", sink)
                        .await
                }
            });
            assert!(
                until(|| sink.saw(|kind| matches!(kind, AgentEventKind::SamplingStarted { .. })))
                    .await
            );
            // The turn waits for a response that never starts; the next
            // queued commit is its cancellation.
            *store.commit_gate.lock().unwrap() = Some(gate);
            turn.abort();
            assert!(turn.await.unwrap_err().is_cancelled());
            assert!(until(|| parked.load(Ordering::SeqCst)).await);
            assert!(!sink.saw(cancelled));
            returned.store(true, Ordering::SeqCst);
            assert!(
                until(|| sink.saw(cancelled)).await,
                "the cancellation never became visible"
            );
            service.snapshot(session_id).unwrap()
        }
    });

    assert!(
        watcher.join().unwrap(),
        "the drop guard waited for its own append"
    );
    assert_eq!(snapshot.turn.unwrap().phase, AgentTurnPhase::Cancelled);
    assert_eq!(
        last_committed_turn_phase(&store),
        Some(AgentTurnPhase::Cancelled)
    );
}

/// A compaction aborted while it waits for the model: its guard hands the
/// cancelled state to the store and returns; the state is published when the
/// store acknowledges it.
#[test]
fn an_aborted_compaction_hands_its_terminal_state_to_the_store_without_waiting() {
    let store = Arc::new(RecordingHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(AnswerThenPendingModel::default()),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session_id = service.create_session(text_session_config()).unwrap().id;
    let gate = Arc::new(AppendGate::new());
    let (parked, returned) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let watcher = release_after_return(gate.clone(), parked.clone(), returned.clone());

    let compaction = on_a_current_thread_runtime({
        let (service, store) = (service.clone(), store.clone());
        move || async move {
            service.start_turn(session_id, "hello").await.unwrap();
            let session = service.snapshot(session_id).unwrap();
            let compacting = tokio::spawn({
                let service = service.clone();
                async move {
                    service
                        .compact_session(session_id, session.generation, session.revision)
                        .await
                }
            });
            let status = |service: &AgentService| {
                service
                    .snapshot(session_id)
                    .unwrap()
                    .compaction
                    .map(|compaction| compaction.status)
            };
            assert!(until(|| status(&service) == Some(AgentCompactionStatus::Running)).await);
            // The compaction waits for a response that never starts; the next
            // queued commit is its terminal state.
            *store.commit_gate.lock().unwrap() = Some(gate);
            compacting.abort();
            assert!(compacting.await.unwrap_err().is_cancelled());
            assert!(until(|| parked.load(Ordering::SeqCst)).await);
            returned.store(true, Ordering::SeqCst);
            service.snapshot(session_id).unwrap().compaction.unwrap()
        }
    });

    assert!(
        watcher.join().unwrap(),
        "the drop guard waited for its own append"
    );
    assert_eq!(compaction.status, AgentCompactionStatus::Cancelled);
    let committed = store.commits.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        committed
            .runtime
            .compaction
            .map(|compaction| compaction.status),
        Some(AgentCompactionStatus::Cancelled)
    );
}
