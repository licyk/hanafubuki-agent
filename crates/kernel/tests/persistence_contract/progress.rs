//! Execution progress a tool's sink reports is queued on its session and
//! committed in order by the session's consumer: reporting never waits, a
//! full queue coalesces outputs only, a turn's end follows what was queued
//! before it, and the application's synchronous reports follow the queue.

use std::sync::OnceLock;

use hanafubuki_agent_kernel::AgentTurnPhase;

use super::off_worker::service_over;
use super::*;

fn started(execution_id: u64) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionStarted {
        execution_id,
        generation: 1,
        action: AgentActivityAction::fixed("exec_command"),
        status: AgentExecutionProgressStatus::Running,
    }
}

fn output(execution_id: u64, text: &str, next_offset: u64) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionOutput {
        execution_id,
        generation: 1,
        text: text.to_string(),
        next_offset,
        truncated: false,
    }
}

fn exited(execution_id: u64) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionStatus {
        execution_id,
        generation: 1,
        status: AgentExecutionProgressStatus::Exited,
        exit_code: Some(0),
        cleanup_required: false,
    }
}

fn disposed(execution_id: u64) -> AgentToolProgressUpdate {
    AgentToolProgressUpdate::ExecutionDisposed {
        execution_id,
        generation: 1,
    }
}

/// The progress updates the session's journal holds, in order.
fn committed_progress(
    service: &AgentService,
    session_id: AgentSessionId,
) -> Vec<AgentToolProgressUpdate> {
    service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ExecutionProgress { update, .. } => Some(update),
            _ => None,
        })
        .collect()
}

/// A tool call's sink stamps its progress with the call, as the event
/// stream carries it; the application's own reports name none.
#[tokio::test]
async fn a_calls_progress_names_the_call_and_the_applications_names_none() {
    let store = Arc::new(RecordingHistoryStore::default());
    let (service, session_id) = service_over(store);

    service
        .execution_progress_sink(session_id, Some("call-1".to_string()))
        .report(started(3));
    service
        .flush_session_execution_progress(session_id)
        .await
        .unwrap();
    let application = service.clone();
    tokio::task::spawn_blocking(move || {
        application
            .report_session_execution_progress(session_id, disposed(3))
            .unwrap()
    })
    .await
    .unwrap();

    let calls: Vec<_> = service
        .events_since(session_id, None)
        .unwrap()
        .events
        .into_iter()
        .filter_map(|event| match event.kind {
            AgentEventKind::ExecutionProgress { call_id, update } => Some((call_id, update)),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        vec![
            (Some("call-1".to_string()), started(3)),
            (None, disposed(3)),
        ]
    );
}

#[tokio::test]
async fn a_failed_queued_commit_rolls_back_and_the_next_report_commits() {
    let store = Arc::new(RecordingHistoryStore::default());
    let (service, session_id) = service_over(store.clone());
    let created = service.snapshot(session_id).unwrap();
    store.fail_next_append.store(true, Ordering::SeqCst);

    report_progress(&service, session_id, started(7)).await;

    let rolled_back = service.snapshot(session_id).unwrap();
    assert_eq!(rolled_back.revision, created.revision);
    assert!(rolled_back.active_activities.is_empty());
    assert!(committed_progress(&service, session_id).is_empty());

    report_progress(&service, session_id, started(7)).await;

    assert_eq!(committed_progress(&service, session_id), vec![started(7)]);
    assert_eq!(
        service
            .snapshot(session_id)
            .unwrap()
            .active_activities
            .len(),
        1
    );
}

/// While the consumer's first commit is parked, a burst far past the
/// queue's capacity returns at once; once released, every update but the
/// outputs is committed, in order, and the preview ends with the newest
/// output.
#[test]
fn a_burst_of_reports_never_waits_and_keeps_every_other_update_and_the_newest_output() {
    let store = Arc::new(RecordingHistoryStore::default());
    let (service, session_id) = service_over(store.clone());
    let gate = Arc::new(AppendGate::new());
    *store.commit_gate.lock().unwrap() = Some(gate.clone());

    let (burst_returned_while_parked, committed, snapshot) = on_a_current_thread_runtime({
        let service = service.clone();
        move || async move {
            let sink = service.execution_progress_sink(session_id, None);
            sink.report(started(1));
            // The consumer commits the start and its acknowledgement parks.
            meet(|gate| &gate.started, &gate).await;
            let burst = thread::spawn(move || {
                for line in 0..200u64 {
                    sink.report(output(1, &format!("o{line}\n"), line + 1));
                }
                sink.report(exited(1));
                sink.report(disposed(1));
            });
            let burst_returned = tokio::task::spawn_blocking(move || burst.join().is_ok())
                .await
                .unwrap();
            let still_parked = store.commits.lock().unwrap().len() == 2;
            meet(|gate| &gate.release, &gate).await;
            service
                .flush_session_execution_progress(session_id)
                .await
                .unwrap();
            (
                burst_returned && still_parked,
                committed_progress(&service, session_id),
                service.snapshot(session_id).unwrap(),
            )
        }
    });

    assert!(burst_returned_while_parked);
    let others = committed
        .iter()
        .filter(|update| !matches!(update, AgentToolProgressUpdate::ExecutionOutput { .. }))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(others, vec![started(1), exited(1), disposed(1)]);
    assert_eq!(committed.first(), Some(&started(1)));
    assert_eq!(&committed[committed.len() - 2..], &[exited(1), disposed(1)]);
    let Some(AgentToolProgressUpdate::ExecutionOutput {
        text, next_offset, ..
    }) = committed.get(committed.len() - 3)
    else {
        panic!("no output precedes the status");
    };
    assert!(text.ends_with("o199\n"));
    assert_eq!(*next_offset, 200);
    assert!(committed.len() < 200, "the outputs were not coalesced");
    let activity = snapshot.recent_activities.last().unwrap();
    let preview = activity.output.as_ref().unwrap();
    assert_eq!(preview.tail.last().map(String::as_str), Some("o199"));
    assert_eq!(preview.next_offset, 200);
}

/// Reports from inside a turn (here, the model's stream) are committed
/// before its end, even while the first of them is still being appended.
#[test]
fn the_progress_a_turn_reports_is_committed_before_its_end() {
    struct ReportingModel {
        session: OnceLock<(AgentService, AgentSessionId)>,
    }
    impl ModelPort for ReportingModel {
        fn stream(
            &self,
            _request: ModelRequest,
            _cancellation: CancellationToken,
        ) -> ModelFuture<'_, ModelResult<ModelStream>> {
            let (service, session_id) = self.session.get().unwrap();
            let sink = service.execution_progress_sink(*session_id, None);
            sink.report(started(5));
            sink.report(disposed(5));
            let events = final_answer_events("Done.");
            Box::pin(
                async move { Ok(Box::new(ScriptedPersistenceStream { events }) as ModelStream) },
            )
        }
    }

    let store = Arc::new(RecordingHistoryStore::default());
    let model = Arc::new(ReportingModel {
        session: OnceLock::new(),
    });
    let service = AgentService::with_history_store(
        model.clone(),
        AgentKernelRegistries::default(),
        store.clone(),
    );
    let session_id = service.create_session(text_session_config()).unwrap().id;
    let _ = model.session.set((service.clone(), session_id));
    // The start's acknowledgement parks for a while, so the turn's end has
    // to wait for the commit, not only for the queue to empty.
    let gate = Arc::new(AppendGate::new());
    *store.commit_gate.lock().unwrap() = Some(gate.clone());
    let release = thread::spawn(move || {
        gate.started.wait();
        thread::sleep(Duration::from_millis(50));
        gate.release.wait();
    });

    let kinds = on_a_current_thread_runtime({
        let service = service.clone();
        move || async move {
            service.start_turn(session_id, "hello").await.unwrap();
            service
                .events_since(session_id, None)
                .unwrap()
                .events
                .into_iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>()
        }
    });
    release.join().unwrap();

    let position =
        |wanted: &dyn Fn(&AgentEventKind) -> bool| kinds.iter().position(wanted).unwrap();
    let disposed_at = position(
        &|kind| matches!(kind, AgentEventKind::ExecutionProgress { update, .. } if *update == disposed(5)),
    );
    let completed_at = position(&|kind| matches!(kind, AgentEventKind::TurnCompleted { .. }));
    assert!(
        disposed_at < completed_at,
        "the turn ended before its progress was committed"
    );
    assert_eq!(
        service.snapshot(session_id).unwrap().turn.unwrap().phase,
        AgentTurnPhase::Completed
    );
}

/// The application's synchronous report waits for what the session's sinks
/// queued before it, so an execution's last output precedes its disposal.
#[test]
fn a_synchronous_report_follows_the_progress_queued_before_it() {
    let store = Arc::new(RecordingHistoryStore::default());
    let (service, session_id) = service_over(store.clone());
    let gate = Arc::new(AppendGate::new());
    *store.commit_gate.lock().unwrap() = Some(gate.clone());

    let (waited, committed, snapshot) = on_a_current_thread_runtime({
        let service = service.clone();
        move || async move {
            let sink = service.execution_progress_sink(session_id, None);
            sink.report(started(3));
            sink.report(output(3, "tail\n", 5));
            meet(|gate| &gate.started, &gate).await;
            let report = tokio::task::spawn_blocking({
                let service = service.clone();
                move || service.report_session_execution_progress(session_id, disposed(3))
            });
            tokio::time::sleep(Duration::from_millis(50)).await;
            let waited = !report.is_finished();
            meet(|gate| &gate.release, &gate).await;
            report.await.unwrap().unwrap();
            (
                waited,
                committed_progress(&service, session_id),
                service.snapshot(session_id).unwrap(),
            )
        }
    });

    assert!(waited, "the synchronous report did not wait for the queue");
    assert_eq!(
        committed,
        vec![started(3), output(3, "tail\n", 5), disposed(3)]
    );
    let preview = snapshot
        .recent_activities
        .last()
        .unwrap()
        .output
        .clone()
        .unwrap();
    assert_eq!(preview.head, vec!["tail".to_string()]);
}
