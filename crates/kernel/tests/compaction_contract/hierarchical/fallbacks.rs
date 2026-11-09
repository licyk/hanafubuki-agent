use hanafubuki_agent_kernel::test_support::COMPRESSION_PANIC_MARKER;

use super::*;

fn context_error() -> ScriptedResponse {
    ScriptedResponse::Events(vec![Err(ModelError::ContextWindowExceeded {
        status: Some(400),
        code: Some("context_length_exceeded".to_string()),
        message: "maximum context length exceeded".to_string(),
        request_id: Some("request-context".to_string()),
    })])
}

/// The model folds the window: no threshold compacts it, however full the
/// provider says it is.
#[tokio::test]
async fn the_threshold_does_not_compact_a_hierarchical_session() {
    let model = ScriptedModel::new([
        response("one", &FIRST_ANSWER, 1_000),
        response("two", &SECOND_ANSWER, 90_000),
        response("three", "Still here.", 90_000),
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

    assert_eq!(model.requests().len(), 3);
    assert_eq!(service.snapshot(session.id).unwrap().compaction, None);
}

/// A provider's context error runs one standard compaction and retries. The
/// compaction reads the projection (the summary, not what it folded); once
/// installed, the folded messages have left the window and the block
/// deactivates.
#[tokio::test]
async fn a_context_error_compacts_the_projection_once_and_retries() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
        context_error(),
        response("compact", "Two exchanges, the first folded.", 50),
        response("four", "Retried.", 1_000),
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
    let outcome = service
        .start_turn(session.id, turn("Continue."))
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "Retried.");
    let requests = model.requests();
    assert_eq!(requests.len(), 7);
    let compaction = &requests[5];
    assert!(compaction.tools.is_empty());
    assert!(!text(compaction).contains("a long first answer"));
    assert!(text(compaction).contains("First exchange"));
    assert!(text(compaction).contains("a long second answer"));
    let snapshot = service.snapshot(session.id).unwrap();
    assert_eq!(
        snapshot.compaction.unwrap().status,
        AgentCompactionStatus::Completed
    );
    let state = state(&service, session.id);
    assert_eq!(state.blocks.len(), 1);
    assert_eq!(state.active_blocks().count(), 0);
}

/// When nothing can be folded or clipped and the context stays over the
/// emergency threshold for three requests, one standard compaction runs
/// instead of telling the user to start over (decision 10).
#[tokio::test]
async fn the_terminal_escape_runs_one_standard_compaction() {
    let model = ScriptedModel::new([
        tool_response_with_usage(19_500),
        tool_response_with_usage(19_500),
        tool_response_with_usage(19_500),
        response("compact", "The instances were listed three times.", 50),
        response("answer", "No instances.", 1_000),
    ]);
    let tools = FakeTools::new((0..3).map(|_| {
        FakeRegisteredExchange::success("list_instances", json!({}), json!({"instances": []}))
    }))
    .await;
    let service = AgentService::new(Arc::new(model.clone()), tools.registries());
    let session = service
        .create_session(AgentSessionConfig {
            context_window_tokens: Some(20_000),
            ..hierarchical_config()
        })
        .unwrap();
    let outcome = service
        .start_turn(session.id, turn("List the instances."))
        .await
        .unwrap();

    assert_eq!(outcome.final_answer, "No instances.");
    let requests = model.requests();
    assert_eq!(requests.len(), 5);
    assert!(requests[3].tools.is_empty());
    let compaction = service.snapshot(session.id).unwrap().compaction.unwrap();
    assert_eq!(
        compaction.reason,
        hanafubuki_agent_kernel::AgentCompactionReason::ContextLimit
    );
    assert_eq!(state(&service, session.id).terminal_streak, 0);
}

/// A panic in the projection costs the request its projection, not the
/// session: the turn sends the full history as `standard` would and ends.
/// A panic in `compress` is the call's error.
#[tokio::test]
async fn a_panic_in_the_algorithm_is_contained() {
    let mut responses = two_exchanges();
    responses.extend([
        compress("fold-1", "m00002–m00002", "First exchange"),
        response("three", "Folded.", 3_000),
        response("four", "Full history.", 3_000),
        compress("fold-2", "m00004–m00004", "Second exchange"),
        response("five", "Not folded.", 3_000),
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

    let mut corrupt = state(&service, session.id);
    corrupt.view_latch_config_digest = Some(COMPRESSION_PANIC_MARKER.to_string());
    service
        .test_set_compression_state(session.id, Some(corrupt.clone()))
        .unwrap();
    let outcome = service
        .start_turn(session.id, turn("Are you there?"))
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Full history.");
    let request = &model.requests()[4];
    assert!(text(request).contains("a long first answer"));
    assert!(!request.instructions.contains("ACP TAGS"));
    assert_eq!(state(&service, session.id), corrupt);

    let corrupt = AgentCompressionState {
        view_latch_config_digest: Some(COMPRESSION_PANIC_MARKER.to_string()),
        ..AgentCompressionState::default()
    };
    service
        .test_set_compression_state(session.id, Some(corrupt.clone()))
        .unwrap();
    service
        .start_turn(session.id, turn("Fold the second exchange."))
        .await
        .unwrap();
    let receipt = tool_output(&service, session.id, "fold-2");
    assert!(receipt.contains("[Compression FAILED"), "{receipt}");
    assert!(receipt.contains("internal error"), "{receipt}");
    assert!(state(&service, session.id).blocks.is_empty());
}
