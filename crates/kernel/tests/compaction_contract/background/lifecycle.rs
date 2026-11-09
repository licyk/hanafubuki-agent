use super::*;

/// Its usage is the session's, recorded once whether the result is
/// installed or discarded, and no turn's budget counts it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_is_recorded_once_and_never_charged_to_a_turn() {
    let model = BackgroundModel::local(
        seed_then([answer("third answer", 200)]),
        [compaction(Answer::Summary("the first turn"))],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    seed(&service, session.id).await;
    background_ready(&service, session.id).await;
    let usage = service.snapshot(session.id).unwrap().usage.unwrap();
    assert_eq!(usage.cumulative_input_tokens, 100 + IN_BAND + 700);
    assert_eq!(usage.cumulative_output_tokens, 2 + 2 + 30);
    // The compaction's 700 input tokens would exceed the third turn's
    // budget; its own 200 do not.
    service
        .set_session_turn_budget_policy(
            session.id,
            AgentTurnBudgetPolicy {
                maximum_cumulative_model_input_tokens: 800,
                ..AgentTurnBudgetPolicy::default()
            },
        )
        .unwrap();
    service.start_turn(session.id, turn("third")).await.unwrap();
    assert!(background_completed(&service, session.id));
    let usage = service.snapshot(session.id).unwrap().usage.unwrap();
    assert_eq!(usage.cumulative_input_tokens, 100 + IN_BAND + 700 + 200);

    // Discarded when the strategy changes: still recorded, once.
    let background = Gate::default();
    let model = BackgroundModel::local(
        seed_then([answer("third answer", 200)]),
        [ScriptedCompaction {
            answer_when_cancelled: true,
            ..held(&background, Answer::Summary("the first turn"))
        }],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    seed(&service, session.id).await;
    background_sent(&model).await;
    let strategy = tokio::task::spawn_blocking({
        let service = service.clone();
        move || {
            service.set_session_compaction_strategy(session.id, AgentCompactionStrategy::Standard)
        }
    });
    assert!(strategy.await.unwrap().unwrap());
    assert_eq!(background_state(&service, session.id), None);
    eventually("the discarded compaction's usage", || {
        service
            .snapshot(session.id)
            .unwrap()
            .usage
            .is_some_and(|usage| usage.cumulative_input_tokens == 100 + IN_BAND + 700)
    })
    .await;
    let sink = Arc::new(RecordingSink::default());
    service
        .start_turn_with_sink(session.id, turn("third"), sink.clone())
        .await
        .unwrap();
    assert!(sink.compaction_phases().is_empty());
    let usage = service.snapshot(session.id).unwrap().usage.unwrap();
    assert_eq!(usage.cumulative_input_tokens, 100 + IN_BAND + 700 + 200);
}

/// Shutdown cancels the running compaction and waits for its task, so the
/// usage of one that answered anyway is recorded; a restart finds nothing
/// of it to recover.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_the_compaction_and_a_restart_recovers_nothing_of_it() {
    let background = Gate::default();
    let model = BackgroundModel::local(
        seed_then([answer("third answer", 200)]),
        [ScriptedCompaction {
            answer_when_cancelled: true,
            ..held(&background, Answer::Summary("the first turn"))
        }],
    );
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    seed(&service, session.id).await;
    background_sent(&model).await;

    let stopped = tokio::task::spawn_blocking({
        let service = service.clone();
        move || service.stop_operations(Instant::now() + OUTCOME_DEADLINE)
    });
    assert!(stopped.await.unwrap().is_empty());
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .usage
            .unwrap()
            .cumulative_input_tokens,
        100 + IN_BAND + 700
    );

    let restarted = AgentService::with_history_store(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    let report = restarted.recover_sessions().unwrap();
    assert_eq!(report.restored, 1);
    let recovered = restarted.snapshot(session.id).unwrap();
    assert_eq!(recovered.compaction, None);
    assert_eq!(recovered.background_compaction, None);
    assert_eq!(
        recovered.usage.unwrap().cumulative_input_tokens,
        100 + IN_BAND + 700
    );
}

/// Archiving cancels the compaction and waits for it before the history
/// moves; the session restored has none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archiving_cancels_the_compaction_and_waits_for_it() {
    let background = Gate::default();
    let model = BackgroundModel::local(
        seed_then([]),
        [held(&background, Answer::Summary("the first turn"))],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    seed(&service, session.id).await;
    background_sent(&model).await;
    let before = service.snapshot(session.id).unwrap();
    let archived = tokio::task::spawn_blocking({
        let service = service.clone();
        move || {
            let archived = service
                .archive_session(session.id, before.generation, before.revision)
                .unwrap();
            // Stopped already: no task of it is left to wait for.
            let stopped = service.stop_operations(Instant::now());
            (archived, stopped)
        }
    });
    let (archived, stopped) = archived.await.unwrap();
    assert!(stopped.is_empty(), "{stopped:?}");
    assert_eq!(background_state(&service, session.id), None);
    assert_eq!(
        archived.lifecycle,
        hanafubuki_agent_kernel::AgentSessionLifecycle::Archived
    );
}
