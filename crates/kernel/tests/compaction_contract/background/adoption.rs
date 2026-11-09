use super::*;

#[tokio::test]
async fn a_result_ready_before_the_next_turn_is_installed_before_it_samples() {
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
    let sink = Arc::new(RecordingSink::default());
    service
        .start_turn_with_sink(session.id, turn("third"), sink.clone())
        .await
        .unwrap();

    assert!(background_completed(&service, session.id));
    assert_eq!(sink.compaction_phases(), [AgentCompactionPhase::Background]);
    assert_eq!(model.compaction_requests().len(), 1);
    assert_eq!(model.requests().len(), 3);
    let context = service.test_model_context(session.id).unwrap();
    assert_eq!(
        layout(&context),
        [
            "user:first",
            "summary:the first turn",
            "turn",
            "user:second",
            "assistant:second answer",
            "turn",
            "user:third",
            "assistant:third answer",
        ]
    );
    // The request the compaction sent held the first turn only.
    let compacted = &model.compaction_requests()[0];
    assert!(format!("{:?}", compacted.input).contains("first"));
    assert!(!format!("{:?}", compacted.input).contains("second"));
}

/// The result is installed between two steps of a turn that started after
/// its snapshot: the turns since keep their items and order after it, and
/// their world-state updates still apply to the state it starts from.
/// Forked after it, the session reads the same context.
#[tokio::test]
async fn a_result_ready_between_steps_is_installed_before_the_work_added_since() {
    let background = Gate::default();
    let step = Gate::default();
    let model = BackgroundModel::local(
        seed_then([
            gated(&step, tool_response_with_usage(IN_BAND)),
            answer("third answer", 300),
            answer("fourth answer", 300),
        ]),
        [held(&background, Answer::Summary("the first turn"))],
    );
    let service = AgentService::new(Arc::new(model.clone()), list_tools().await.registries());
    let session = service
        .create_session(background_config(ModelCompactionCapability::LocalSummary))
        .unwrap();
    service
        .start_turn(session.id, turn_at("first", "chooser"))
        .await
        .unwrap();
    service
        .start_turn(session.id, turn("second"))
        .await
        .unwrap();
    let third = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .start_turn(session.id, turn_at("third", "editor"))
                .await
        }
    });
    eventually("the third turn's first request", || {
        model.requests().len() == 3
    })
    .await;
    background.open();
    background_ready(&service, session.id).await;
    step.open();
    third.await.unwrap().unwrap();

    assert!(background_completed(&service, session.id));
    let context = service.test_model_context(session.id).unwrap();
    assert_eq!(
        layout(&context),
        [
            "world",
            "user:first",
            "summary:the first turn",
            "turn",
            "world",
            "user:second",
            "assistant:second answer",
            "turn",
            "world",
            "user:third",
            "tool_call",
            "tool_output",
            "assistant:third answer",
        ]
    );
    // The world state where the second turn began, which the turns since
    // update in order: the second removed the location, the third set it.
    let world = |index: usize| match &context[index] {
        ConversationItem::WorldStateUpdate { updates } => format!("{updates:?}"),
        other => panic!("not a world-state update: {other:?}"),
    };
    assert!(world(0).contains("chooser"));
    assert!(world(4).contains("Remove"));
    assert!(world(8).contains("editor"));
    // The step after the install sent that context.
    let sent = &model.requests()[3].input;
    let position = |needle: &str| {
        sent.iter()
            .position(|item| format!("{item:?}").contains(needle))
            .unwrap()
    };
    assert!(position("the first turn") < position("second answer"));
    assert!(position("second answer") < position("third"));

    let fourth = turn("fourth");
    service.start_turn(session.id, fourth).await.unwrap();
    let completed = service.snapshot(session.id).unwrap();
    let fork = service
        .fork_session_before_turn(hanafubuki_agent_kernel::AgentSessionForkRequest {
            source_session_id: session.id,
            source_generation: completed.generation,
            source_revision: completed.revision,
            before_turn_id: completed.turn.unwrap().id,
        })
        .unwrap();
    assert_eq!(fork.restored_prompt, "fourth");
    assert_eq!(
        layout(&service.test_model_context(fork.session.id).unwrap()),
        layout(&context)
    );
}

/// The model asked for a new context while the compaction ran: the turn
/// compacted in line, and the result the background then delivers is for a
/// window the session left, which its install refuses.
#[tokio::test]
async fn a_result_for_a_window_the_session_left_is_refused_at_install() {
    let background = Gate::default();
    let model = BackgroundModel::local(
        seed_then([
            new_context_tool_turn(),
            answer("third answer", 200),
            answer("fourth answer", 200),
        ]),
        [
            held(&background, Answer::Summary("the first turn")),
            compaction(Answer::Summary("new context")),
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
    background_sent(&model).await;
    service.start_turn(session.id, turn("third")).await.unwrap();
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .compaction
            .unwrap()
            .phase,
        AgentCompactionPhase::MidTurn
    );
    background.open();
    background_ready(&service, session.id).await;
    let sink = Arc::new(RecordingSink::default());
    service
        .start_turn_with_sink(session.id, turn("fourth"), sink.clone())
        .await
        .unwrap();

    assert!(sink.compaction_phases().is_empty());
    assert_eq!(background_state(&service, session.id), None);
    assert!(
        !format!("{:?}", service.test_model_context(session.id).unwrap())
            .contains("the first turn")
    );
}

fn new_context_tool_turn() -> ScriptedTurnResponse {
    ScriptedTurnResponse {
        gate: None,
        response: new_context_tool_response(),
    }
}
