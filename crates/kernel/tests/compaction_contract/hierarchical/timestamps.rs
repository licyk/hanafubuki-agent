//! Message timestamps (`messageTimestamps`): each message is stamped with
//! the clock when a request first shows it, its tag shows that time on
//! every later request and after a restart, a fold's receipt shows the span
//! it covered, and with the setting off nothing is stamped or shown.

use hanafubuki_agent_compression::LocalTime;

use super::*;

/// 2026-09-28 15:59:09 at +08:00.
const SEPTEMBER: LocalTime = LocalTime {
    at: 1_790_582_349_000,
    utc_offset_seconds: 8 * 3600,
};

fn minutes_later(minutes: u64) -> LocalTime {
    LocalTime {
        at: SEPTEMBER.at + minutes * 60_000,
        ..SEPTEMBER
    }
}

/// The tag `request` sent for `ref`, as JSON-escaped in its input.
fn tag(request: &ModelRequest, r#ref: &str) -> String {
    let input = text(request);
    let end = input
        .find(&format!(">{ref}</acp>"))
        .unwrap_or_else(|| panic!("{ref} is tagged: {input}"));
    let start = input[..end].rfind("<acp ").unwrap();
    input[start..end + r#ref.len() + 7].to_owned()
}

fn timestamps_off(text: &str) -> AgentTurnInput {
    let mut input = AgentTurnInput::from(text);
    input.turn = TurnExtensions::default().with(Arc::new(AgentHierarchicalCompression {
        message_timestamps: false,
        ..tuning()
    }));
    input
}

/// A stamp is made once: every later request sends the same tag, and so
/// does a restarted kernel, which reads the stamps back from the chain.
#[tokio::test]
async fn a_tag_s_time_is_the_same_on_every_request_and_after_a_restart() {
    let model = ScriptedModel::new(two_exchanges());
    let store = Arc::new(EphemeralAgentHistoryStore::default());
    let service = AgentService::with_history_store(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    seed(&service, session.id).await;
    let requests = model.requests();
    let first = tag(&requests[0], "m00001");
    assert!(first.contains(" time=\\\""), "{first}");
    assert_eq!(tag(&requests[1], "m00001"), first);
    let answer = tag(&requests[1], "m00002");
    assert!(answer.contains(" time=\\\""), "{answer}");
    let before = state(&service, session.id);
    assert_eq!(
        before.ts_stamps.len(),
        3,
        "the messages the requests showed"
    );

    let restarted_model = ScriptedModel::new([response("three", "Back.", 3_000)]);
    let restarted = AgentService::without_default_model_with_history_store(
        FakeTools::new([]).await.registries(),
        store.clone(),
    );
    restarted
        .recover_sessions_with_test_model(Arc::new(restarted_model.clone()))
        .unwrap();
    assert_eq!(state(&restarted, session.id).ts_stamps, before.ts_stamps);
    restarted
        .start_turn(session.id, turn("Are you there?"))
        .await
        .unwrap();
    let request = &restarted_model.requests()[0];
    assert_eq!(tag(request, "m00001"), first);
    assert_eq!(tag(request, "m00002"), answer);
}

/// A message's time is the clock of the first request that shows it (an
/// answer's is the next turn's), with the instructions explaining the
/// attribute; a fold's receipt shows the earliest and latest of the times
/// it covered.
#[tokio::test]
async fn a_tag_shows_when_its_message_was_first_seen_and_a_receipt_the_span() {
    let mut responses = two_exchanges();
    responses.push(compress("fold-1", "m00002–m00004", "Both exchanges"));
    responses.push(response("three", "Folded.", 3_000));
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    for (minutes, text) in [(0, "first"), (1, "second"), (2, "Fold both.")] {
        service
            .test_fix_message_clock(session.id, Some(minutes_later(minutes)))
            .unwrap();
        service.start_turn(session.id, turn(text)).await.unwrap();
    }

    let requests = model.requests();
    assert!(requests[0].instructions.contains(
        "<acp tokens=\"2.1K\" type=\"tool:bash\" time=\"2026-10-30 09:02:11\">m00175</acp>"
    ));
    assert!(tag(&requests[0], "m00001").contains(" time=\\\"2026-09-28 15:59:09\\\">"));
    assert!(tag(&requests[1], "m00002").contains(" time=\\\"2026-09-28 16:00:09\\\">"));
    assert!(tag(&requests[2], "m00004").contains(" time=\\\"2026-09-28 16:01:09\\\">"));
    let receipt = tool_output(&service, session.id, "fold-1");
    assert!(
        receipt.contains(" full summary 84ch [2026-09-28 16:00:09 – 2026-09-28 16:01:09] · head "),
        "{receipt}"
    );
}

/// With `messageTimestamps = false` the requests are as they were before
/// timestamps: no stamp, no time attribute, the plain tag section and no
/// span in a receipt.
#[tokio::test]
async fn with_timestamps_off_nothing_is_stamped_or_shown() {
    let mut responses = two_exchanges();
    responses.push(compress("fold-1", "m00002–m00002", "First exchange"));
    responses.push(response("three", "Folded.", 3_000));
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    for text in ["first", "second", "Fold the first exchange."] {
        service
            .start_turn(session.id, timestamps_off(text))
            .await
            .unwrap();
    }

    let requests = model.requests();
    for request in &requests {
        assert!(!text(request).contains(" time="));
        assert!(!request.instructions.contains("time=\""));
        assert!(request
            .instructions
            .contains("<acp tokens=\"2.1K\" type=\"tool:bash\">m00175</acp>"));
    }
    assert_eq!(
        tag(&requests[1], "m00001"),
        "<acp tokens=\\\"2\\\" type=\\\"text\\\">m00001</acp>"
    );
    assert!(state(&service, session.id).ts_stamps.is_empty());
    let receipt = tool_output(&service, session.id, "fold-1");
    assert!(receipt.contains("ch · head "), "{receipt}");
    assert!(!receipt.contains(" – 2026"), "{receipt}");
}

/// A fold over a message shown before timestamps were on shows no span:
/// one unknown time never yields an invented or narrowed one.
#[tokio::test]
async fn a_fold_over_an_untimed_message_shows_no_span() {
    let mut responses = two_exchanges();
    responses.push(compress("fold-1", "m00003–m00004", "Second exchange"));
    responses.push(response("three", "Folded.", 3_000));
    let model = ScriptedModel::new(responses);
    let service = AgentService::new(
        Arc::new(model.clone()),
        FakeTools::new([]).await.registries(),
    );
    let session = service.create_session(hierarchical_config()).unwrap();
    for text in ["first", "second"] {
        service
            .start_turn(session.id, timestamps_off(text))
            .await
            .unwrap();
    }
    service
        .test_fix_message_clock(session.id, Some(SEPTEMBER))
        .unwrap();
    service
        .start_turn(session.id, turn("Fold the second exchange."))
        .await
        .unwrap();

    let requests = model.requests();
    assert!(!tag(&requests[2], "m00003").contains(" time="));
    assert!(tag(&requests[2], "m00004").contains(" time=\\\"2026-09-28 15:59:09\\\">"));
    let receipt = tool_output(&service, session.id, "fold-1");
    assert!(receipt.contains("[Compressed m00003–m00004"), "{receipt}");
    assert!(receipt.contains("ch · head "), "{receipt}");
}
