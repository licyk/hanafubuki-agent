//! A panic in a turn's own work ends that turn as an internal failure (never
//! as the user cancellation its drop guard commits) and leaves the session
//! ready for its next turn.

use super::*;

/// Panics while its first response streams; answers every later request.
#[derive(Default)]
struct PanickingStreamModel {
    attempts: AtomicUsize,
}

struct PanickingStream {
    started: bool,
}

impl ModelEventStream for PanickingStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelResult<ModelEvent>>> {
        Box::pin(async move {
            if !self.started {
                self.started = true;
                return Some(Ok(ModelEvent::ResponseStarted {
                    response_id: "response-panics".to_string(),
                    metadata: ModelResponseMetadata::default(),
                }));
            }
            panic!("the model stream broke");
        })
    }
}

impl ModelPort for PanickingStreamModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> ModelFuture<'_, ModelResult<ModelStream>> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(if attempt == 0 {
                Box::new(PanickingStream { started: false }) as ModelStream
            } else {
                Box::new(RecordedEventStream {
                    events: deterministic_assistant_events("Recovered").into(),
                }) as ModelStream
            })
        })
    }
}

#[derive(Default)]
struct TerminalEvents(Mutex<Vec<AgentEventKind>>);

impl AgentEventSink for TerminalEvents {
    fn emit(&self, event: AgentEvent) {
        if event.kind.is_terminal() {
            self.0.lock().unwrap().push(event.kind);
        }
    }
}

fn assert_failed_internally(service: &AgentService, session_id: AgentSessionId) {
    let turn = test_session_view(service, session_id)
        .unwrap()
        .runtime
        .turn
        .unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(AgentTurnTerminalReason::InternalFailure)
    );
}

#[tokio::test]
async fn a_turn_whose_model_stream_panics_fails_and_the_session_takes_the_next_turn() {
    let service = AgentService::new(
        Arc::new(PanickingStreamModel::default()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let events = Arc::new(TerminalEvents::default());

    let failed = service
        .start_turn_with_sink(session.id, "hello", events.clone())
        .await;

    assert!(matches!(failed, Err(AgentError::Internal(_))), "{failed:?}");
    assert_failed_internally(&service, session.id);
    assert!(matches!(
        events.0.lock().unwrap().as_slice(),
        [AgentEventKind::TurnFailed {
            reason: AgentTurnTerminalReason::InternalFailure,
            ..
        }]
    ));
    let next = service
        .start_turn(session.id, "again".to_string())
        .await
        .unwrap();
    assert_eq!(next.final_answer, "Recovered");
}

#[tokio::test]
async fn a_submitted_turn_whose_model_stream_panics_fails_rather_than_cancels() {
    let service = AgentService::new(
        Arc::new(PanickingStreamModel::default()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let events = Arc::new(TerminalEvents::default());

    service
        .submit_turn_with_sink(session.id, "hello", events.clone())
        .unwrap();
    for _ in 0..500 {
        if !events.0.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    assert!(matches!(
        events.0.lock().unwrap().as_slice(),
        [AgentEventKind::TurnFailed {
            reason: AgentTurnTerminalReason::InternalFailure,
            ..
        }]
    ));
    assert_failed_internally(&service, session.id);
}

#[derive(Default)]
struct RecordedKernelFaults(Mutex<Vec<(String, String)>>);

impl hanafubuki_agent_kernel::AgentKernelFaultSink for RecordedKernelFaults {
    fn kernel_fault(&self, site: &str, message: &str) {
        self.0
            .lock()
            .unwrap()
            .push((site.to_string(), message.to_string()));
    }
}

#[tokio::test]
async fn a_turn_panic_is_reported_as_a_kernel_fault_once() {
    let faults = Arc::new(RecordedKernelFaults::default());
    let service = hanafubuki_agent_kernel::test_support::service_reporting_kernel_faults(
        Arc::new(PanickingStreamModel::default()),
        FakeTools::new([]).await.registries(),
        faults.clone(),
    );
    let session = service.create_session(config()).unwrap();

    let failed = service.start_turn(session.id, "hello".to_string()).await;

    assert!(matches!(failed, Err(AgentError::Internal(_))), "{failed:?}");
    assert_eq!(
        faults.0.lock().unwrap().as_slice(),
        [("turn".to_string(), "the model stream broke".to_string())]
    );
}
