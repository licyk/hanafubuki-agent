use super::*;

/// The requests of a `hierarchical` session carry the static prompt, the
/// five tools and the ref tags; a `compress` call folds a range with its
/// receipt in the same commit, and from then on the requests leave the
/// folded messages out while the stored history keeps them.
#[tokio::test]
async fn a_fold_projects_the_requests_and_never_rewrites_the_history() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
        response("four", "Still here.", 3_000),
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
    service
        .start_turn(session.id, turn("Are you there?"))
        .await
        .unwrap();

    let requests = model.requests();
    assert_eq!(requests.len(), 5);
    assert!(requests[0]
        .instructions
        .starts_with("fixture instructions\n\n"));
    assert!(requests[0].instructions.contains("ACP TAGS"));
    for tool in COMPRESSION_TOOLS {
        assert!(
            tool_names(&requests[0]).iter().any(|name| name == tool),
            "{tool}"
        );
    }
    assert!(text(&requests[1]).contains(">m00001</acp>\\nfirst"));
    assert!(text(&requests[1]).contains(">m00002</acp>\\na long first answer"));

    let receipt = tool_output(&service, session.id, "fold-1");
    assert!(
        receipt.contains("[Compressed m00002–m00002 → 1 block(s)"),
        "{receipt}"
    );
    let state = state(&service, session.id);
    let active = state.active_blocks().collect::<Vec<_>>();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].tier, 1);
    assert_eq!(active[0].compress_call_id.as_deref(), Some("fold-1"));

    for request in &requests[3..] {
        let input = text(request);
        assert!(!input.contains("a long first answer"));
        assert!(input.contains("a long second answer"));
        assert!(input.contains("First exchange"));
    }
    assert!(service
        .test_model_context(session.id)
        .unwrap()
        .iter()
        .any(|item| matches!(
            item,
            ConversationItem::AssistantMessage { text, .. } if *text == *FIRST_ANSWER
        )));
}

/// Two tier-1 blocks distil into one tier-2 block named by their block
/// refs; `decompress` brings a block's messages back as its output, and
/// the read-only tools answer from the state.
#[tokio::test]
async fn blocks_distil_to_tier_two_and_the_tools_read_them() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        compress("fold-2", "m00004–m00004", "Second exchange"),
        call(
            "compress",
            "distil",
            json!({ "content": "b1–b2 Both exchanges\nThe user asked twice and the assistant answered at length both times; nothing is open." }),
        ),
        call("decompress", "restore", json!({ "blockId": "b3", "full": true })),
        call("search_context", "search", json!({ "query": "exchanges" })),
        call("acp_status", "status", json!({})),
        call("acp_cache", "cache", json!({})),
        response("three", "Distilled.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Fold and distil."))
        .await
        .unwrap();

    let distilled = tool_output(&service, session.id, "distil");
    assert!(
        distilled.contains("[Compressed b1–b2 → 1 block(s)"),
        "{distilled}"
    );
    let state = state(&service, session.id);
    let active = state.active_blocks().collect::<Vec<_>>();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].block_id, "b3");
    assert_eq!(active[0].tier, 2);
    assert!(active[0].restored_inline);

    let restored = tool_output(&service, session.id, "restore");
    assert!(restored.contains("[Block b3 content — "), "{restored}");
    assert!(restored.contains("a long first answer"));
    assert!(restored.contains("Re-fold: call compress"));
    let found = tool_output(&service, session.id, "search");
    assert!(found.contains("b3 (T2) \\\"Both exchanges\\\""), "{found}");
    assert!(!tool_output(&service, session.id, "status").is_empty());
    assert!(tool_output(&service, session.id, "cache").contains("ACP CACHE REPORT"));

    assert!(text(model.requests().last().unwrap()).contains("Both exchanges"));
}

/// A call the algorithm cannot apply folds nothing: its receipt says why in
/// billion-context's words, and the state is the one before it.
#[tokio::test]
async fn a_refused_compress_call_leaves_the_state_unchanged() {
    let mut responses = two_exchanges();
    responses.extend([
        call("compress", "empty", json!({})),
        compress("unknown", "m00090–m00095", "Nothing there"),
        response("three", "Nothing folded.", 3_000),
    ]);
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    let before = state(&service, session.id);
    service
        .start_turn(session.id, turn("Fold nothing."))
        .await
        .unwrap();

    assert!(tool_output(&service, session.id, "empty").contains("[Compression FAILED"));
    assert!(tool_output(&service, session.id, "unknown").contains("[Compression FAILED"));
    let after = state(&service, session.id);
    assert!(after.blocks.is_empty());
    assert_eq!(after.next_block_id, before.next_block_id);
}

/// Over the pressure band the request ends with the nudge, a user input
/// that is never committed to the history.
#[tokio::test]
async fn the_nudge_is_the_last_input_and_is_never_committed() {
    // Each answer is worth folding by itself: about 7 500 tokens.
    let model = ScriptedModel::new([
        response("one", &"a long first answer ".repeat(1_500), 1_000),
        response("two", &"a long second answer ".repeat(1_500), 80_000),
        response("three", "Noted.", 80_000),
    ]);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    service
        .start_turn(session.id, turn("Go on."))
        .await
        .unwrap();

    let request = &model.requests()[2];
    let Some(ModelInputItem::UserMessage { content }) = request.input.last() else {
        panic!(
            "the request ends with the nudge: {:?}",
            request.input.last()
        );
    };
    let nudge = serde_json::to_string(content).unwrap();
    assert!(nudge.contains("compress"), "{nudge}");
    assert!(!service
        .test_model_context(session.id)
        .unwrap()
        .iter()
        .any(|item| serde_json::to_string(item).unwrap().contains(&nudge[1..40])));
}
