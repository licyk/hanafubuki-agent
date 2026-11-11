use super::*;
use crate::{AgentActivityAction, AgentExecutionProgressStatus};

mod fixtures {
    use super::*;

    pub(super) fn started(execution_id: u64) -> AgentToolProgressUpdate {
        AgentToolProgressUpdate::ExecutionStarted {
            execution_id,
            generation: 1,
            action: AgentActivityAction::fixed("exec_command"),
            status: AgentExecutionProgressStatus::Running,
        }
    }

    pub(super) fn output(
        execution_id: u64,
        text: &str,
        next_offset: u64,
    ) -> AgentToolProgressUpdate {
        AgentToolProgressUpdate::ExecutionOutput {
            execution_id,
            generation: 1,
            text: text.to_string(),
            next_offset,
            truncated: false,
        }
    }

    pub(super) fn exited(execution_id: u64) -> AgentToolProgressUpdate {
        AgentToolProgressUpdate::ExecutionStatus {
            execution_id,
            generation: 1,
            status: AgentExecutionProgressStatus::Exited,
            exit_code: Some(0),
            cleanup_required: false,
        }
    }

    pub(super) fn disposed(execution_id: u64) -> AgentToolProgressUpdate {
        AgentToolProgressUpdate::ExecutionDisposed {
            execution_id,
            generation: 1,
        }
    }

    /// A consumer that never runs, so reports stay queued.
    pub(super) fn idle_consumer(_id: u64) -> tokio::task::AbortHandle {
        tokio::spawn(std::future::pending::<()>()).abort_handle()
    }

    /// Everything queued, in the order a consumer takes it.
    pub(super) fn drain(queue: &ExecutionProgressQueue) -> Vec<AgentToolProgressUpdate> {
        let id = queue.state.lock().consumer.as_ref().unwrap().0;
        std::iter::from_fn(|| queue.take(id).map(|(_, update)| update)).collect()
    }
}

use fixtures::{disposed, drain, exited, idle_consumer, output, started};

#[tokio::test]
async fn a_full_queue_coalesces_outputs_and_keeps_every_other_update_in_order() {
    let queue = ExecutionProgressQueue::default();
    assert!(queue.report(started(1), idle_consumer));
    for line in 0..100u64 {
        assert!(queue.report(output(1, &format!("o{line}\n"), line + 1), idle_consumer));
    }
    assert!(queue.report(started(2), idle_consumer));
    // After its status, an execution's output is queued, not moved before it.
    assert!(queue.report(exited(1), idle_consumer));
    assert!(queue.report(output(1, "late\n", 101), idle_consumer));
    assert!(queue.report(output(1, "later\n", 102), idle_consumer));
    assert!(queue.report(disposed(1), idle_consumer));

    let drained = drain(&queue);

    let capacity = EXECUTION_PROGRESS_QUEUE_CAPACITY;
    assert_eq!(drained.len(), capacity + 4);
    assert_eq!(drained[0], started(1));
    for (line, update) in drained[1..capacity - 1].iter().enumerate() {
        assert_eq!(*update, output(1, &format!("o{line}\n"), line as u64 + 1));
    }
    let coalesced = (capacity as u64 - 2..100)
        .map(|line| format!("o{line}\n"))
        .collect::<String>();
    assert_eq!(drained[capacity - 1], output(1, &coalesced, 100));
    assert_eq!(drained[capacity], started(2));
    assert_eq!(drained[capacity + 1], exited(1));
    assert_eq!(drained[capacity + 2], output(1, "late\nlater\n", 102));
    assert_eq!(drained[capacity + 3], disposed(1));
}

#[tokio::test]
async fn a_coalesced_output_keeps_its_newest_bytes_and_a_gap_replaces_it() {
    let queue = ExecutionProgressQueue::default();
    for line in 0..EXECUTION_PROGRESS_QUEUE_CAPACITY as u64 {
        assert!(queue.report(output(2, "x", line + 1), idle_consumer));
    }
    let long = "界".repeat(MAXIMUM_COALESCED_OUTPUT_BYTES / 3 + 1);
    assert!(queue.report(output(1, "first", 1), idle_consumer));
    assert!(queue.report(output(1, &long, 2), idle_consumer));
    let drained = drain(&queue);
    let AgentToolProgressUpdate::ExecutionOutput {
        text,
        truncated,
        next_offset,
        ..
    } = drained.last().unwrap()
    else {
        panic!("the last update is not an output");
    };
    assert!(text.len() <= MAXIMUM_COALESCED_OUTPUT_BYTES);
    assert!(long.ends_with(text.as_str()));
    assert!(*truncated);
    assert_eq!(*next_offset, 2);

    for line in 0..EXECUTION_PROGRESS_QUEUE_CAPACITY as u64 {
        assert!(queue.report(output(2, "x", line + 1), idle_consumer));
    }
    assert!(queue.report(output(1, "skipped", 1), idle_consumer));
    let mut gap = output(1, "after the gap", 9);
    if let AgentToolProgressUpdate::ExecutionOutput { truncated, .. } = &mut gap {
        *truncated = true;
    }
    assert!(queue.report(gap.clone(), idle_consumer));
    assert_eq!(drain(&queue).last(), Some(&gap));
}

#[tokio::test]
async fn closing_refuses_reports_stops_the_consumer_and_releases_flushes() {
    let queue = Arc::new(ExecutionProgressQueue::default());
    let consumer = tokio::spawn(std::future::pending::<()>());
    let abort = consumer.abort_handle();
    assert!(queue.report(started(1), move |_| abort));
    let flushed = tokio::spawn({
        let queue = queue.clone();
        async move { queue.flushed().await }
    });
    tokio::task::yield_now().await;
    assert!(!flushed.is_finished());

    queue.close();

    flushed.await.unwrap();
    assert!(consumer.await.unwrap_err().is_cancelled());
    assert!(!queue.report(started(2), idle_consumer));
    assert!(queue.flushed_blocking(Instant::now()));
}

#[tokio::test]
async fn a_consumer_stopped_early_lets_the_next_report_start_another() {
    let queue = ExecutionProgressQueue::default();
    let started_consumers = std::sync::atomic::AtomicUsize::new(0);
    let spawn = |_| {
        started_consumers.fetch_add(1, Ordering::SeqCst);
        idle_consumer(0)
    };
    assert!(queue.report(started(1), spawn));
    assert!(queue.report(disposed(1), spawn));
    assert_eq!(started_consumers.load(Ordering::SeqCst), 1);
    let (sequence, _) = queue.take(0).unwrap();

    queue.consumer_stopped(0);

    // The update it took settles on its own; a flush does not wait for it.
    assert_eq!(queue.state.lock().applied, sequence);
    assert!(queue.report(exited(2), spawn));
    assert_eq!(started_consumers.load(Ordering::SeqCst), 2);
}

/// A runtime that has shut down drops the consumer inside `spawn`: the
/// report returns, the consumer slot is free again, and a flush on a live
/// runtime resumes the consumer and commits what was queued.
#[test]
fn a_report_on_a_runtime_that_has_shut_down_frees_the_consumer_slot() {
    let service = AgentService::without_default_model_with_history_store(
        crate::AgentKernelRegistries::default(),
        Arc::new(crate::EphemeralAgentHistoryStore::default()),
    );
    let session_id = service
        .create_model_session(
            crate::AgentSessionConfig {
                provider_id: "provider-1".to_string(),
                provider_protocol: hanafubuki_agent_model_api::ModelApiProtocol::Responses,
                model: "fixture".to_string(),
                instructions: "fixture".to_string(),
                mode: crate::AgentSessionMode::Default,
                reasoning_effort: None,
                context_window_tokens: None,
                input_capabilities: Default::default(),
                auto_compaction_enabled: true,
                auto_compact_threshold_percent: 80,
                compaction_capability:
                    hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
                sampling_retry_policy: crate::AgentSamplingRetryPolicy::default(),
                context_window_source: None,
                compaction_strategy: Default::default(),
                scope: Default::default(),
                role_instructions: None,
            },
            hanafubuki_agent_model_api::ModelApiProtocol::Responses,
            "http://127.0.0.1:1/v1",
            None,
            Vec::new(),
            hanafubuki_agent_model_api::ModelRetryPolicy::default(),
            hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary,
            crate::AgentSessionWorkspace::default(),
        )
        .unwrap()
        .id;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stopped = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let handle = stopped.handle().clone();
        drop(stopped);
        service.queue_execution_progress(session_id, started(1), &handle);
        let cell = service.inner.sessions.get(session_id).unwrap();
        let freed = cell.progress().state.lock().consumer.is_none();
        let live = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let resumed = live.block_on(async {
            // A flush resumes the consumer, with no new report.
            service.flush_execution_progress(&cell).await;
            let resumed = service.events_since(session_id, None).unwrap().events.len() == 1;
            service.queue_execution_progress(
                session_id,
                disposed(1),
                &tokio::runtime::Handle::current(),
            );
            service.flush_execution_progress(&cell).await;
            resumed
        });
        let committed = service
            .events_since(session_id, None)
            .unwrap()
            .events
            .into_iter()
            .filter_map(|event| match event.kind {
                AgentEventKind::ExecutionProgress { update } => Some(update),
                _ => None,
            })
            .collect::<Vec<_>>();
        let _ = sender.send((freed, resumed, committed));
    });

    let (freed, resumed, committed) = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a report on a stopped runtime deadlocked");
    assert!(freed);
    assert!(resumed, "the flush did not resume the stopped consumer");
    assert_eq!(committed, vec![started(1), disposed(1)]);
}
