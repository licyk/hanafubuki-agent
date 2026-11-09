use super::*;

#[tokio::test]
async fn responses_reasoning_reaches_typed_events_and_history_but_not_the_next_request() {
    let server = FakeServer::start(vec![
        (
            reasoning_tool_events("Thinking about the Skills.", "skills_list", "{}"),
            Duration::ZERO,
        ),
        (
            reasoning_assistant_events("Reasoning lifecycle observed."),
            Duration::ZERO,
        ),
    ])
    .await;
    let (service, paths) = configured_service("reasoning-lifecycle", &server.base_url);
    let session = service
        .create_session(AgentSessionMode::Default)
        .await
        .unwrap();
    let outcome = service
        .start_turn(session.id, "Think, then answer.".to_string().into())
        .await
        .unwrap();
    assert_eq!(outcome.final_answer, "Reasoning lifecycle observed.");

    let snapshot = service.snapshot(session.id).unwrap();
    let reasoning = snapshot.turn.as_ref().unwrap().reasoning;
    assert_eq!(
        reasoning.phase,
        hanafubuki_agent_kernel::AgentReasoningPhase::Completed
    );
    assert_eq!(reasoning.sampling_step, 2);
    assert!(reasoning.active_since.is_none());
    let events = service.events_since(session.id, None).unwrap().events;
    let started = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::ReasoningStarted { .. }))
        .unwrap();
    let shown = events
        .iter()
        .filter_map(|event| match &event.kind {
            AgentEventKind::ReasoningDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        [
            "Thinking about the Skills.",
            "A summary that arrives whole."
        ]
    );
    let completed = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::ReasoningCompleted { .. }))
        .unwrap();
    let tool = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::ToolStarted { .. }))
        .unwrap();
    assert!(started < completed && completed < tool);

    let page = service
        .read_transcript_page(crate::AgentTranscriptPageRequest {
            session_id: session.id,
            page_size: 50,
            cursor: None,
            direction: crate::AgentHistorySortDirection::Ascending,
            through_revision: None,
        })
        .unwrap();
    let kept = page
        .items
        .iter()
        .filter_map(|item| match &item.content {
            ConversationItem::Reasoning {
                text,
                sampling_step,
                ..
            } => Some((text.as_str(), *sampling_step)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kept,
        [
            ("Thinking about the Skills.", 1),
            ("A summary that arrives whole.", 2)
        ],
        "the history store keeps each reasoning item"
    );
    let second_request = String::from_utf8(server.requests()[1].body.clone()).unwrap();
    assert!(second_request.contains("call-tool"));
    assert!(
        !second_request.contains("Thinking about the Skills."),
        "thinking is never model input"
    );

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
