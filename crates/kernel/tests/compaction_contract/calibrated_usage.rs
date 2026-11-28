use super::*;

/// A session counts its model's own ratio: two bytes to a token counts the
/// same text twice as many tokens as the default of four.
#[tokio::test]
async fn a_session_counts_at_the_ratio_it_is_given() {
    let count = async |ratio: Option<u64>| {
        let model = ScriptedModel::new([response("first", "Recorded.", 40_000)]);
        let service = AgentService::new(Arc::new(model), FakeTools::new([]).await.registries());
        let session = service
            .create_session(AgentSessionConfig {
                context_window_tokens: Some(1_000_000),
                bytes_per_token_millis: ratio,
                ..config()
            })
            .unwrap();
        service
            .start_turn(session.id, "configuration ".repeat(40_000))
            .await
            .unwrap();
        service
            .snapshot(session.id)
            .unwrap()
            .usage
            .unwrap()
            .context_estimated_tokens
            .unwrap()
    };
    let dense = count(Some(2_000)).await;
    let default = count(None).await;
    // Each group rounds up, so twice the tokens is a few short of exact.
    assert!(dense + 64 >= default * 2, "dense {dense} default {default}");
}

#[tokio::test]
async fn background_compaction_does_not_use_the_uncalibrated_history_size() {
    use hanafubuki_agent_kernel::{AgentBackgroundCompactionThreshold, AgentCompactionStrategy};
    let model = ScriptedModel::new([
        response("first", "Recorded.", 525_000),
        response("second", "Continued.", 525_100),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            context_window_tokens: Some(1_000_000),
            compaction_strategy: AgentCompactionStrategy::Background,
            ..config()
        })
        .unwrap();
    for text in ["configuration ".repeat(300_000), "Continue.".into()] {
        let mut input = AgentTurnInput::from(text);
        input.turn = input
            .turn
            .with(Arc::new(AgentBackgroundCompactionThreshold { percent: 70 }));
        service.start_turn(session.id, input).await.unwrap();
    }
    let snapshot = service.snapshot(session.id).unwrap();
    assert!(snapshot.compaction.is_none());
    assert!(snapshot.background_compaction.is_none());
    assert_eq!(model.requests().len(), 2);
}

#[tokio::test]
async fn reported_usage_prevents_early_compaction_of_large_history_after_restart() {
    use hanafubuki_agent_kernel::EphemeralAgentHistoryStore;

    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let model = Arc::new(ScriptedModel::new([response(
        "first",
        "Recorded.",
        525_000,
    )]));
    let service = AgentService::with_history_store(
        model,
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            context_window_tokens: Some(1_000_000),
            auto_compact_threshold_percent: 80,
            ..config()
        })
        .unwrap();
    service
        .start_turn(session.id, " configuration".repeat(400_000))
        .await
        .unwrap();
    let usage = service.snapshot(session.id).unwrap().usage.unwrap();
    assert!(usage.context_estimated_tokens.unwrap() > 1_000_000);
    service.shutdown(std::time::Instant::now() + Duration::from_secs(5));
    drop(service);

    let model = Arc::new(ScriptedModel::new([response(
        "second",
        "Continued.",
        525_100,
    )]));
    let service = AgentService::with_history_store(
        model.clone(),
        FakeTools::new([]).await.registries(),
        store,
    );
    assert_eq!(
        service
            .recover_sessions_with_test_model(model.clone())
            .unwrap()
            .restored,
        1
    );
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .usage
            .unwrap()
            .context_estimated_tokens,
        usage.context_estimated_tokens
    );
    assert_eq!(
        service
            .start_turn(session.id, "Continue.")
            .await
            .unwrap()
            .final_answer,
        "Continued."
    );
    assert!(service.snapshot(session.id).unwrap().compaction.is_none());
    assert_eq!(model.requests().len(), 1);
}

#[tokio::test]
async fn new_input_growth_crosses_the_threshold_even_when_last_report_does_not() {
    let model = ScriptedModel::new([
        response("first", "Recorded.", 790_000),
        response("summary", "Previous context summarized.", 790_100),
        response("second", "Continued.", 15_000),
        response("third", "Still small.", 15_100),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            context_window_tokens: Some(1_000_000),
            ..config()
        })
        .unwrap();
    service
        .start_turn(session.id, "configuration ".repeat(100_000))
        .await
        .unwrap();
    assert_eq!(
        service
            .start_turn(session.id, "next ".repeat(10_000))
            .await
            .unwrap()
            .final_answer,
        "Continued."
    );
    let compaction = service.snapshot(session.id).unwrap().compaction.unwrap();
    assert_eq!(compaction.trigger, AgentCompactionTrigger::Automatic);
    assert_eq!(compaction.status, AgentCompactionStatus::Completed);
    assert_eq!(
        service
            .start_turn(session.id, "Again.")
            .await
            .unwrap()
            .final_answer,
        "Still small."
    );
    assert_eq!(
        service.snapshot(session.id).unwrap().compaction.unwrap().id,
        compaction.id
    );
    assert_eq!(model.requests().len(), 4);
}

#[tokio::test]
async fn a_response_without_usage_keeps_the_previous_calibration() {
    let mut missing = response("missing", "No usage this time.", 0);
    if let ScriptedResponse::Events(events) = &mut missing {
        for event in events {
            if let Ok(ModelEvent::ResponseCompleted { usage, .. }) = event {
                *usage = None;
            }
        }
    }
    let model = ScriptedModel::new([
        response("first", "Recorded.", 525_000),
        missing,
        response("third", "Continued.", 525_200),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            context_window_tokens: Some(1_000_000),
            ..config()
        })
        .unwrap();
    service
        .start_turn(session.id, "configuration ".repeat(300_000))
        .await
        .unwrap();
    let before = service.snapshot(session.id).unwrap().usage.unwrap();
    service.start_turn(session.id, "Continue.").await.unwrap();
    assert_eq!(service.snapshot(session.id).unwrap().usage, Some(before));
    service.start_turn(session.id, "Again.").await.unwrap();
    assert!(service.snapshot(session.id).unwrap().compaction.is_none());
    assert_eq!(model.requests().len(), 3);
}

#[tokio::test]
async fn added_images_count_as_growth_without_counting_their_encoded_bytes() {
    use hanafubuki_agent_kernel::{AgentImageDetail, PreparedAgentImage};

    let model = ScriptedModel::new([
        response("first", "Recorded.", 799_900),
        response("summary", "Previous context summarized.", 799_950),
        response("image", "Image received.", 500),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service
        .create_session(AgentSessionConfig {
            context_window_tokens: Some(1_000_000),
            ..config()
        })
        .unwrap();
    service
        .start_turn(session.id, "configuration ".repeat(300_000))
        .await
        .unwrap();
    let image = service
        .stage_image_attachment(
            session.id,
            PreparedAgentImage {
                bytes: vec![1, 2, 3],
                mime_type: "image/png".into(),
                width: 1,
                height: 1,
                display_name: "test.png".into(),
                detail: AgentImageDetail::High,
            },
        )
        .unwrap();
    let mut input = AgentTurnInput::from("Describe it.");
    input.images.push(image);
    assert_eq!(
        service
            .start_turn(session.id, input)
            .await
            .unwrap()
            .final_answer,
        "Image received."
    );
    assert_eq!(
        service
            .snapshot(session.id)
            .unwrap()
            .compaction
            .unwrap()
            .status,
        AgentCompactionStatus::Completed
    );
    assert!(model.requests().last().unwrap().input.iter().any(|item| matches!(item,
        ModelInputItem::UserMessage { content } if content.iter().any(|part| matches!(part, hanafubuki_agent_model_api::ModelUserContent::InputImage { .. }))
    )));
}
