use super::*;

#[tokio::test]
async fn nothing_launches_outside_the_band_without_a_completed_turn_or_the_strategy() {
    let model = BackgroundModel::local(
        [
            // Under the background threshold.
            answer(&FIRST_ANSWER, 100),
            answer("second answer", 4_000),
            // The band, in the first turn of a session.
            answer("only answer", IN_BAND),
            // The band, without the threshold in the turn's extensions.
            answer(&FIRST_ANSWER, 100),
            answer("second answer", IN_BAND),
            // The band, in a standard session.
            answer(&FIRST_ANSWER, 100),
            answer("second answer", IN_BAND),
        ],
        [],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let background = || {
        service
            .create_session(background_config(ModelCompactionCapability::LocalSummary))
            .unwrap()
            .id
    };
    let under = background();
    seed(&service, under).await;
    let first_turn = background();
    service.start_turn(first_turn, turn("only")).await.unwrap();
    let unconfigured = background();
    service.start_turn(unconfigured, "first").await.unwrap();
    service.start_turn(unconfigured, "second").await.unwrap();
    let standard = service
        .create_session(AgentSessionConfig {
            compaction_strategy: AgentCompactionStrategy::Standard,
            ..background_config(ModelCompactionCapability::LocalSummary)
        })
        .unwrap()
        .id;
    seed(&service, standard).await;

    assert!(model.compaction_requests().is_empty());
    for session in [under, first_turn, unconfigured, standard] {
        assert_eq!(background_state(&service, session), None);
    }
}

#[tokio::test]
async fn at_the_foreground_threshold_the_turn_compacts_in_line_and_launches_nothing() {
    let model = BackgroundModel::local(
        [
            answer(&FIRST_ANSWER, 100),
            answer("second answer", OVER_THRESHOLD),
            answer("third answer", 200),
        ],
        [compaction(Answer::Summary("in line"))],
    );
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    seed(&service, session.id).await;
    assert_eq!(background_state(&service, session.id), None);
    service.start_turn(session.id, turn("third")).await.unwrap();

    let compaction = service.snapshot(session.id).unwrap().compaction.unwrap();
    assert_eq!(compaction.phase, AgentCompactionPhase::PreTurn);
    assert_eq!(model.compaction_requests().len(), 1);
}

#[tokio::test]
async fn a_failed_attempt_is_not_launched_again_until_another_turn_completes() {
    let model = BackgroundModel::local(
        [
            answer(&FIRST_ANSWER, 100),
            tool_call(IN_BAND),
            answer("second answer", IN_BAND),
            answer("third answer", IN_BAND),
        ],
        [
            compaction(Answer::Fail),
            compaction(Answer::Summary("the first two turns")),
        ],
    );
    let service = AgentService::new(Arc::new(model.clone()), list_tools().await.registries());
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    service.start_turn(session.id, turn("first")).await.unwrap();
    // Both steps of the second turn end in the band; only the first
    // launches.
    service
        .start_turn(session.id, turn("second"))
        .await
        .unwrap();
    eventually("the failed compaction to end", || {
        background_state(&service, session.id).is_none()
    })
    .await;
    assert_eq!(model.compaction_requests().len(), 1);

    // The third turn has another completed turn before it.
    service.start_turn(session.id, turn("third")).await.unwrap();
    background_ready(&service, session.id).await;
    assert_eq!(model.compaction_requests().len(), 2);
    assert!(format!("{:?}", model.compaction_requests()[1].input).contains("second"));
}

#[tokio::test]
async fn each_implementation_runs_in_the_background() {
    let payload = ModelOpaqueCompactionPayload::parse("opaque-v1".to_string()).unwrap();
    for (capability, answer_given, compacted) in [
        (
            ModelCompactionCapability::ResponsesCompact,
            Answer::Replacement(vec![
                ModelInputItem::user_text("first"),
                ModelInputItem::Compaction { payload },
            ]),
            "payload:opaque-v1",
        ),
        (
            ModelCompactionCapability::ResponsesCompactV2,
            Answer::Payload("opaque-v2"),
            "payload:opaque-v2",
        ),
    ] {
        let model = BackgroundModel::new(
            capability,
            seed_then([answer("third answer", 200)]),
            [compaction(answer_given)],
        );
        let service = AgentService::new(
            Arc::new(model.clone()),
            FakeTools::new([]).await.registries(),
        );
        let session = service
            .create_session(background_config(capability))
            .unwrap();
        seed(&service, session.id).await;
        background_ready(&service, session.id).await;
        service.start_turn(session.id, turn("third")).await.unwrap();

        assert!(background_completed(&service, session.id));
        assert_eq!(
            layout(&service.test_model_context(session.id).unwrap())[..3],
            ["user:first", compacted, "turn"]
        );
    }
}
