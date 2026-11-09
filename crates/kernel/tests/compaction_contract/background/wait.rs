use super::*;

/// The second turn ends over the foreground threshold, after its first
/// step launched a compaction in the band.
fn required_seed_then(
    rest: impl IntoIterator<Item = ScriptedTurnResponse>,
) -> Vec<ScriptedTurnResponse> {
    [
        answer(&FIRST_ANSWER, 100),
        tool_call(IN_BAND),
        answer("second answer", OVER_THRESHOLD),
    ]
    .into_iter()
    .chain(rest)
    .collect()
}

/// Starts the third turn on its own task once the background compaction
/// was sent, and returns it with its sink.
async fn spawn_required_turn(
    service: &AgentService,
    model: &BackgroundModel,
    session: AgentSessionId,
) -> (
    tokio::task::JoinHandle<Result<hanafubuki_agent_kernel::AgentTurnOutcome, AgentError>>,
    Arc<RecordingSink>,
) {
    service.start_turn(session, turn("first")).await.unwrap();
    service.start_turn(session, turn("second")).await.unwrap();
    background_sent(model).await;
    let sink = Arc::new(RecordingSink::default());
    let task = tokio::spawn({
        let service = service.clone();
        let sink = sink.clone();
        async move {
            service
                .start_turn_with_sink(session, turn("third"), sink)
                .await
        }
    });
    eventually("the third turn", || {
        service
            .snapshot(session)
            .unwrap()
            .turn
            .is_some_and(|turn| !turn.phase.is_terminal() && turn.sampling_step == 0)
    })
    .await;
    (task, sink)
}

#[tokio::test]
async fn a_required_compaction_waits_for_the_running_one_and_installs_it() {
    let background = Gate::default();
    let model = BackgroundModel::local(
        required_seed_then([answer("third answer", 200)]),
        [held(&background, Answer::Summary("the first turn"))],
    );
    let service = AgentService::new(Arc::new(model.clone()), list_tools().await.registries());
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    let (third, sink) = spawn_required_turn(&service, &model, session.id).await;
    background.open();
    third.await.unwrap().unwrap();

    assert_eq!(sink.compaction_phases(), [AgentCompactionPhase::Background]);
    assert_eq!(model.compaction_requests().len(), 1);
    assert!(background_completed(&service, session.id));
}

#[tokio::test]
async fn a_required_compaction_compacts_in_line_when_the_running_one_fails() {
    let background = Gate::default();
    let model = BackgroundModel::local(
        required_seed_then([answer("third answer", 200)]),
        [
            held(&background, Answer::Fail),
            compaction(Answer::Summary("in line")),
        ],
    );
    let service = AgentService::new(Arc::new(model.clone()), list_tools().await.registries());
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    let (third, sink) = spawn_required_turn(&service, &model, session.id).await;
    background.open();
    third.await.unwrap().unwrap();

    assert_eq!(sink.compaction_phases(), [AgentCompactionPhase::PreTurn]);
    assert_eq!(model.compaction_requests().len(), 2);
}

#[tokio::test]
async fn an_interrupted_wait_ends_the_turn_and_leaves_the_compaction_running() {
    let background = Gate::default();
    let model = BackgroundModel::local(
        required_seed_then([]),
        [held(&background, Answer::Summary("the first turn"))],
    );
    let service = AgentService::new(Arc::new(model.clone()), list_tools().await.registries());
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    let (third, sink) = spawn_required_turn(&service, &model, session.id).await;
    let running = service.snapshot(session.id).unwrap().turn.unwrap();
    service
        .request_turn_cancellation(session.id, running.id, running.generation)
        .await
        .unwrap();
    assert!(matches!(third.await.unwrap(), Err(AgentError::Cancelled)));
    assert_eq!(
        service.snapshot(session.id).unwrap().turn.unwrap().phase,
        AgentTurnPhase::Cancelled
    );
    assert_eq!(
        background_state(&service, session.id),
        Some(AgentBackgroundCompactionState::Running)
    );
    background.open();
    background_ready(&service, session.id).await;
    assert!(sink.compaction_phases().is_empty());
    assert_eq!(model.compaction_requests().len(), 1);
    assert_eq!(model.requests().len(), 3);
}

/// The second turn's answer is large, after the snapshot: the installed
/// result leaves the next request over the threshold, and the turn
/// compacts in line as well.
#[tokio::test]
async fn an_install_that_leaves_the_context_over_the_threshold_is_followed_by_one_in_line() {
    let background = Gate::default();
    let large = "a large second answer ".repeat(1_800);
    let model = BackgroundModel::local(
        [
            answer(&FIRST_ANSWER, 100),
            answer(&large, IN_BAND),
            answer("third answer", 200),
        ],
        [
            held(&background, Answer::Summary("the first turn")),
            compaction(Answer::Summary("in line")),
        ],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    let (third, sink) = spawn_required_turn(&service, &model, session.id).await;
    background.open();
    third.await.unwrap().unwrap();

    assert_eq!(
        sink.compaction_phases(),
        [
            AgentCompactionPhase::Background,
            AgentCompactionPhase::PreTurn
        ]
    );
    assert_eq!(model.compaction_requests().len(), 2);
    assert!(format!("{:?}", model.compaction_requests()[1].input).contains("the first turn"));
}

#[tokio::test]
async fn compact_takes_the_running_compaction_and_compacts_itself_when_it_fails() {
    for (answer_given, expected) in [
        (
            Answer::Summary("the first turn"),
            AgentCompactionTrigger::Automatic,
        ),
        (Answer::Fail, AgentCompactionTrigger::Manual),
    ] {
        let background = Gate::default();
        let model = BackgroundModel::local(
            seed_then([]),
            [
                held(&background, answer_given),
                compaction(Answer::Summary("manual")),
            ],
        );
        let service = AgentService::new(
            Arc::new(model.clone()),
            FakeTools::new([]).await.registries(),
        );
        let session = service
            .create_session(background_config(ModelCompactionCapability::LocalSummary))
            .unwrap();
        seed(&service, session.id).await;
        let before = service.snapshot(session.id).unwrap();
        let compacting = tokio::spawn({
            let service = service.clone();
            async move {
                service
                    .compact_session(session.id, before.generation, before.revision)
                    .await
            }
        });
        background_sent(&model).await;
        background.open();
        let outcome = compacting.await.unwrap().unwrap();
        assert_eq!(outcome.compaction.trigger, expected);
        assert_eq!(
            outcome.compaction.phase == AgentCompactionPhase::Background,
            expected == AgentCompactionTrigger::Automatic
        );
        assert_eq!(
            model.compaction_requests().len(),
            if expected == AgentCompactionTrigger::Manual {
                2
            } else {
                1
            }
        );
    }
}
