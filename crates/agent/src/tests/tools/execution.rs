use super::*;

#[tokio::test]
async fn responses_reasoning_lifecycle_reaches_typed_agent_state_without_content() {
    let server = FakeServer::start(vec![(
        reasoning_assistant_events("Reasoning lifecycle observed."),
        Duration::ZERO,
    )])
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
    assert_eq!(reasoning.sampling_step, 1);
    assert!(reasoning.active_since.is_none());
    let events = service.events_since(session.id, None).unwrap().events;
    let thinking = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                AgentEventKind::ReasoningUpdated { reasoning }
                    if reasoning.phase == hanafubuki_agent_kernel::AgentReasoningPhase::Thinking
            )
        })
        .unwrap();
    let completed = events
        .iter()
        .position(|event| {
            matches!(
                event.kind,
                AgentEventKind::ReasoningUpdated { reasoning }
                    if reasoning.phase == hanafubuki_agent_kernel::AgentReasoningPhase::Completed
            )
        })
        .unwrap();
    let assistant = events
        .iter()
        .position(|event| matches!(event.kind, AgentEventKind::AssistantMessageStarted { .. }))
        .unwrap();
    assert!(thinking < completed && completed < assistant);
    let public_json = serde_json::to_string(&(snapshot, events)).unwrap();
    assert!(public_json.contains("reasoning_updated"));
    assert!(public_json.contains("activeSince"));
    assert!(!public_json.contains("private fixture reasoning"));

    assert!(service
        .shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE)
        .is_empty());
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}
