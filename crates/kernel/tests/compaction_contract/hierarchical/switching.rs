use super::*;

async fn set_strategy(
    service: &AgentService,
    session: AgentSessionId,
    strategy: AgentCompactionStrategy,
) -> bool {
    let service = service.clone();
    tokio::task::spawn_blocking(move || service.set_session_compaction_strategy(session, strategy))
        .await
        .unwrap()
        .unwrap()
}

/// A session that leaves `hierarchical` keeps projecting its blocks, so the
/// next request does not carry what they folded; only the prompt, the tools
/// and the nudges stop. Its next standard compaction ends the blocks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_away_keeps_projecting_until_the_next_compaction() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
        response("four", "Standard now.", 3_000),
        response("compact", "Two exchanges.", 50),
        response("five", "Compacted.", 1_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold the first exchange."))
        .await
        .unwrap();

    assert!(set_strategy(&service, session.id, AgentCompactionStrategy::Standard).await);
    service
        .start_turn(session.id, turn("Go on."))
        .await
        .unwrap();
    let carried = &model.requests()[4];
    assert!(!text(carried).contains("a long first answer"));
    assert!(text(carried).contains("First exchange"));
    assert!(text(carried).contains("</acp>"));
    assert!(!carried.instructions.contains("ACP TAGS"));
    assert!(tool_names(carried)
        .iter()
        .all(|name| !COMPRESSION_TOOLS.contains(&name.as_str())));
    assert!(matches!(
        carried.input.last(),
        Some(ModelInputItem::AssistantMessage { .. }) | Some(ModelInputItem::UserMessage { .. })
    ));

    let snapshot = service.snapshot(session.id).unwrap();
    service
        .compact_session(session.id, snapshot.generation, snapshot.revision)
        .await
        .unwrap();
    assert_eq!(service.test_compression_state(session.id).unwrap(), None);
    service
        .start_turn(session.id, turn("And now?"))
        .await
        .unwrap();
    let after = model.requests().last().unwrap().clone();
    assert!(!text(&after).contains("</acp>"));
}

/// A session that takes up `hierarchical` starts an empty state and mints
/// its refs over the window as it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switching_to_hierarchical_mints_refs_over_the_window() {
    let model = ScriptedModel::new([
        response("one", &FIRST_ANSWER, 1_000),
        response("two", "Tagged.", 1_000),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(config()).unwrap();
    service.start_turn(session.id, "first").await.unwrap();
    assert_eq!(service.test_compression_state(session.id).unwrap(), None);
    assert!(!model.requests()[0].instructions.contains("ACP TAGS"));

    assert!(set_strategy(&service, session.id, AgentCompactionStrategy::Hierarchical).await);
    assert_eq!(
        state(&service, session.id),
        AgentCompressionState::default()
    );
    service
        .start_turn(session.id, turn("second"))
        .await
        .unwrap();
    let request = &model.requests()[1];
    assert!(request.instructions.contains("ACP TAGS"));
    assert!(text(request).contains(">m00001</acp>\\nfirst"));
    assert!(text(request).contains(">m00002</acp>\\na long first answer"));
}

/// The state is persisted with the runtime checkpoint: a restarted kernel
/// projects the session as before.
#[tokio::test]
async fn the_state_survives_a_restart() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold the first exchange."))
        .await
        .unwrap();
    let before = state(&service, session.id);
    assert_eq!(before.active_blocks().count(), 1);

    let restarted_model = ScriptedModel::new([response("four", "Back.", 3_000)]);
    let restarted = AgentService::without_default_model_with_history_store(
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    assert_eq!(
        restarted
            .recover_sessions_with_test_model(Arc::new(restarted_model.clone()))
            .unwrap()
            .restored,
        1
    );
    assert_eq!(state(&restarted, session.id), before);
    restarted
        .start_turn(session.id, turn("Are you there?"))
        .await
        .unwrap();
    let request = &restarted_model.requests()[0];
    assert!(!text(request).contains("a long first answer"));
    assert!(text(request).contains("First exchange"));
}
