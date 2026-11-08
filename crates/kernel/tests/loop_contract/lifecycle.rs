use super::*;

#[tokio::test]
async fn event_retention_is_bounded_and_reports_sequence_truncation() {
    let limits = AgentHardLimits {
        event_buffer_capacity: 3,
        ..FIRST_BATCH_HARD_LIMITS
    };
    let expected = request(vec![ModelInputItem::user_text("hello")]);
    let model = FakeModelTransport::new([FakeModelExchange::new(
        expected,
        deterministic_assistant_events("world"),
    )]);
    let service = AgentService::with_limits(
        Arc::new(model),
        FakeTools::new([]).await.registries(),
        limits,
    );
    let session = service.create_session(config()).unwrap();
    service
        .start_turn(session.id, "hello".to_string())
        .await
        .unwrap();

    let events = service.events_since(session.id, None).unwrap();
    assert_eq!(events.events.len(), 3);
    assert!(!events.truncated);
    let from_initial_sequence = service
        .events_since(session.id, Some(session.next_event_sequence))
        .unwrap();
    assert!(from_initial_sequence.truncated);
}

#[tokio::test]
async fn cancellation_commits_one_terminal_event_before_confirmed_deletion() {
    let service = AgentService::new(
        Arc::new(CancellationOnlyModel),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let executing_service = service.clone();
    let execution = tokio::spawn(async move {
        executing_service
            .start_turn(session.id, "wait".to_string())
            .await
    });
    let turn = wait_for_active_turn(&service, session.id).await;
    let active_revision = test_session_view(&service, session.id).unwrap().revision;

    assert_eq!(
        service.delete_session_confirmed(session.id, turn.generation, active_revision),
        Err(AgentError::SessionMutationNotConfirmed)
    );
    service
        .request_turn_cancellation(session.id, turn.id, turn.generation)
        .await
        .unwrap();
    assert_eq!(execution.await.unwrap(), Err(AgentError::Cancelled));

    let snapshot = test_session_view(&service, session.id).unwrap();
    assert_eq!(
        snapshot.turn.as_ref().unwrap().phase,
        AgentTurnPhase::Cancelled
    );
    assert_eq!(
        snapshot
            .model_context
            .iter()
            .filter(|item| matches!(item, ConversationItem::UserMessage { .. }))
            .count(),
        1
    );
    assert!(!snapshot
        .model_context
        .iter()
        .any(|item| matches!(item, ConversationItem::AssistantMessage { .. })));
    let events = service.events_since(session.id, None).unwrap();
    assert_eq!(
        events
            .events
            .iter()
            .filter(|event| event.kind.is_terminal())
            .count(),
        1
    );
    let receipt = service
        .delete_session_confirmed(session.id, turn.generation, snapshot.revision)
        .unwrap();
    assert_eq!(receipt.session_id, session.id);
    assert_eq!(receipt.generation, turn.generation);
    assert_eq!(
        test_session_view(&service, session.id),
        Err(AgentError::SessionNotFound)
    );
}

#[tokio::test]
async fn managed_turn_submission_returns_identity_and_releases_after_cancellation() {
    let service = AgentService::new(
        Arc::new(CancellationOnlyModel),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let sink = Arc::new(SnapshotCheckingEventSink {
        service: service.clone(),
        session_id: session.id,
        events: Mutex::new(Vec::new()),
    });

    let accepted = service
        .submit_turn_with_sink(session.id, "wait".to_string(), sink)
        .unwrap();
    assert_eq!(accepted.session_id, session.id);
    let turn = wait_for_active_turn(&service, session.id).await;
    assert_eq!(accepted.turn_id, turn.id);
    assert_eq!(accepted.generation, turn.generation);

    let outcome = service
        .cancel_turn_confirmed(session.id, accepted.turn_id, accepted.generation)
        .await
        .unwrap();
    assert_eq!(outcome.phase, AgentTurnPhase::Cancelled);
    assert_eq!(
        outcome.reason,
        hanafubuki_agent_kernel::AgentTurnTerminalReason::UserInterrupted
    );
    let turn = service.snapshot(session.id).unwrap().turn.unwrap();
    assert_eq!(turn.phase, AgentTurnPhase::Cancelled);
    assert_eq!(turn.terminal_reason, Some(outcome.reason));
    assert!(service
        .shutdown(std::time::Instant::now() + Duration::from_secs(1))
        .is_empty());
}

#[tokio::test]
async fn confirmed_cancellation_aborts_only_the_exact_uncooperative_managed_turn() {
    let service = AgentService::new(
        Arc::new(UncooperativeModel),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let sink = Arc::new(SnapshotCheckingEventSink {
        service: service.clone(),
        session_id: session.id,
        events: Mutex::new(Vec::new()),
    });
    let accepted = service
        .submit_turn_with_sink(session.id, "wait".to_string(), sink)
        .unwrap();
    wait_for_active_turn(&service, session.id).await;

    assert_eq!(
        service
            .cancel_turn_confirmed(
                session.id,
                accepted.turn_id,
                AgentGeneration::new(accepted.generation.value() + 1),
            )
            .await,
        Err(AgentError::StaleGeneration)
    );
    let outcome = service
        .cancel_turn_confirmed(session.id, accepted.turn_id, accepted.generation)
        .await
        .unwrap();
    assert_eq!(outcome.phase, AgentTurnPhase::Cancelled);
    assert_eq!(
        outcome.reason,
        hanafubuki_agent_kernel::AgentTurnTerminalReason::UserInterrupted
    );
    assert_eq!(
        service
            .events_since(session.id, None)
            .unwrap()
            .events
            .iter()
            .filter(|event| event.kind.is_terminal())
            .count(),
        1
    );
}

#[tokio::test]
async fn dropping_the_execution_future_still_commits_a_terminal_state() {
    let service = AgentService::new(
        Arc::new(CancellationOnlyModel),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let executing_service = service.clone();
    let execution = tokio::spawn(async move {
        executing_service
            .start_turn(session.id, "wait".to_string())
            .await
    });
    wait_for_active_turn(&service, session.id).await;

    execution.abort();
    let _ = execution.await;

    let snapshot = test_session_view(&service, session.id).unwrap();
    assert_eq!(
        snapshot.runtime.turn.unwrap().phase,
        AgentTurnPhase::Cancelled
    );
    assert_eq!(
        service
            .events_since(session.id, None)
            .unwrap()
            .events
            .iter()
            .filter(|event| event.kind.is_terminal())
            .count(),
        1
    );
}

#[tokio::test]
async fn each_terminal_turn_settles_its_own_duration_on_its_final_history_item() {
    let model = RecordingAssistantModel::default();
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();

    service.start_turn(session.id, "first").await.unwrap();
    let after_first = service.snapshot(session.id).unwrap();
    let first_turn = after_first.turn.clone().unwrap();
    assert!(first_turn.phase.is_terminal());
    let first_elapsed = u64::try_from(first_turn.updated_at - first_turn.created_at).unwrap();
    let first_items = service
        .open_session(session.id, 100)
        .unwrap()
        .transcript
        .items;
    let first_final = first_items
        .iter()
        .rev()
        .find(|item| item.turn_id == Some(first_turn.id))
        .unwrap();
    assert_eq!(first_final.turn_working_millis, Some(first_elapsed));
    assert_eq!(first_final.item_revision, 1);

    service.start_turn(session.id, "second").await.unwrap();
    let after_second = service.snapshot(session.id).unwrap();
    let second_turn = after_second.turn.clone().unwrap();
    let second_elapsed = u64::try_from(second_turn.updated_at - second_turn.created_at).unwrap();
    let items = service
        .open_session(session.id, 100)
        .unwrap()
        .transcript
        .items;
    let settled = items
        .iter()
        .filter_map(|item| {
            item.turn_working_millis
                .map(|elapsed| (item.turn_id, elapsed))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        settled,
        vec![
            (Some(first_turn.id), first_elapsed),
            (Some(second_turn.id), second_elapsed),
        ],
        "each loop must retain its own duration instead of a session total"
    );
}

#[tokio::test]
async fn a_running_turn_has_no_settled_duration_until_terminalization() {
    let service = AgentService::new(
        Arc::new(CancellationOnlyModel),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    let executing_service = service.clone();
    let execution = tokio::spawn(async move {
        executing_service
            .start_turn(session.id, "wait".to_string())
            .await
    });
    let turn = wait_for_active_turn(&service, session.id).await;

    let running = service.snapshot(session.id).unwrap();
    assert!(!running.turn.unwrap().phase.is_terminal());
    assert!(service
        .open_session(session.id, 100)
        .unwrap()
        .transcript
        .items
        .iter()
        .all(|item| item.turn_working_millis.is_none()));

    service
        .request_turn_cancellation(session.id, turn.id, turn.generation)
        .await
        .unwrap();
    let _ = execution.await.unwrap();

    let cancelled = service.snapshot(session.id).unwrap();
    let settled = cancelled.turn.clone().unwrap();
    assert_eq!(settled.phase, AgentTurnPhase::Cancelled);
    let final_item = service
        .open_session(session.id, 100)
        .unwrap()
        .transcript
        .items
        .into_iter()
        .rev()
        .find(|item| item.turn_id == Some(settled.id))
        .unwrap();
    assert_eq!(
        final_item.turn_working_millis,
        Some(u64::try_from(settled.updated_at - settled.created_at).unwrap())
    );
}
