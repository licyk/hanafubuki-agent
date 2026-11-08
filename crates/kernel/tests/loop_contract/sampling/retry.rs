use super::*;

#[tokio::test]
async fn response_stream_retry_reuses_turn_session_without_duplicate_history() {
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let inner = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("recovered"),
    )]);
    let model = Arc::new(EstablishmentFailingModel::new(inner, true));
    let service = AgentService::new(model.clone(), FakeTools::new([]).await.registries());
    let mut session_config = config();
    session_config.sampling_retry_policy = hanafubuki_agent_kernel::AgentSamplingRetryPolicy {
        maximum_retries: 2,
        initial_backoff_millis: 7,
        maximum_backoff_millis: 7,
    };
    let session = service.create_session(session_config).unwrap();

    let outcome = service
        .start_turn(session.id, "hello".to_string())
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "recovered");
    assert_eq!(model.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        test_session_view(&service, session.id)
            .unwrap()
            .model_context
            .iter()
            .filter(|item| matches!(item, ConversationItem::UserMessage { .. }))
            .count(),
        1
    );
    let retry = service
        .events_since(session.id, None)
        .unwrap()
        .events
        .iter()
        .find_map(|event| match &event.kind {
            AgentEventKind::ModelRetryScheduled {
                current_retry,
                maximum_retries,
                delay_millis,
                ..
            } => Some((*current_retry, *maximum_retries, *delay_millis)),
            _ => None,
        });
    assert_eq!(retry, Some((1, 2, 7)));
}

#[tokio::test]
async fn exhausted_request_retries_are_not_replayed_by_the_agent() {
    let expected = request(vec![
        ModelInputItem::user_text("hello"),
        ModelInputItem::user_text("again"),
    ]);
    let inner = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("recovered next turn"),
    )]);
    let model = Arc::new(EstablishmentFailingModel::new(inner, false));
    let service = AgentService::new(model.clone(), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    assert!(matches!(
        service.start_turn(session.id, "hello".to_string()).await,
        Err(AgentError::ModelRequestRetriesExhausted { attempts: 3, .. })
    ));
    assert_eq!(model.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(model.inner.remaining_exchanges(), 1);
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(AgentTurnTerminalReason::ModelRequestRetriesExhausted)
    );
    let next = service
        .start_turn(session.id, "again".to_string())
        .await
        .unwrap();
    assert_eq!(next.final_answer, "recovered next turn");
    assert_eq!(model.inner.remaining_exchanges(), 0);
}

#[tokio::test]
async fn response_stream_retry_exhaustion_fails_once_with_a_structured_reason() {
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let inner = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("must not run"),
    )]);
    let model = Arc::new(EstablishmentFailingModel::with_failures(inner, true, 3));
    let service = AgentService::new(model.clone(), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();

    assert!(matches!(
        service.start_turn(session.id, "hello".to_string()).await,
        Err(AgentError::ModelStreamRetriesExhausted { retries: 2, .. })
    ));
    assert_eq!(model.attempts.load(Ordering::SeqCst), 3);
    assert_eq!(model.inner.remaining_exchanges(), 1);
    let snapshot = service.snapshot(session.id).unwrap();
    let turn = snapshot.turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Failed);
    assert_eq!(
        turn.terminal_reason,
        Some(AgentTurnTerminalReason::ModelStreamRetriesExhausted)
    );
    assert_eq!(
        service
            .events_since(session.id, None)
            .unwrap()
            .events
            .iter()
            .filter(|event| matches!(event.kind, AgentEventKind::ModelRetryScheduled { .. }))
            .count(),
        2
    );
}

#[tokio::test]
async fn cancellation_interrupts_response_stream_retry_backoff() {
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let inner = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("must not run"),
    )]);
    let model = Arc::new(EstablishmentFailingModel::new(inner, true));
    let service = AgentService::new(model.clone(), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    let sink = Arc::new(SnapshotCheckingEventSink {
        service: service.clone(),
        session_id: session.id,
        events: Mutex::new(Vec::new()),
    });
    let accepted = service
        .submit_turn_with_sink(session.id, "hello".to_string(), sink)
        .unwrap();
    for _ in 0..100 {
        if service
            .events_since(session.id, None)
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ModelRetryScheduled { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    let outcome = service
        .cancel_turn_confirmed(session.id, accepted.turn_id, accepted.generation)
        .await
        .unwrap();
    assert_eq!(outcome.phase, AgentTurnPhase::Cancelled);
    assert_eq!(outcome.reason, AgentTurnTerminalReason::UserInterrupted);
    assert_eq!(model.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(model.inner.remaining_exchanges(), 1);
}

#[tokio::test]
async fn per_turn_sink_observes_committed_events_in_sequence_outside_the_registry_lock() {
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("hello back"),
    )]);
    let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    let sink = Arc::new(SnapshotCheckingEventSink {
        service: service.clone(),
        session_id: session.id,
        events: Mutex::new(Vec::new()),
    });

    service
        .start_turn_with_sink(session.id, "hello".to_string(), sink.clone())
        .await
        .unwrap();

    let events = sink.events.lock().unwrap();
    assert!(matches!(
        events.first().unwrap().kind,
        AgentEventKind::TurnStarted
    ));
    assert!(matches!(
        events.last().unwrap().kind,
        AgentEventKind::TurnCompleted { .. }
    ));
    assert!(events
        .windows(2)
        .all(|pair| pair[0].sequence.value() + 1 == pair[1].sequence.value()));
}

#[tokio::test]
async fn cancellation_wins_before_image_fallback_retry() {
    let model = Arc::new(CancellingImageRejectionModel {
        attempts: AtomicUsize::new(0),
    });
    let service = AgentService::new(model.clone(), FakeTools::new([]).await.registries());
    let session = service.create_session(config()).unwrap();
    let input = direct_image_turn(&service, session.id, "inspect");

    assert!(matches!(
        service.start_turn(session.id, input).await,
        Err(AgentError::Cancelled)
    ));
    assert_eq!(model.attempts.load(Ordering::SeqCst), 1);
}
